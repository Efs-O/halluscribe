// HalluScribe - raw transcript matching and archive search orchestration.

use super::fold::fold_for_search;
use crate::archive::{load_captured, raw_rel_path, read_raw_at, read_sessions, IndexEntry};
use std::collections::HashSet;
use std::fmt;
use std::path::Path;

const EXCERPT_CHARS: usize = 200;
const MAX_EXCERPTS_PER_SESSION: usize = 20;
pub(crate) const MAX_SESSION_GROUPS: usize = 200;

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct RawExcerpt {
    pub line_no: usize,
    pub excerpt: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RawScanOutcome {
    pub total_hits: usize,
    pub excerpts: Vec<RawExcerpt>,
    pub excerpts_truncated: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct RawSessionMatches {
    pub session_id: String,
    pub total_hits: usize,
    pub excerpts: Vec<RawExcerpt>,
    pub excerpts_truncated: bool,
    /// True for sessions present in the archive index (the normal, summarised
    /// case) — `read_session` returns their distilled summary. False for
    /// sessions the startup capture pass (`archive::capture`) preserved a raw
    /// copy of that no sweep has summarised yet: these have no summary/title
    /// to read, only the raw transcript, so `title`/`date` below stand in for
    /// the metadata `read_session` would otherwise supply.
    pub summarised: bool,
    /// Display title for an unsummarised session, derived from the source
    /// filename. Empty for summarised sessions (use `read_session` instead).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Display date (`YYYY-MM-DD`, from the source file's mtime) for an
    /// unsummarised session. Empty for summarised sessions.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub date: String,
}

#[derive(Debug, serde::Serialize)]
pub struct RawSearchResult {
    pub sessions: Vec<RawSessionMatches>,
    pub sessions_scanned: usize,
    pub sessions_without_raw: usize,
    pub sessions_failed: Vec<String>,
    pub total_hits: usize,
    pub results_truncated: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RawSearchError {
    EmptyQuery,
    Archive(String),
}

impl fmt::Display for RawSearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyQuery => write!(f, "raw transcript search query cannot be empty"),
            Self::Archive(message) => write!(f, "failed to read archive index: {message}"),
        }
    }
}

#[derive(serde::Deserialize)]
struct ArchiveIndexValidation {
    #[serde(rename = "sessions")]
    _sessions: Vec<IndexEntry>,
}

pub fn search_raw(
    archive_dir: &Path,
    needle: &str,
    allowed_ids: Option<&HashSet<String>>,
) -> Result<RawSearchResult, RawSearchError> {
    if needle.trim().is_empty() {
        return Err(RawSearchError::EmptyQuery);
    }
    validate_index(archive_dir)?;

    let mut entries = read_sessions(archive_dir);
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.date.clone()));
    // Captured-only sessions (below) are only ever added when the caller has
    // no allowed_ids scope, so this set is only needed for that path — but
    // it's cheap to build either way and keeps the two loops independent.
    let indexed_ids: HashSet<String> = entries.iter().map(|entry| entry.id.clone()).collect();
    let mut result = RawSearchResult {
        sessions: Vec::new(),
        sessions_scanned: 0,
        sessions_without_raw: 0,
        sessions_failed: Vec::new(),
        total_hits: 0,
        results_truncated: false,
    };

    for entry in entries.into_iter().filter(|entry| {
        allowed_ids
            .map(|ids| ids.contains(&entry.id))
            .unwrap_or(true)
    }) {
        if entry.raw_path.is_empty() {
            result.sessions_without_raw += 1;
            continue;
        }
        let text = match read_raw_at(archive_dir, &entry.raw_path) {
            Ok(text) => text,
            Err(_) => {
                result.sessions_failed.push(entry.id);
                continue;
            }
        };

        result.sessions_scanned += 1;
        let outcome = scan_text(&text, needle, MAX_EXCERPTS_PER_SESSION);
        if outcome.total_hits == 0 {
            continue;
        }

        result.total_hits += outcome.total_hits;
        if result.sessions.len() < MAX_SESSION_GROUPS {
            result.sessions.push(RawSessionMatches {
                session_id: entry.id,
                total_hits: outcome.total_hits,
                excerpts: outcome.excerpts,
                excerpts_truncated: outcome.excerpts_truncated,
                summarised: true,
                title: String::new(),
                date: String::new(),
            });
        } else {
            result.results_truncated = true;
        }
    }

    // Sessions the startup capture pass preserved a raw copy of but that no
    // sweep has summarised yet (`archive::capture`) are invisible to the loop
    // above — they have no index entry at all. Surface them too, flagged
    // `summarised: false`, so L1's rescue copies are actually reachable
    // before a sweep gets to them (or forever, if the source was pruned
    // first). Restricted to the unscoped (allowed_ids: None) case only:
    // an allowed_ids set is always built from index session ids by its
    // callers (chat/briefing scope selection), so a captured-only session
    // was never a candidate for that scope in the first place — including
    // it anyway would silently widen a scope the caller deliberately
    // narrowed.
    if allowed_ids.is_none() {
        let mut captured: Vec<_> = load_captured(archive_dir)
            .into_iter()
            .filter(|(id, _)| !indexed_ids.contains(id))
            .collect();
        captured.sort_by_key(|(_, record)| std::cmp::Reverse(record.mtime_secs));

        for (id, record) in captured {
            let text = match read_raw_at(archive_dir, &raw_rel_path(&id)) {
                Ok(text) => text,
                Err(_) => {
                    result.sessions_failed.push(id);
                    continue;
                }
            };

            result.sessions_scanned += 1;
            let outcome = scan_text(&text, needle, MAX_EXCERPTS_PER_SESSION);
            if outcome.total_hits == 0 {
                continue;
            }

            result.total_hits += outcome.total_hits;
            if result.sessions.len() < MAX_SESSION_GROUPS {
                result.sessions.push(RawSessionMatches {
                    session_id: id,
                    total_hits: outcome.total_hits,
                    excerpts: outcome.excerpts,
                    excerpts_truncated: outcome.excerpts_truncated,
                    summarised: false,
                    title: display_title(&record.source_path),
                    date: display_date(record.mtime_secs),
                });
            } else {
                result.results_truncated = true;
            }
        }
    }

    Ok(result)
}

