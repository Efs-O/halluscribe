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

/// Verify that the archive index is either absent (a new archive) or readable.
/// Call UI and job entry points before treating an empty list as meaningful.
pub fn ensure_index_readable(archive_dir: &Path) -> Result<(), ArchiveError> {
    load_index(archive_dir).map(|_| ())
}

pub fn find_session(archive_dir: &Path, id: &str) -> Option<IndexEntry> {
    load_index(archive_dir)
        .ok()?
        .sessions
        .into_iter()
        .find(|entry| entry.id == id)
}

/// Size and modification-time of the archive's `index.json`, as a cheap
/// "has this archive changed" stamp. Used by the search body cache to notice a
/// sweep run by another process without stat-ing every session file. A missing
/// or unreadable index stamps as `(0, 0)`, which simply never matches a real
/// one and so errs towards rebuilding.
pub type IndexStamp = (u64, u64);

pub fn index_stamp(archive_dir: &Path) -> IndexStamp {
    let Ok(meta) = std::fs::metadata(archive_dir.join("index.json")) else {
        return (0, 0);
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
        .unwrap_or(0);
    (meta.len(), modified)
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
        save_index(archive_dir, &idx)?;
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
    save_index(archive_dir, &idx)?;
    Ok(())
}

/// Replace the `raw_path` on the index entry matching `id`. No-op (Ok) if the
/// id is not present in the index - used by the raw backfill so a recovered
/// entry's `raw_path` updates immediately, not just on next sweep.
pub fn set_raw_path(archive_dir: &Path, id: &str, rel: String) -> Result<(), ArchiveError> {
    let mut idx = load_index(archive_dir)?;
    let Some(entry) = idx.sessions.iter_mut().find(|e| e.id == id) else {
        return Ok(());
    };
    entry.raw_path = rel;
    save_index(archive_dir, &idx)?;
    Ok(())
}

pub(super) fn append_index(archive_dir: &Path, entry: IndexEntry) -> Result<(), ArchiveError> {
    fs::create_dir_all(archive_dir)?;
    // `load_index` already treats an absent index as a new empty archive. Do
    // not extend that recovery to a corrupt or unreadable existing file: doing
    // so would replace every prior entry with just this new one.
    let mut idx = load_index(archive_dir)?;
    idx.sessions.retain(|e| e.id != entry.id);
    idx.sessions.push(entry);
    save_index(archive_dir, &idx)?;
    Ok(())
}

fn save_index(archive_dir: &Path, index: &Index) -> Result<(), ArchiveError> {
    crate::atomic_file::write_atomic(
        &archive_dir.join("index.json"),
        serde_json::to_string_pretty(index)?,
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
