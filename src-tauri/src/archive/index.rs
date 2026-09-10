// HalluScribe - archive index load/save/delete helpers.

use super::{ArchiveError, IndexEntry};
use serde::{Deserialize, Serialize};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};

/// Outcome of a best-effort bulk delete. A failure leaves that session's index
/// row and private files intact, so it can safely be retried from the UI.
#[derive(Debug, Default, Serialize)]
pub struct DeleteSessionsResult {
    pub deleted_ids: Vec<String>,
    pub failures: Vec<DeleteSessionFailure>,
}

#[derive(Debug, Serialize)]
pub struct DeleteSessionFailure {
    pub id: String,
    pub error: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    sessions: Vec<IndexEntry>,
}

pub fn session_id(source: &Path) -> String {
    if let Some(stem) = source.file_stem().and_then(|s| s.to_str()) {
        if !stem.is_empty() {
            return stem.to_string();
        }
    }
    // A path with no usable stem (trailing dot, non-UTF-8 filename, a path
    // ending in `..`) must not collapse onto a single literal id: `append_index`
    // does `retain(|e| e.id != entry.id)` before pushing, so every such session
    // would evict the previous one. Fall back to a stable per-path hash so the
    // ids stay distinct and idempotent.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    format!("unknown-{:016x}", hasher.finish())
}

/// Resolve a proposed session ID against durable archive ownership. A legacy
/// bare stem remains its existing owner's ID; a different source requesting
/// that stem receives a deterministic suffix. The suffix is persisted by the
/// normal index/manifest writes, so its `DefaultHasher` value is never later
/// recomputed as the authority for an existing session.
pub fn resolve_session_id(archive_dir: &Path, source: &Path, proposed: &str) -> String {
    let source_text = source.to_string_lossy();
    let index_owner = load_index(archive_dir).ok().and_then(|index| {
        index
            .sessions
            .into_iter()
            .find(|entry| entry.id == proposed)
            .map(|entry| entry.source_jsonl)
    });
    let manifest_owner = super::captured_manifest::load_captured(archive_dir)
        .get(proposed)
        .map(|record| record.source_path.clone());
    let owners = [index_owner, manifest_owner];
    if owners.iter().flatten().any(|owner| owner == &source_text) {
        return proposed.to_string();
    }
    if owners.iter().flatten().next().is_some() {
        format!("{proposed}-{}", path_hash(source))
    } else {
        proposed.to_string()
    }
}

fn path_hash(source: &Path) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Serialise the read-modify-write cycle of every index mutation within this
/// process. Without it, a redaction clearing `secret_flags` racing a raw
/// backfill updating `raw_path` (or either racing the sweep's `append_index`)
/// is a classic lost update: the last writer wins and silently discards the
/// other. Poisoned-lock recovery keeps one panicking mutation from wedging all
/// later ones. Cross-process writers would need file locking, which is out of
/// scope here.
fn index_mutation_lock() -> std::sync::MutexGuard<'static, ()> {
    INDEX_MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

static INDEX_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// Exact `index.json` bytes, used as a cross-process cache freshness stamp.
/// `None` is deliberately never considered fresh by the body cache, so a
/// missing or unreadable index conservatively rebuilds rather than serving a
/// potentially stale corpus. Keeping the bytes rather than a metadata tuple
/// catches equal-length, same-timestamp rewrites without a new hash dependency.
pub type IndexStamp = Option<Vec<u8>>;

pub fn index_stamp(archive_dir: &Path) -> IndexStamp {
    std::fs::read(archive_dir.join("index.json")).ok()
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

pub fn delete_sessions(
    archive_dir: &Path,
    ids: &[String],
) -> Result<DeleteSessionsResult, ArchiveError> {
    let _guard = index_mutation_lock();
    let mut idx = load_index(archive_dir)?;
    let mut result = DeleteSessionsResult::default();
    for id in ids {
        if let Some(pos) = idx.sessions.iter().position(|e| &e.id == id) {
            let md = {
                let entry = &idx.sessions[pos];
                archive_path(archive_dir, &entry.archive_path)?
            };
            // Only drop the index row once the markdown is actually gone. A
            // `remove_file` failure (Windows: file open in an editor / AV lock)
            // must not abort the whole batch, and must not leave a row pointing
            // at a file we just deleted. On failure we keep both the file and
            // its row (consistent, retryable) and continue with the rest.
            // Delete private source material first. If that fails, leave the
            // summary and its index entry intact so the operation is retryable.
            let entry = &idx.sessions[pos];
            if let Err(error) = remove_private_session_files(archive_dir, entry, &md) {
                eprintln!("[archive] could not purge private files for {id}: {error}");
                result.failures.push(DeleteSessionFailure {
                    id: id.clone(),
                    error: error.to_string(),
                });
                continue;
            }
            if md.exists() {
                if let Err(error) = fs::remove_file(&md) {
                    eprintln!(
                        "[archive] could not remove session file {}: {error}",
                        md.display()
                    );
                    result.failures.push(DeleteSessionFailure {
                        id: id.clone(),
                        error: error.to_string(),
                    });
                    continue;
                }
            }
            idx.sessions.remove(pos);
            result.deleted_ids.push(id.clone());
        }
    }
    if !result.deleted_ids.is_empty() {
        save_index(archive_dir, &idx)?;
    }
    Ok(result)
}

pub(super) fn archive_path(archive_dir: &Path, relative: &str) -> Result<PathBuf, ArchiveError> {
    let path = Path::new(relative);
    if relative.is_empty()
        || path.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ArchiveError::Invalid(format!(
            "invalid archive-relative path: {relative}"
        )));
    }
    Ok(archive_dir.join(path))
}

