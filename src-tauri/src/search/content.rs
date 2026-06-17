use crate::archive::{read_sessions, IndexEntry};
use std::{fs, path::Path};

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

    let md_path = archive_dir.join(&entry.archive_path);
    fs::read_to_string(&md_path)
        .map(|markdown| markdown.to_lowercase().contains(query))
        .unwrap_or(false)
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
