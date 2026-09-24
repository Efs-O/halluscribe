// HalluScribe - private temp copies of backup files, for the backup resolver.
//
// A `TempCopy` is a file copied out of a backup (plus its `-wal`, when the
// backup holds one) to a unique temp path, so its SQLite can be opened without
// ever touching the backup directory. The copy deletes itself on drop.

use super::{open_sqlite_read_only, BackupError};
use rusqlite::Connection;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A temp copy of a backup file (plus its `-wal`, when the backup has one).
/// Deletes itself, and any `-wal`/`-shm` SQLite left beside it, on drop.
pub struct TempCopy {
    path: PathBuf,
    has_wal: bool,
}

impl TempCopy {
    pub(super) fn new(source: &Path) -> Result<TempCopy, BackupError> {
        let dest = unique_temp_path();
        fs::copy(source, &dest).map_err(|e| BackupError::Io(e.to_string()))?;
        Ok(TempCopy {
            path: dest,
            has_wal: false,
        })
    }

    /// Copy `wal` beside this copy as `<path>-wal`.
    pub(super) fn attach_wal(&mut self, wal: &Path) -> Result<(), BackupError> {
        fs::copy(wal, sibling(&self.path, "-wal")).map_err(|e| BackupError::Io(e.to_string()))?;
        self.has_wal = true;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Open the copied database. Without a `-wal` it is opened read-only and
    /// `immutable`; with one, it is opened normally so SQLite replays the WAL.
    /// That is safe because the copy is private to this process and deleted on
    /// drop - the backup itself is never opened.
    pub fn open(&self) -> rusqlite::Result<Connection> {
        if self.has_wal {
            Connection::open(&self.path)
        } else {
            open_sqlite_read_only(&self.path)
        }
    }
}

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        // Best-effort: these exist only when the copy had a WAL.
        let _ = fs::remove_file(sibling(&self.path, "-wal"));
        let _ = fs::remove_file(sibling(&self.path, "-shm"));
    }
}

/// `path` with `suffix` appended to its file name (`x.db` -> `x.db-wal`).
pub(super) fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique path under `std::env::temp_dir()`: pid + nanos + a per-process
/// counter, so concurrent copies never collide.
fn unique_temp_path() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "halluscribe-backup-{}-{}-{}",
        std::process::id(),
        nanos,
        counter
    ))
}