fn remove_private_session_files(
    archive_dir: &Path,
    entry: &IndexEntry,
    markdown: &Path,
) -> Result<(), ArchiveError> {
    remove_if_file(&archive_dir.join(super::raw_rel_path(&entry.id)))?;
    if !entry.raw_path.is_empty() {
        remove_if_file(&archive_path(archive_dir, &entry.raw_path)?)?;
    }
    let superseded = archive_dir.join("raw").join("superseded");
    if let Ok(files) = fs::read_dir(superseded) {
        let prefix = format!("{}.", entry.id);
        for file in files.flatten() {
            let name = file.file_name();
            if name.to_string_lossy().starts_with(&prefix) {
                remove_if_file(&file.path())?;
            }
        }
    }
    let Some(parent) = markdown.parent() else {
        return Ok(());
    };
    let Some(file_name) = markdown.file_name().and_then(|name| name.to_str()) else {
        return Ok(());
    };
    let backup_prefix = format!("{file_name}.bak-");
    if let Ok(files) = fs::read_dir(parent) {
        for file in files.flatten() {
            if file
                .file_name()
                .to_string_lossy()
                .starts_with(&backup_prefix)
            {
                remove_if_file(&file.path())?;
            }
        }
    }
    Ok(())
}

fn remove_if_file(path: &Path) -> Result<(), ArchiveError> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Replace the `secret_flags` on the index entry matching `id`. No-op (Ok) if
/// the id is not present in the index - used after a redaction rewrites a
/// session so the badge clears/updates immediately, not just on next sweep.
pub fn set_secret_flags(
    archive_dir: &Path,
    id: &str,
    flags: Vec<String>,
) -> Result<(), ArchiveError> {
    let _guard = index_mutation_lock();
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
    let _guard = index_mutation_lock();
    let mut idx = load_index(archive_dir)?;
    let Some(entry) = idx.sessions.iter_mut().find(|e| e.id == id) else {
        return Ok(());
    };
    entry.raw_path = rel;
    save_index(archive_dir, &idx)?;
    Ok(())
}

pub(super) fn append_index(archive_dir: &Path, entry: IndexEntry) -> Result<(), ArchiveError> {
    let _guard = index_mutation_lock();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_entry(id: &str) -> IndexEntry {
        IndexEntry {
            id: id.to_string(),
            project: "proj".into(),
            date: "2026-04-15".into(),
            title: format!("Session {id}"),
            tool: "Claude Code".into(),
            fill_pct: 10.0,
            session_timestamp: "2026-04-15T14:30:00+00:00".into(),
            updated_at: String::new(),
            session_type: "Debugging".into(),
            error_tags: vec![],
            topic_tags: vec![],
            archive_path: format!("{id}.md"),
            source_jsonl: format!("/fake/{id}.jsonl"),
            source_size_bytes: 10,
            provider: "claude_code".into(),
            fill_estimated: true,
            output_tokens: 0,
            tokens_estimated: true,
            transcript_hash: "hash".into(),
            secret_flags: vec![],
            raw_path: String::new(),
        }
    }

    fn test_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "halluscribe_index_test_{}_{}",
            std::process::id(),
            name
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_poisoned_mutation_lock_does_not_wedge_index_writes() {
        let dir = test_dir("poison_recovery");
        append_index(&dir, sample_entry("first")).unwrap();
        // Poison the lock the way a panicking holder would.
        drop(INDEX_MUTEX.lock().unwrap_or_else(|e| e.into_inner()));
        std::panic::catch_unwind(|| {
            let _g = INDEX_MUTEX.lock().unwrap();
            panic!("holder panicked while holding the index lock");
        })
        .expect_err("the holder panics by design");
        // The next mutation must recover instead of propagating the poison.
        append_index(&dir, sample_entry("second")).unwrap();
        let ids: Vec<String> = read_sessions(&dir).into_iter().map(|e| e.id).collect();
        assert_eq!(ids, vec!["first", "second"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_mutations_never_lose_rows_or_corrupt_the_index() {
        // Without the mutation lock, two read-modify-write threads each load
        // the pre-update index and the later rename drops the other's row.
        let dir = test_dir("concurrent_mutations");
        let mut handles = Vec::new();
        for i in 0..8 {
            let d = dir.clone();
            handles.push(
                std::thread::Builder::new()
                    .stack_size(8 * 1024 * 1024)
                    .spawn(move || append_index(&d, sample_entry(&format!("s{i}"))).unwrap())
                    .unwrap(),
            );
        }
        for h in handles {
            h.join().unwrap();
        }
        let entries = read_sessions(&dir);
        assert_eq!(entries.len(), 8, "every concurrent append must survive");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn collision_resolver_keeps_legacy_owner_and_suffixes_the_new_source() {
        let dir = test_dir("collision_resolver");
        let first = Path::new("C:/one/session.jsonl");
        let second = Path::new("C:/two/session.jsonl");
        let mut legacy = sample_entry("session");
        legacy.source_jsonl = first.to_string_lossy().to_string();
        append_index(&dir, legacy).unwrap();

        assert_eq!(resolve_session_id(&dir, first, "session"), "session");
        let second_id = resolve_session_id(&dir, second, "session");
        assert!(second_id.starts_with("session-"));
        assert_ne!(second_id, "session");
        let _ = fs::remove_dir_all(&dir);
    }
}