/// Display title for a captured-only (unsummarised) session: the source
/// file's name, since there is no distilled title to show instead.
fn display_title(source_path: &str) -> String {
    Path::new(source_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown session")
        .to_string()
}

/// Display date for a captured-only (unsummarised) session, derived from the
/// source file's mtime at capture time. Empty (never fabricated) if the
/// timestamp doesn't convert to a valid date.
fn display_date(mtime_secs: i64) -> String {
    chrono::DateTime::from_timestamp(mtime_secs, 0)
        .map(|dt: chrono::DateTime<chrono::Utc>| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn validate_index(archive_dir: &Path) -> Result<(), RawSearchError> {
    let index_path = archive_dir.join("index.json");
    let raw = match std::fs::read_to_string(index_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(RawSearchError::Archive(error.to_string())),
    };
    serde_json::from_str::<ArchiveIndexValidation>(&raw)
        .map(|_| ())
        .map_err(|error| RawSearchError::Archive(error.to_string()))
}

/// Return the representation stored inside a JSON string value, without the
/// serializer's surrounding quotes.
pub(crate) fn json_escaped(needle: &str) -> String {
    let serialized = serde_json::to_string(needle)
        .expect("serializing a Rust string as a JSON string is infallible");
    serialized[1..serialized.len() - 1].to_owned()
}

/// Scan one decompressed transcript while retaining at most one excerpt per
/// matching line. Matching offsets belong to the lowercased line; excerpts
/// deliberately use that same copy because Unicode lowercasing can change
/// byte lengths relative to the source.
pub(crate) fn scan_text(text: &str, needle: &str, max_excerpts: usize) -> RawScanOutcome {
    if needle.trim().is_empty() {
        return RawScanOutcome {
            total_hits: 0,
            excerpts: Vec::new(),
            excerpts_truncated: false,
        };
    }

    // Fold (case + Greek accents, D9) rather than just lowercase, on BOTH the
    // needle and the haystack, so `καλημερα` finds `Καλημέρα`. The fold
    // preserves char count (see fold.rs), so the offsets below stay valid.
    let literal = fold_for_search(needle);
    let escaped = fold_for_search(&json_escaped(needle));
    let second_needle = (escaped != literal).then_some(escaped.as_str());
    let literal_chars = literal.chars().count();
    let escaped_chars = escaped.chars().count();
    let mut outcome = RawScanOutcome {
        total_hits: 0,
        excerpts: Vec::new(),
        excerpts_truncated: false,
    };

    for (line_index, line) in text.lines().enumerate() {
        let lowered = fold_for_search(line);
        let mut line_hits = 0;
        let mut first_hit = None;

        // Starting only at char boundaries makes overlapping matching safe for
        // UTF-8 while the OR merges dual-needle hits at an identical position.
        for (byte_index, _) in lowered.char_indices() {
            let tail = &lowered[byte_index..];
            let literal_match = tail.starts_with(&literal);
            let escaped_match = second_needle.is_some_and(|candidate| tail.starts_with(candidate));
            if literal_match || escaped_match {
                line_hits += 1;
                let matched_chars = if literal_match {
                    literal_chars
                } else {
                    escaped_chars
                };
                first_hit.get_or_insert((byte_index, matched_chars));
            }
        }

        outcome.total_hits += line_hits;
        let Some((hit_byte, hit_chars)) = first_hit else {
            continue;
        };

        if outcome.excerpts.len() < max_excerpts {
            outcome.excerpts.push(RawExcerpt {
                line_no: line_index + 1,
                excerpt: excerpt_around(line, &lowered, hit_byte, hit_chars),
            });
        } else {
            outcome.excerpts_truncated = true;
        }
    }

    outcome
}

/// Cut the excerpt from the ORIGINAL line so the user sees the text as it was
/// written (case and accents intact); the hit position comes from the folded
/// line. The fold keeps char count for real text, so char offsets carry over;
/// in the rare case it does not (e.g. `İ` lowercases to two chars), the folded
/// line is used instead so the excerpt still frames the hit.
fn excerpt_around(original: &str, folded: &str, hit_byte: usize, hit_chars: usize) -> String {
    let hit_start = folded[..hit_byte].chars().count();
    let total_chars = original.chars().count();
    let line = if total_chars == folded.chars().count() {
        original
    } else {
        folded
    };
    let total_chars = line.chars().count();
    if total_chars <= EXCERPT_CHARS {
        return line.to_owned();
    }

    let hit_center = hit_start + hit_chars / 2;
    let mut start = hit_center.saturating_sub(EXCERPT_CHARS / 2);
    start = start.min(total_chars - EXCERPT_CHARS);
    line.chars().skip(start).take(EXCERPT_CHARS).collect()
}
