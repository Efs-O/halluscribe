#[cfg(test)]
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

/// Environment variables are process-global. Tests that temporarily redirect
/// HOME/USERPROFILE must use this crate-wide lock rather than a module-local
/// mutex so concurrent scanners cannot observe another fixture's fake home.
#[cfg(test)]
pub(crate) static HOME_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(super) fn home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                let drive = std::env::var("HOMEDRIVE").ok()?;
                let path = std::env::var("HOMEPATH").ok()?;
                Some(PathBuf::from(format!("{drive}{path}")))
            })
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}

/// How deep below a scan root `collect_jsonl` will descend. Real coding-tool
/// layouts nest only a few levels (`.claude/projects`, `.codex/sessions/YYYY/
/// MM/DD`), so this never truncates a genuine tree; it bounds the recursion
/// against a pathologically deep (or, on a platform whose `DirEntry::metadata`
/// follows links, cyclic) directory so a scan cannot exhaust the stack.
const MAX_SCAN_DEPTH: usize = 12;

/// Recursively collect all `.jsonl` files under `dir`, sorted by mtime descending.
pub(super) fn collect_jsonl(dir: &Path) -> Vec<(SystemTime, PathBuf)> {
    let mut files = Vec::new();
    collect_jsonl_inner(dir, &mut files, 0);
    files.sort_by_key(|file| std::cmp::Reverse(file.0));
    files
}

/// `depth` counts directories descended from the scan root. A subdirectory is
/// entered only while `depth` stays within [`MAX_SCAN_DEPTH`], so a file sitting
/// below `MAX_SCAN_DEPTH` directories is not collected.
fn collect_jsonl_inner(dir: &Path, out: &mut Vec<(SystemTime, PathBuf)>, depth: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            if depth < MAX_SCAN_DEPTH {
                collect_jsonl_inner(&path, out, depth + 1);
            }
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            if let Ok(mtime) = meta.modified() {
                out.push((mtime, path));
            }
        }
    }
}

/// Test-only view of the depth limit so a boundary test can build exactly the
/// tree it needs without hard-coding the constant in two places.
#[cfg(test)]
pub(super) const SCAN_DEPTH_LIMIT: usize = MAX_SCAN_DEPTH;

/// File modified timestamp as seconds since the Unix epoch.
///
/// A pre-1970 mtime (a restored backup, clock skew) makes `duration_since`
/// return `Err`; the old code collapsed that to `0`, so such a file became
/// indistinguishable from an epoch-zero file and mis-sorted the newest-first
/// scan order the sweep depends on. Return the negative offset instead, so a
/// pre-epoch file sorts honestly as older than every post-epoch one.
pub(crate) fn mtime_secs(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(dur) => dur.as_secs() as i64,
        Err(err) => -(err.duration().as_secs() as i64),
    }
}

#[cfg(test)]
pub(super) fn path_to_session_id(path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(super) fn cutoff(lookback_secs: u64) -> SystemTime {
    SystemTime::now()
        .checked_sub(Duration::from_secs(lookback_secs))
        .unwrap_or(UNIX_EPOCH)
}
