// HalluScribe - permanent-delete records, so a deleted session is never re-archived.

use super::ArchiveError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Deleted session ids, next to `index.json`. A deleted session's source is
/// usually still on disk, and without this record the next sweep or capture
/// would archive it again as if it were new.
pub const DELETED_SESSIONS_FILE: &str = "deleted_sessions.json";

/// One permanently deleted session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tombstone {
    pub deleted_at: String,
    /// The source the session was read from. Empty means "any source".
    #[serde(default)]
    pub source_path: String,
}

impl Tombstone {
    /// Whether this record covers the session read from `source`. A different
    /// source that happens to propose the same id is a different session.
    pub fn covers(&self, source: &Path) -> bool {
        self.source_path.is_empty() || self.source_path == source.to_string_lossy()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DeletedSessions {
    sessions: BTreeMap<String, Tombstone>,
}

/// Every deleted session by id. A missing file means nothing was deleted; an
/// unreadable or corrupt one is an error, because treating it as empty would
/// silently bring every deleted session back on the next sweep.
pub fn load_deleted(archive_dir: &Path) -> Result<BTreeMap<String, Tombstone>, ArchiveError> {
    let path = archive_dir.join(DELETED_SESSIONS_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeMap::new());
        }
        Err(error) => return Err(error.into()),
    };
    let deleted: DeletedSessions = serde_json::from_str(&raw).map_err(|error| {
        ArchiveError::Invalid(format!("{DELETED_SESSIONS_FILE} is unreadable: {error}"))
    })?;
    Ok(deleted.sessions)
}

/// Refuse to run a sweep or capture when the delete records cannot be read.
pub fn ensure_deleted_readable(archive_dir: &Path) -> Result<(), ArchiveError> {
    load_deleted(archive_dir).map(|_| ())
}

/// Add `(id, source_path)` pairs to the delete records. Callers hold the index
/// mutation lock, so this read-modify-write cannot lose a concurrent delete.
pub(super) fn record_deleted(
    archive_dir: &Path,
    sessions: &[(String, String)],
    now: DateTime<Utc>,
) -> Result<(), ArchiveError> {
    if sessions.is_empty() {
        return Ok(());
    }
    let mut deleted = DeletedSessions {
        sessions: load_deleted(archive_dir)?,
    };
    for (id, source_path) in sessions {
        deleted.sessions.insert(
            id.clone(),
            Tombstone {
                deleted_at: now.to_rfc3339(),
                source_path: source_path.clone(),
            },
        );
    }
    crate::atomic_file::write_atomic(
        &archive_dir.join(DELETED_SESSIONS_FILE),
        serde_json::to_string_pretty(&deleted)?,
    )?;
    Ok(())
}
