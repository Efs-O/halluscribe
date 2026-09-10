// HalluScribe - session content search: shared .md body read + full-text matcher.

use super::body_cache;
use super::tokenize::{parse_query, ParsedQuery};
use crate::archive::{read_sessions, IndexEntry};
use std::{fs, path::Path};

/// Test each of `needles` against a session's lowercased `.md` body in a
/// single pass. Returns a vec parallel to `needles` (`true` = substring
/// found). An absent/unreadable body counts as "not found" for every needle
/// rather than erroring - this is the single body-read point shared by every
/// keyword search path (chat tool, session list, briefing), so the read logic
/// lives in exactly one place. The body comes from `body_cache`, which holds
/// the whole corpus in memory; see that module for how staleness is caught.
pub(crate) fn body_find(archive_dir: &Path, entry: &IndexEntry, needles: &[&str]) -> Vec<bool> {
    let stamp = body_cache::stamp(archive_dir);
    body_find_with_stamp(archive_dir, entry, needles, &stamp)
}

pub(crate) fn body_find_with_stamp(
    archive_dir: &Path,
    entry: &IndexEntry,
    needles: &[&str],
    stamp: &crate::archive::IndexStamp,
) -> Vec<bool> {
    body_cache::with_body_for_stamp(archive_dir, &entry.archive_path, stamp, |lower| {
        needles
            .iter()
            .map(|needle| lower.contains(needle))
            .collect()
    })
    .unwrap_or_else(|| vec![false; needles.len()])
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

pub(crate) fn body_contains_with_stamp(
    archive_dir: &Path,
    entry: &IndexEntry,
    query: &str,
    stamp: &crate::archive::IndexStamp,
) -> bool {
    body_find_with_stamp(archive_dir, entry, &[query], stamp)
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
pub(super) fn matches_fulltext_with_stamp(
    archive_dir: &Path,
    entry: &IndexEntry,
    query: &str,
    stamp: &crate::archive::IndexStamp,
) -> bool {
    match parse_query(query) {
        ParsedQuery::Phrase(phrase) => {
            metadata_contains(entry, &phrase)
                || body_contains_with_stamp(archive_dir, entry, &phrase, stamp)
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
            body_find_with_stamp(archive_dir, entry, &unmatched, stamp)
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
