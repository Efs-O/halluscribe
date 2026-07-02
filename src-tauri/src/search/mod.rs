// HalluScribe - session archive search module.
// Implements filter logic over IndexEntry records loaded from index.json.
// archive.rs owns all index read/write; this module owns query/filter only.
// Used both by Tauri commands and by the Gemma chat tool-call loop.

mod content;
mod filtering;
mod params;

pub(crate) use content::body_contains;
pub use params::SearchParams;

use crate::archive::{read_sessions, IndexEntry};
use std::collections::HashSet;
use std::path::Path;

/// Return sessions matching `params`, sorted newest-first, up to `limit`.
pub fn search_sessions(archive_dir: &Path, params: &SearchParams) -> Vec<IndexEntry> {
    search_sessions_in_scope(archive_dir, params, None)
}

pub fn search_sessions_in_scope(
    archive_dir: &Path,
    params: &SearchParams,
    allowed_ids: Option<&HashSet<String>>,
) -> Vec<IndexEntry> {
    let limit = params.limit.unwrap_or(20).min(30);
    let mut sessions = read_sessions(archive_dir);
    sessions.sort_by_key(|s| std::cmp::Reverse(s.date.clone()));
    sessions
        .into_iter()
        .filter(|entry| {
            allowed_ids
                .map(|ids| ids.contains(&entry.id))
                .unwrap_or(true)
        })
        .filter(|entry| filtering::matches_params(archive_dir, entry, params))
        .take(limit)
        .collect()
}

/// Search sessions by query, checking metadata first then full .md content.
/// Returns all matches sorted newest-first - no limit (caller paginates).
/// Same logic as the briefing keyword filter so behaviour is consistent.
pub fn search_fulltext(archive_dir: &Path, query: &str) -> Vec<IndexEntry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        let mut all = read_sessions(archive_dir);
        all.sort_by_key(|s| std::cmp::Reverse(s.date.clone()));
        return all;
    }

    let mut results: Vec<IndexEntry> = read_sessions(archive_dir)
        .into_iter()
        .filter(|entry| content::matches_fulltext(archive_dir, entry, &q))
        .collect();
    results.sort_by_key(|s| std::cmp::Reverse(s.date.clone()));
    results
}

/// Read the full markdown content of a session by its `session_id` (= index `id`).
pub fn read_session(archive_dir: &Path, session_id: &str) -> Result<String, String> {
    read_session_in_scope(archive_dir, session_id, None)
}

pub fn read_session_in_scope(
    archive_dir: &Path,
    session_id: &str,
    allowed_ids: Option<&HashSet<String>>,
) -> Result<String, String> {
    if let Some(ids) = allowed_ids {
        if !ids.contains(session_id) {
            return Err(format!(
                "session '{session_id}' is outside the active chat scope"
            ));
        }
    }
    content::read_session(archive_dir, session_id)
}

#[cfg(test)]
mod tests;
