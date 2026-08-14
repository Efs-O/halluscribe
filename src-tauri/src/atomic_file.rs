// HalluScribe - small shared atomic file replacement helper for durable state.

use std::fs;
use std::io;
use std::path::Path;

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
    let temporary = path.with_file_name(format!(".{}.tmp", file_name.to_string_lossy()));
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
}
