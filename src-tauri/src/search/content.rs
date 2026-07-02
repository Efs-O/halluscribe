// HalluScribe - session content search: shared .md body read + full-text matcher.

use crate::archive::{read_sessions, IndexEntry};
use std::{fs, path::Path};

/// Read a session's `.md` body and report whether it contains `query`.
/// `query` must already be lowercased. This is the single disk-read fallback
/// shared by every keyword search path (chat tool, session list, briefing),
/// so the file-read logic lives in exactly one place.
pub(crate) fn body_contains(archive_dir: &Path, entry: &IndexEntry, query: &str) -> bool {
    let md_path = archive_dir.join(&entry.archive_path);
    fs::read_to_string(&md_path)
        .map(|markdown| markdown.to_lowercase().contains(query))
        .unwrap_or(false)
}

pub(super) fn matches_fulltext(archive_dir: &Path, entry: &IndexEntry, query: &str) -> bool {
    if entry.title.to_lowercase().contains(query) {
        return true;
    }
    if entry.project.to_lowercase().contains(query) {
        return true;
    }
    if entry
        .error_tags
        .iter()
        .any(|tag| tag.to_lowercase().contains(query))
    {
        return true;
    }
    if entry
        .topic_tags
        .iter()
        .any(|tag| tag.to_lowercase().contains(query))
    {
        return true;
    }

    body_contains(archive_dir, entry, query)
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
