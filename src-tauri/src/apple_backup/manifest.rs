// HalluScribe - opens an iPhone backup read-only and resolves its files.
//
// A backup is a directory holding `Manifest.db` (a SQLite index of every file
// the phone backed up) plus hashed payload files at `<dir>/<fileID[0..2]>/<fileID>`.
// `open_backup` reads the manifest into an in-memory index; `resolve` maps a
// (domain, relativePath) pair to the on-disk path if the file actually exists;
// `copy_to_temp` copies a file out so its SQLite can be opened on a temp copy,
// never touching the backup directory. A `-wal` the backup holds beside the
// database is copied with it, so rows written since the last checkpoint are
// read too. Handles alive at the same time share one manifest index, so a
// sweep that keeps one handle open copies and reads `Manifest.db` once.
//
// This is the Phase 1 probe layer: it reports metadata only.

use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::SystemTime;

#[path = "temp_copy.rs"]
mod temp_copy;
pub use temp_copy::TempCopy;

/// Errors from opening or reading a backup.
#[derive(Debug)]
pub enum BackupError {
    /// The directory is not a backup: it is missing, or has no `Manifest.db`.
    NotABackup,
    /// `Manifest.db` exists but does not start with the SQLite header, i.e.
    /// the backup is encrypted.
    Encrypted,
    /// `Manifest.db` is a SQLite file but its `Files` table could not be read.
    ManifestUnreadable(String),
    /// A filesystem or SQLite I/O failure (reading the header, copying a file).
    Io(String),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::NotABackup => write!(f, "not a backup (missing Manifest.db)"),
            BackupError::Encrypted => write!(f, "backup is encrypted"),
            BackupError::ManifestUnreadable(msg) => write!(f, "manifest unreadable: {msg}"),
            BackupError::Io(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl std::error::Error for BackupError {}

/// An opened, decrypted backup: its directory plus the in-memory manifest index.
#[derive(Debug)]
pub struct BackupHandle {
    dir: PathBuf,
    manifest: Arc<ManifestIndex>,
}

/// The in-memory manifest, shared by every handle open on the same unchanged
/// `Manifest.db`.
#[derive(Debug)]
struct ManifestIndex {
    /// (domain, relativePath) -> fileID, from the `Files` table.
    index: HashMap<(String, String), String>,
    /// Every distinct domain seen in the manifest.
    domains: Vec<String>,
}

/// Identifies one version of one backup's `Manifest.db`: a rewritten backup
/// changes its size or mtime, so a stale index is never reused.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestKey {
    dir: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

/// The manifests read so far, held weakly: one is reused only while some handle
/// still owns it, so nothing stays in memory after the last one drops.
static SHARED_MANIFESTS: Mutex<Vec<(ManifestKey, Weak<ManifestIndex>)>> = Mutex::new(Vec::new());

/// Open a backup directory, reading its manifest into memory.
///
/// Returns `NotABackup` if the directory or `Manifest.db` is missing,
/// `Encrypted` if the manifest is not a plain SQLite file, and
/// `ManifestUnreadable` if the `Files` table cannot be read.
pub fn open_backup(dir: &Path) -> Result<BackupHandle, BackupError> {
    let manifest_path = dir.join("Manifest.db");
    if !dir.is_dir() || !manifest_path.is_file() {
        return Err(BackupError::NotABackup);
    }

    // Header check: an unencrypted backup's Manifest.db starts with the
    // SQLite magic; an encrypted one does not.
    const MAGIC: &[u8] = b"SQLite format 3\0";
    let mut file = File::open(&manifest_path).map_err(|e| BackupError::Io(e.to_string()))?;
    let mut header = [0u8; MAGIC.len()];
    file.read_exact(&mut header)
        .map_err(|e| BackupError::Io(e.to_string()))?;
    if header != MAGIC {
        return Err(BackupError::Encrypted);
    }

    // Reuse the index another live handle already read from this unchanged
    // manifest, instead of copying and reading it again.
    let meta = fs::metadata(&manifest_path).map_err(|e| BackupError::Io(e.to_string()))?;
    let key = ManifestKey {
        dir: dir.to_path_buf(),
        len: meta.len(),
        modified: meta.modified().ok(),
    };
    let mut shared = SHARED_MANIFESTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    // Forget dropped indexes and older versions of this backup's manifest.
    shared.retain(|(k, weak)| weak.strong_count() > 0 && (k.dir != key.dir || *k == key));
    if let Some(manifest) = shared
        .iter()
        .find(|(k, _)| *k == key)
        .and_then(|(_, w)| w.upgrade())
    {
        return Ok(BackupHandle {
            dir: key.dir,
            manifest,
        });
    }

    // Read the Files table through a temp copy so the original is never
    // opened (Apple Devices may be writing it).
    let temp = TempCopy::new(&manifest_path)?;
    let manifest = Arc::new(read_files_table(temp.path())?);
    shared.push((key, Arc::downgrade(&manifest)));
    Ok(BackupHandle {
        dir: dir.to_path_buf(),
        manifest,
    })
}

impl BackupHandle {
    /// Resolve a logical file to its on-disk path, if the manifest has a row
    /// for it AND the hashed file exists in the backup directory.
    pub fn resolve(&self, domain: &str, relative_path: &str) -> Option<PathBuf> {
        let file_id = self
            .manifest
            .index
            .get(&(domain.to_string(), relative_path.to_string()))?;
        let hashed = hashed_path(&self.dir, file_id)?;
        if hashed.is_file() {
            Some(hashed)
        } else {
            None
        }
    }

    /// Distinct domains from the manifest matching `needle` as a
    /// case-insensitive substring, sorted and deduped.
    pub fn domains_matching(&self, needle: &str) -> Vec<String> {
        let needle = needle.to_ascii_lowercase();
        let mut out: Vec<String> = self
            .manifest
            .domains
            .iter()
            .filter(|d| d.to_ascii_lowercase().contains(&needle))
            .cloned()
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// The `relativePath`s the manifest lists for one domain, sorted and
    /// deduped. Used by the probe to find a domain's database files; the
    /// resolver itself only needs `resolve`/`copy_to_temp`.
    pub fn files_in_domain(&self, domain: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .manifest
            .index
            .keys()
            .filter(|(d, _)| d == domain)
            .map(|(_, rel)| rel.clone())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Copy a logical file to a unique temp path and return a guard that
    /// deletes it on drop. When the backup also holds the file's `-wal`, that
    /// is copied beside it (see `TempCopy::open`). Errors if the file does not
    /// resolve or a copy fails.
    pub fn copy_to_temp(&self, domain: &str, relative_path: &str) -> Result<TempCopy, BackupError> {
        let source = self.resolve(domain, relative_path).ok_or_else(|| {
            BackupError::Io(format!(
                "file not found in backup: {domain}/{relative_path}"
            ))
        })?;
        let mut copy = TempCopy::new(&source)?;
        if let Some(wal) = self.resolve(domain, &format!("{relative_path}-wal")) {
            copy.attach_wal(&wal)?;
        }
        Ok(copy)
    }
}

/// The on-disk location of a hashed backup file: `<dir>/<fileID[0..2]>/<fileID>`.
fn hashed_path(dir: &Path, file_id: &str) -> Option<PathBuf> {
    let sub = file_id.get(..2)?;
    Some(dir.join(sub).join(file_id))
}

/// Read the `Files` table of a (temp-copy) manifest into an index plus the
/// distinct domain list.
fn read_files_table(path: &Path) -> Result<ManifestIndex, BackupError> {
    let conn =
        open_sqlite_read_only(path).map_err(|e| BackupError::ManifestUnreadable(e.to_string()))?;
    let mut stmt = conn
        .prepare("SELECT fileID, domain, relativePath FROM Files")
        .map_err(|e| BackupError::ManifestUnreadable(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| BackupError::ManifestUnreadable(e.to_string()))?;

    let mut index: HashMap<(String, String), String> = HashMap::new();
    let mut domains: Vec<String> = Vec::new();
    for row in rows {
        let (file_id, domain, rel) =
            row.map_err(|e| BackupError::ManifestUnreadable(e.to_string()))?;
        domains.push(domain.clone());
        index.insert((domain, rel), file_id);
    }
    Ok(ManifestIndex { index, domains })
}

/// Open a SQLite file read-only via an `immutable=1` URI. The file is treated
/// as never changing, so SQLite takes no locks and never writes a journal into
/// the backup directory. Shared with the probe binary, which must not carry its
/// own copy of the URI/flags logic.
pub fn open_sqlite_read_only(path: &Path) -> rusqlite::Result<Connection> {
    let uri = format!("file:{}?immutable=1", path_to_uri(path));
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
}

/// Turn a filesystem path into the path portion of a `file:` URI: forward
/// slashes, and every byte outside the unreserved set percent-encoded (so a
/// space in the temp dir becomes `%20`). The drive-letter colon is left
/// literal; `?`/`#` cannot appear in a Windows path.
fn path_to_uri(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "manifest_wal_tests.rs"]
mod wal_tests;
