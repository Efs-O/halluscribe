// HalluScribe - raw preservation tests: one-file-per-session overwrite,
// multi-session byte preservation, and the supersede-before-replace path
// that keeps a shrunken chat re-export from destroying the fuller raw.

use super::raw::*;
use std::path::Path;

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("halluscribe_raw_test_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every `.jsonl.zst` currently parked under `raw/superseded/`.
fn superseded_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir.join(SUPERSEDED_DIR)) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    paths.sort();
    paths
}

fn read_zst(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap();
    String::from_utf8(zstd::decode_all(bytes.as_slice()).unwrap()).unwrap()
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

#[test]
fn preserve_leaves_no_tmp_file_behind() {
    let dir = tmp_dir("atomic");
    let source = dir.join("session.jsonl");
    std::fs::write(&source, "{\"role\":\"user\"}\n").unwrap();

    let rel = preserve_raw(&dir, "abc-123", &source).unwrap();
    assert!(dir.join(&rel).exists());
    assert!(!dir.join("raw/abc-123.jsonl.zst.tmp").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn coding_sources_keep_no_superseded_versions() {
    // The 1:1 path is append-only by nature: versioning it would snapshot
    // every growing Claude Code session on every sweep for no benefit.
    let dir = tmp_dir("coding_no_versions");
    let source = dir.join("session.jsonl");

    std::fs::write(&source, "first\n").unwrap();
    preserve_raw(&dir, "id", &source).unwrap();
    std::fs::write(&source, "first\nsecond\n").unwrap();
    preserve_raw(&dir, "id", &source).unwrap();

    assert!(superseded_files(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn first_slice_supersedes_nothing() {
    let dir = tmp_dir("first_slice");
    let preserved = preserve_raw_bytes(&dir, "conv-1", b"only version\n").unwrap();

    assert_eq!(preserved.rel, "raw/conv-1.jsonl.zst");
    assert!(preserved.superseded.is_none());
    assert!(superseded_files(&dir).is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn identical_slice_is_not_superseded() {
    // Re-importing the same export must not pile up versions.
    let dir = tmp_dir("identical_slice");
    preserve_raw_bytes(&dir, "conv-1", b"same bytes\n").unwrap();
    let preserved = preserve_raw_bytes(&dir, "conv-1", b"same bytes\n").unwrap();

    assert!(preserved.superseded.is_none());
    assert!(superseded_files(&dir).is_empty());
    assert_eq!(read_raw(&dir, "conv-1").unwrap(), "same bytes\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_shrunken_slice_keeps_the_longer_previous_version() {
    // The case this whole mechanism exists for: the provider hands back a
    // trimmed conversation, and the fuller copy must survive it.
    let dir = tmp_dir("shrunken_slice");
    let full = "turn one\nturn two\nturn three\n";
    preserve_raw_bytes(&dir, "conv-1", full.as_bytes()).unwrap();

    let trimmed = "turn three\n";
    let preserved = preserve_raw_bytes(&dir, "conv-1", trimmed.as_bytes()).unwrap();

    // Current raw is the new (shorter) one, as the index will record.
    assert_eq!(read_raw(&dir, "conv-1").unwrap(), trimmed);
    // …and the fuller version is still on disk, at the reported path.
    let kept = preserved.superseded.expect("previous version kept");
    assert!(kept.starts_with(SUPERSEDED_DIR));
    assert_eq!(read_zst(&dir.join(&kept)), full);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_grown_slice_also_keeps_the_previous_version() {
    // Growth is not proof nothing was lost: a conversation can gain ten
    // messages and lose three in the same export, so every content change
    // is versioned rather than only the ones that shrink.
    let dir = tmp_dir("grown_slice");
    preserve_raw_bytes(&dir, "conv-1", b"a\nb\nc\n").unwrap();
    let preserved = preserve_raw_bytes(&dir, "conv-1", b"b\nc\nd\ne\nf\n").unwrap();

    let kept = preserved.superseded.expect("previous version kept");
    assert_eq!(read_zst(&dir.join(&kept)), "a\nb\nc\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn repeated_replacements_keep_every_version() {
    let dir = tmp_dir("version_history");
    preserve_raw_bytes(&dir, "conv-1", b"v1\n").unwrap();
    preserve_raw_bytes(&dir, "conv-1", b"v2\n").unwrap();
    preserve_raw_bytes(&dir, "conv-1", b"v3\n").unwrap();

    let kept: Vec<String> = superseded_files(&dir)
        .iter()
        .map(|path| read_zst(path))
        .collect();
    assert_eq!(kept.len(), 2);
    assert!(kept.contains(&"v1\n".to_string()));
    assert!(kept.contains(&"v2\n".to_string()));
    assert_eq!(read_raw(&dir, "conv-1").unwrap(), "v3\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_corrupt_stored_raw_is_kept_not_overwritten() {
    let dir = tmp_dir("corrupt_stored");
    std::fs::create_dir_all(dir.join(RAW_DIR)).unwrap();
    std::fs::write(dir.join(raw_rel_path("conv-1")), b"not zstd data").unwrap();

    let preserved = preserve_raw_bytes(&dir, "conv-1", b"good bytes\n").unwrap();

    let kept = preserved.superseded.expect("corrupt copy kept");
    assert_eq!(std::fs::read(dir.join(&kept)).unwrap(), b"not zstd data");
    assert_eq!(read_raw(&dir, "conv-1").unwrap(), "good bytes\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn superseded_copies_stay_inside_the_raw_directory() {
    // `read_raw_at` refuses to escape the archive dir, so a recorded
    // superseded path must remain a plain relative path under `raw/`.
    let dir = tmp_dir("superseded_path_shape");
    preserve_raw_bytes(&dir, "conv-1", b"v1\n").unwrap();
    let preserved = preserve_raw_bytes(&dir, "conv-1", b"v2\n").unwrap();

    let kept = preserved.superseded.unwrap();
    assert!(kept.starts_with("raw/"));
    assert!(!kept.contains(".."));
    assert_eq!(read_raw_at(&dir, &kept).unwrap(), "v1\n");

    let _ = std::fs::remove_dir_all(&dir);
}
