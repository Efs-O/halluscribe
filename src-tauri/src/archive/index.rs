// HalluScribe - archive index load/save/delete helpers.

use super::{ArchiveError, IndexEntry};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    sessions: Vec<IndexEntry>,
}

pub fn session_id(source: &Path) -> String {
    source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

pub fn find_session(archive_dir: &Path, id: &str) -> Option<IndexEntry> {
    load_index(archive_dir)
        .ok()?
        .sessions
        .into_iter()
        .find(|entry| entry.id == id)
}

pub fn read_sessions(archive_dir: &Path) -> Vec<IndexEntry> {
    load_index(archive_dir)
        .map(|idx| idx.sessions)
        .unwrap_or_default()
}

pub fn is_archived(archive_dir: &Path, id: &str) -> bool {
    load_index(archive_dir)
        .map(|idx| idx.sessions.iter().any(|e| e.id == id))
        .unwrap_or(false)
}

pub fn archived_source_size(archive_dir: &Path, id: &str) -> Option<u64> {
    let size = load_index(archive_dir)
        .ok()?
        .sessions
        .into_iter()
        .find(|e| e.id == id)
        .map(|e| e.source_size_bytes)?;
    if size == 0 {
        None
    } else {
        Some(size)
    }
}

pub fn delete_sessions(archive_dir: &Path, ids: &[String]) -> Result<Vec<String>, ArchiveError> {
    let mut idx = load_index(archive_dir)?;
    let mut deleted = Vec::new();
    for id in ids {
        if let Some(pos) = idx.sessions.iter().position(|e| &e.id == id) {
            let entry = idx.sessions.remove(pos);
            let md = archive_dir.join(&entry.archive_path);
            if md.exists() {
                fs::remove_file(&md)?;
            }
            deleted.push(id.clone());
        }
    }
    if !deleted.is_empty() {
        fs::write(
            archive_dir.join("index.json"),
            serde_json::to_string_pretty(&idx)?,
        )?;
    }
    Ok(deleted)
}

/// Replace the `secret_flags` on the index entry matching `id`. No-op (Ok) if
/// the id is not present in the index - used after a redaction rewrites a
/// session so the badge clears/updates immediately, not just on next sweep.
pub fn set_secret_flags(
    archive_dir: &Path,
    id: &str,
    flags: Vec<String>,
) -> Result<(), ArchiveError> {
    let mut idx = load_index(archive_dir)?;
    let Some(entry) = idx.sessions.iter_mut().find(|e| e.id == id) else {
        return Ok(());
    };
    entry.secret_flags = flags;
    fs::write(
        archive_dir.join("index.json"),
        serde_json::to_string_pretty(&idx)?,
    )?;
    Ok(())
}

pub(super) fn append_index(archive_dir: &Path, entry: IndexEntry) -> Result<(), ArchiveError> {
    fs::create_dir_all(archive_dir)?;
    let mut idx = load_index(archive_dir).unwrap_or_default();
    idx.sessions.retain(|e| e.id != entry.id);
    idx.sessions.push(entry);
    fs::write(
        archive_dir.join("index.json"),
        serde_json::to_string_pretty(&idx)?,
    )?;
    Ok(())
}

fn load_index(archive_dir: &Path) -> Result<Index, ArchiveError> {
    let p = archive_dir.join("index.json");
    if !p.exists() {
        return Ok(Index::default());
    }
    let raw = fs::read_to_string(p)?;
    Ok(serde_json::from_str(&raw)?)
}
