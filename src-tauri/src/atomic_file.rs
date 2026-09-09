// HalluScribe - small shared atomic file replacement helper for durable state.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Replace `path` with `contents` through a sibling temporary file. A crash
/// before rename leaves the previous durable file intact; callers can surface
/// the returned I/O error instead of interpreting damaged data as empty.
pub(crate) fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic write target must include a file name",
        )
    })?;
    // A deterministic temp name would let two concurrent writers to the same
    // target interleave into one shared temp file and race on rename. A
    // process-id + monotonic counter suffix makes each writer's temp unique.
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        file_name.to_string_lossy(),
        std::process::id(),
        sequence
    ));
    fs::write(&temporary, contents)?;
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_leaves_only_the_new_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, "old").unwrap();

        write_atomic(&path, "new").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert!(!dir.path().join(".state.json.tmp").exists());
    }

    #[test]
    fn concurrent_writers_do_not_share_a_temp_file() {
        // The old deterministic temp name (`.{name}.tmp`) let two writers to the
        // same target interleave into one temp and race on rename. Unique temp
        // names mean every concurrent write lands intact and none leaves a
        // stray temp file behind.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, "seed").unwrap();

        let mut handles = Vec::new();
        for index in 0..16u8 {
            let target = path.clone();
            handles.push(std::thread::spawn(move || {
                write_atomic(&target, format!("payload-{index}")).unwrap();
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        let final_contents = fs::read_to_string(&path).unwrap();
        assert!(
            final_contents.starts_with("payload-"),
            "final write is intact: {final_contents}"
        );
        let strays: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(strays.is_empty(), "no temp files left behind: {strays:?}");
    }
}
