// HalluScribe - tests for permanent-delete records and how lookups honour them.

use super::{
    delete_sessions, ensure_deleted_readable, load_deleted, read_sessions, write_session,
    SessionLookup, SessionMeta, DELETED_SESSIONS_FILE,
};
use crate::gemma::{GemmaOutput, SessionType};
use chrono::{TimeZone, Utc};
use std::fs;
use std::path::{Path, PathBuf};

fn tmp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "halluscribe_tombstone_test_{}_{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn archive(dir: &Path, id: &str, source: &str) {
    let meta = SessionMeta {
        id: id.into(),
        source: PathBuf::from(source),
        project: "proj".into(),
        tool: "Claude Code".into(),
        provider: "claude_code".into(),
        fill_pct: 50.0,
        fill_estimated: false,
        output_tokens: 0,
        tokens_estimated: true,
        backend: "llama.cpp".into(),
        model: "model.gguf".into(),
        session_timestamp: Utc.with_ymd_and_hms(2026, 4, 15, 9, 0, 0).unwrap(),
        updated_at: None,
        transcript_hash: "hash".into(),
        raw_path: None,
    };
    let output = GemmaOutput {
        title: "Title".into(),
        summary: "Summary.".into(),
        session_type: SessionType::Building,
        error_tags: vec![],
        topic_tags: vec![],
        verbatim_highlights: vec![],
    };
    write_session(dir, &meta, &output, Utc::now()).unwrap();
}

#[test]
fn no_record_file_means_nothing_was_deleted() {
    let dir = tmp_dir("missing");
    assert!(load_deleted(&dir).unwrap().is_empty());
    assert!(ensure_deleted_readable(&dir).is_ok());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_deleted_session_stays_deleted_for_its_own_source_only() {
    let dir = tmp_dir("recorded");
    archive(&dir, "abc", "/fake/abc.jsonl");

    delete_sessions(&dir, &["abc".to_string()]).unwrap();

    let deleted = load_deleted(&dir).unwrap();
    assert_eq!(deleted["abc"].source_path, "/fake/abc.jsonl");
    let lookup = SessionLookup::load(&dir);
    assert!(lookup.is_deleted("abc", Path::new("/fake/abc.jsonl")));
    assert!(!lookup.is_deleted("abc", Path::new("/other/abc.jsonl")));
    // The original source resolves to the deleted id; a different source that
    // proposes the same id gets its own, so it is not swallowed by the delete.
    assert_eq!(
        lookup.resolve_session_id(Path::new("/fake/abc.jsonl"), "abc"),
        "abc"
    );
    assert_ne!(
        lookup.resolve_session_id(Path::new("/other/abc.jsonl"), "abc"),
        "abc"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_corrupt_record_file_blocks_deletes_instead_of_losing_them() {
    let dir = tmp_dir("corrupt");
    archive(&dir, "abc", "/fake/abc.jsonl");
    fs::write(dir.join(DELETED_SESSIONS_FILE), "{not json").unwrap();

    assert!(ensure_deleted_readable(&dir).is_err());
    assert!(delete_sessions(&dir, &["abc".to_string()]).is_err());
    assert_eq!(read_sessions(&dir).len(), 1, "nothing may be deleted");
    let _ = fs::remove_dir_all(&dir);
}
