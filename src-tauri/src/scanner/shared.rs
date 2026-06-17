#[cfg(test)]
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
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

/// Recursively collect all `.jsonl` files under `dir`, sorted by mtime descending.
pub(super) fn collect_jsonl(dir: &Path) -> Vec<(SystemTime, PathBuf)> {
    let mut files = Vec::new();
    collect_jsonl_inner(dir, &mut files);
    files.sort_by_key(|file| std::cmp::Reverse(file.0));
    files
}

fn collect_jsonl_inner(dir: &Path, out: &mut Vec<(SystemTime, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            collect_jsonl_inner(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            if let Ok(mtime) = meta.modified() {
                out.push((mtime, path));
            }
        }
    }
}

pub(super) fn mtime_secs(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |dur| dur.as_secs() as i64)
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
