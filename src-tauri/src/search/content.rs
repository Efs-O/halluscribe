// HalluScribe - session content search: shared .md body read + full-text matcher.

use super::tokenize::{parse_query, ParsedQuery};
use crate::archive::{read_sessions, IndexEntry};
use std::{fs, path::Path};

/// Read a session's `.md` body **once**, lowercased, and test each of
/// `needles` against it in a single pass. Returns a vec parallel to
/// `needles` (`true` = substring found). An absent/unreadable body counts as
/// "not found" for every needle rather than erroring - this is the single
/// disk-read point shared by every keyword search path (chat tool, session
/// list, briefing), so the file-read logic lives in exactly one place.
pub(crate) fn body_find(archive_dir: &Path, entry: &IndexEntry, needles: &[&str]) -> Vec<bool> {
    let md_path = archive_dir.join(&entry.archive_path);
    match fs::read_to_string(&md_path) {
        Ok(markdown) => {
            let lower = markdown.to_lowercase();
            needles
                .iter()
                .map(|needle| lower.contains(needle))
                .collect()
        }
        Err(_) => vec![false; needles.len()],
    }
}

/// Single-needle convenience wrapper over `body_find`, kept for callers that
/// only ever test one term (briefing keyword filter, quoted-phrase mode).
/// `query` must already be lowercased.
pub(crate) fn body_contains(archive_dir: &Path, entry: &IndexEntry, query: &str) -> bool {
    body_find(archive_dir, entry, &[query])
        .into_iter()
        .next()
        .unwrap_or(false)
}

/// Does `needle` (already lowercased) appear in this entry's session-list
/// metadata fields (title, project, error_tags, topic_tags)? No disk read.
fn metadata_contains(entry: &IndexEntry, needle: &str) -> bool {
    entry.title.to_lowercase().contains(needle)
        || entry.project.to_lowercase().contains(needle)
        || entry
            .error_tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(needle))
        || entry
            .topic_tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(needle))
}

/// Full-text session-list matcher: title, project, error_tags, topic_tags,
/// then body. `query` must already be trimmed/lowercased by the caller.
///
/// Quoted phrases reproduce the old exact-substring behaviour verbatim.
/// Unquoted multi-word queries are AND-of-tokens: every token is tested
/// against in-memory metadata first, and the `.md` body is read at most
/// once - only for the tokens still unmatched after that pass.
pub(super) fn matches_fulltext(archive_dir: &Path, entry: &IndexEntry, query: &str) -> bool {
    match parse_query(query) {
        ParsedQuery::Phrase(phrase) => {
            metadata_contains(entry, &phrase) || body_contains(archive_dir, entry, &phrase)
        }
        ParsedQuery::Tokens(tokens) => {
            let unmatched: Vec<&str> = tokens
                .iter()
                .map(String::as_str)
                .filter(|token| !metadata_contains(entry, token))
                .collect();
            if unmatched.is_empty() {
                return true;
            }
            body_find(archive_dir, entry, &unmatched)
                .into_iter()
                .all(|hit| hit)
        }
    }
}

pub(super) fn read_session(archive_dir: &Path, session_id: &str) -> Result<String, String> {
    let sessions = read_sessions(archive_dir);
    let entry = sessions
        .iter()
        .find(|entry| entry.id == session_id)
        .ok_or_else(|| format!("session '{session_id}' not found in index"))?;
    let path = archive_dir.join(&entry.archive_path);
    fs::read_to_string(&path).map_err(|error| format!("failed to read session file: {error}"))
}
