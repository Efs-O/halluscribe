// HalluScribe - raw transcript preservation (Persona Protocol Phase 1).
//
// The archive normally keeps only the Gemma summary plus a `source_jsonl`
// pointer to the original. Coding tools prune old logs, so that pointer dangles
// and the raw detail is lost forever. When `preserve_raw_transcripts` is on, the
// sweep copies each archived session's original transcript here, compressed with
// zstd (agent JSONL compresses roughly 10:1), so a persona layer built on
// summaries can always drill back to ground truth.
//
// Raw copies are the UNTOUCHED source: redaction and the secret scan apply only
// to the shareable `.md` summaries, never to these files. Persona Pack exports
// exclude the `raw/` directory unless the user opts in per-export, so raw secrets
// never leave the machine implicitly.

use super::ArchiveError;
use std::path::{Component, Path};

/// Archive-relative directory holding preserved raw transcripts.
pub const RAW_DIR: &str = "raw";

/// zstd compression level. Level 3 (the library default) gives near-10:1 on
/// agent JSONL while staying fast enough to run inline during the sweep.
const ZSTD_LEVEL: i32 = 3;

/// Archive-relative path for a session's preserved raw transcript.
pub fn raw_rel_path(session_id: &str) -> String {
    format!("{RAW_DIR}/{session_id}.jsonl.zst")
}

/// Compress `source` into `<archive_dir>/raw/<session_id>.jsonl.zst`, overwriting
/// any existing copy (a re-swept, changed session replaces its raw copy — the
/// transcript hash already detected the change). Returns the archive-relative
/// path to record in the index entry.
pub fn preserve_raw(
    archive_dir: &Path,
    session_id: &str,
    source: &Path,
) -> Result<String, ArchiveError> {
    let rel = raw_rel_path(session_id);
    let dest = archive_dir.join(&rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = std::fs::read(source)?;
    let compressed = zstd::encode_all(bytes.as_slice(), ZSTD_LEVEL)?;
    std::fs::write(&dest, compressed)?;
    Ok(rel)
}

/// Read and decompress a preserved raw transcript back to its original text.
/// Used by future readers (MCP `read_session`, Persona Pack) that want the
/// verbatim source rather than the summary.
pub fn read_raw(archive_dir: &Path, session_id: &str) -> Result<String, ArchiveError> {
    let path = archive_dir.join(raw_rel_path(session_id));
    let bytes = std::fs::read(&path)?;
    let decoded = zstd::decode_all(bytes.as_slice())?;
    String::from_utf8(decoded).map_err(|error| ArchiveError::Invalid(error.to_string()))
}

/// Read and decompress a raw transcript by its archive-relative path (the
/// index entry's recorded `raw_path`). Rejects paths that are absolute or
/// contain `..` components — index data must not escape the archive dir.
pub fn read_raw_at(archive_dir: &Path, rel_path: &str) -> Result<String, ArchiveError> {
    let rel = Path::new(rel_path);
    if rel_path.is_empty()
        || rel.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ArchiveError::Invalid(format!(
            "invalid archive-relative raw path: {rel_path}"
        )));
    }

    let bytes = std::fs::read(archive_dir.join(rel))?;
    let decoded = zstd::decode_all(bytes.as_slice())?;
    String::from_utf8(decoded).map_err(|error| ArchiveError::Invalid(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("halluscribe_raw_test_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn raw_rel_path_is_stable() {
        assert_eq!(raw_rel_path("abc-123"), "raw/abc-123.jsonl.zst");
    }

    #[test]
    fn preserve_then_read_round_trips_the_source() {
        let dir = tmp_dir("round_trip");
        let source = dir.join("session.jsonl");
        let contents = "{\"role\":\"user\"}\n{\"role\":\"assistant\"}\n".repeat(200);
        std::fs::write(&source, &contents).unwrap();

        let rel = preserve_raw(&dir, "abc-123", &source).unwrap();
        assert_eq!(rel, "raw/abc-123.jsonl.zst");
        assert!(dir.join(&rel).exists());
        assert_eq!(read_raw(&dir, "abc-123").unwrap(), contents);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compressed_copy_is_smaller_than_source() {
        let dir = tmp_dir("ratio");
        let source = dir.join("session.jsonl");
        // Repetitive JSONL compresses well; assert we actually shrank it.
        let contents = "{\"type\":\"message\",\"text\":\"hello world\"}\n".repeat(500);
        std::fs::write(&source, &contents).unwrap();

        preserve_raw(&dir, "id", &source).unwrap();
        let compressed_len = std::fs::metadata(dir.join(raw_rel_path("id")))
            .unwrap()
            .len();
        assert!((compressed_len as usize) < contents.len());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preserve_overwrites_existing_copy() {
        let dir = tmp_dir("overwrite");
        let source = dir.join("session.jsonl");

        std::fs::write(&source, "first\n").unwrap();
        preserve_raw(&dir, "id", &source).unwrap();
        std::fs::write(&source, "second version\n").unwrap();
        preserve_raw(&dir, "id", &source).unwrap();

        assert_eq!(read_raw(&dir, "id").unwrap(), "second version\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preserve_errors_when_source_missing() {
        let dir = tmp_dir("missing");
        let result = preserve_raw(&dir, "id", &dir.join("does-not-exist.jsonl"));
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
