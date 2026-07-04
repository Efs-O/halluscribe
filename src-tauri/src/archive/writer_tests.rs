// HalluScribe - unit tests for the markdown archive writer.

use super::*;
use crate::archive::{
    archived_source_size, delete_sessions, is_archived, read_sessions, session_id,
};
use chrono::TimeZone;

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 15, 14, 30, 0).unwrap()
}

fn sample_meta(source: &Path) -> SessionMeta {
    SessionMeta {
        id: session_id(source),
        source: source.to_path_buf(),
        project: "my-project".into(),
        tool: "Claude Code".into(),
        provider: "claude_code".into(),
        fill_pct: 78.5,
        fill_estimated: false,
        backend: "llama.cpp".into(),
        session_timestamp: fixed_now(),
        updated_at: None,
        transcript_hash: "abc123".into(),
    }
}

fn sample_output() -> GemmaOutput {
    GemmaOutput {
        title: "Fix JWT bug".into(),
        summary: "The session fixed a JWT expiry issue.".into(),
        session_type: SessionType::Debugging,
        error_tags: vec!["JWT".into()],
        topic_tags: vec!["auth".into(), "Rust".into()],
    }
}

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("halluscribe_test_{name}"));
    let _ = fs::remove_dir_all(&d);
    d
}

#[test]
fn session_id_uses_file_stem() {
    let p = Path::new("/foo/bar/abc-123-def.jsonl");
    assert_eq!(crate::archive::session_id(p), "abc-123-def");
}

#[test]
fn is_archived_false_when_no_index() {
    let dir = tmp_dir("no_index");
    assert!(!is_archived(&dir, "some-id"));
}

#[test]
fn write_session_creates_markdown_file() {
    let dir = tmp_dir("write_md");
    let src = Path::new("/fake/project/d4632e2c-dead-beef.jsonl");
    let meta = sample_meta(src);
    let out = sample_output();

    let written = write_session(&dir, &meta, &out, fixed_now()).unwrap();
    assert!(written.path.exists());

    let content = fs::read_to_string(&written.path).unwrap();
    assert!(content.contains("# Fix JWT bug"));
    assert!(content.contains("**Tool:** Claude Code"));
    assert!(content.contains("**Provider:** claude_code"));
    assert!(content.contains("78.5%"));
    assert!(content.contains("llama.cpp"));
    assert!(content.contains("The session fixed a JWT expiry issue."));
}

#[test]
fn write_session_path_uses_date_and_tool_slug() {
    let dir = tmp_dir("path_check");
    let src = Path::new("/fake/d4632e2c-dead-beef.jsonl");
    let meta = sample_meta(src);
    let written = write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();
    let s = written.path.to_string_lossy();
    assert!(s.contains("2026-04-15"));
    assert!(s.contains("14-30-00"));
    assert!(s.contains("claudecode"));
    assert!(s.contains("sweep"));
}

#[test]
fn write_session_updates_index() {
    let dir = tmp_dir("index_check");
    let src = Path::new("/fake/abc-uuid.jsonl");
    let meta = sample_meta(src);
    write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();

    let sessions = read_sessions(&dir);
    assert_eq!(sessions.len(), 1);
    let e = &sessions[0];
    assert_eq!(e.id, "abc-uuid");
    assert_eq!(e.title, "Fix JWT bug");
    assert_eq!(e.session_type, "debugging");
    assert_eq!(e.error_tags, vec!["JWT"]);
    assert_eq!(e.tool, "Claude Code");
    assert_eq!(e.session_timestamp, "2026-04-15T14:30:00+00:00");
}

#[test]
fn write_session_is_idempotent() {
    let dir = tmp_dir("idempotent");
    let src = Path::new("/fake/abc-uuid.jsonl");
    let meta = sample_meta(src);
    write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();
    write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();

    let sessions = read_sessions(&dir);
    assert_eq!(sessions.len(), 1);
}

#[test]
fn is_archived_true_after_write() {
    let dir = tmp_dir("is_archived");
    let src = Path::new("/fake/my-session-id.jsonl");
    let meta = sample_meta(src);
    assert!(!is_archived(&dir, "my-session-id"));
    write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();
    assert!(is_archived(&dir, "my-session-id"));
}

#[test]
fn fallback_title_when_output_title_empty() {
    let dir = tmp_dir("fallback_title");
    let src = Path::new("/fake/uuid.jsonl");
    let meta = sample_meta(src);
    let mut out = sample_output();
    out.title = String::new();
    let written = write_session(&dir, &meta, &out, fixed_now()).unwrap();
    let content = fs::read_to_string(written.path).unwrap();
    assert!(content.contains("my-project"));
    assert!(content.contains("2026-04-15"));
}

#[test]
fn tool_slug_variants() {
    assert_eq!(tool_slug("Claude Code"), "claudecode");
    assert_eq!(tool_slug("Codex"), "codex");
    assert_eq!(tool_slug("Forge"), "forge");
    assert_eq!(tool_slug("Gemma 4"), "gemma4");
    assert_eq!(tool_slug("Continue"), "continue");
    assert_eq!(tool_slug("Unknown"), "continue");
}

#[test]
fn session_type_str_all_variants() {
    assert_eq!(session_type_str(&SessionType::Debugging), "debugging");
    assert_eq!(session_type_str(&SessionType::Building), "building");
    assert_eq!(session_type_str(&SessionType::Refactoring), "refactoring");
    assert_eq!(session_type_str(&SessionType::Exploration), "exploration");
}

#[test]
fn delete_sessions_removes_file_and_index_entry() {
    let dir = tmp_dir("delete_sessions");
    let src = Path::new("/fake/to-delete.jsonl");
    let meta = sample_meta(src);
    let written = write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();
    assert!(written.path.exists());
    let deleted = delete_sessions(&dir, &["to-delete".to_string()]).unwrap();
    assert_eq!(deleted, vec!["to-delete"]);
    assert!(!written.path.exists());
    assert!(!is_archived(&dir, "to-delete"));
}

#[test]
fn delete_sessions_ignores_unknown_ids() {
    let dir = tmp_dir("delete_unknown");
    let deleted = delete_sessions(&dir, &["nonexistent".to_string()]).unwrap();
    assert!(deleted.is_empty());
}

#[test]
fn archived_source_size_none_when_absent() {
    let dir = tmp_dir("arc_size_absent");
    assert!(archived_source_size(&dir, "no-such-id").is_none());
}

#[test]
fn archived_source_size_stored_and_retrieved() {
    let dir = tmp_dir("arc_size_stored");
    fs::create_dir_all(&dir).unwrap();
    let src = dir.join("session.jsonl");
    fs::write(&src, b"hello world").unwrap();
    let meta = SessionMeta {
        id: session_id(&src),
        source: src.clone(),
        project: "p".into(),
        tool: "Claude Code".into(),
        provider: "claude_code".into(),
        fill_pct: 50.0,
        fill_estimated: false,
        backend: "Ollama".into(),
        session_timestamp: fixed_now(),
        updated_at: None,
        transcript_hash: "hash".into(),
    };
    write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();
    let stored = archived_source_size(&dir, "session");
    assert!(stored.is_some());
    assert_eq!(stored.unwrap(), 11);
}

// -- secret flags (Phase 0b) --

#[test]
fn write_session_flags_aws_access_key_in_summary() {
    let dir = tmp_dir("secret_flags_aws");
    let src = Path::new("/fake/secret-session.jsonl");
    let meta = sample_meta(src);
    let out = output_with_summary("found a stray key AKIAIOSFODNN7EXAMPLE in the .env file");

    let written = write_session(&dir, &meta, &out, fixed_now()).unwrap();
    assert_eq!(written.secret_flags, vec!["aws_access_key".to_string()]);

    let sessions = read_sessions(&dir);
    assert_eq!(sessions[0].secret_flags, vec!["aws_access_key".to_string()]);
}

#[test]
fn write_session_clean_summary_has_no_secret_flags() {
    let dir = tmp_dir("secret_flags_clean");
    let src = Path::new("/fake/clean-session.jsonl");
    let meta = sample_meta(src);
    let out = output_with_summary("refactored the parser module, no issues found");

    let written = write_session(&dir, &meta, &out, fixed_now()).unwrap();
    assert!(written.secret_flags.is_empty());

    let sessions = read_sessions(&dir);
    assert!(sessions[0].secret_flags.is_empty());
}

fn output_with_summary(summary: &str) -> GemmaOutput {
    GemmaOutput {
        title: "Test session".into(),
        summary: summary.to_string(),
        session_type: SessionType::Debugging,
        error_tags: vec![],
        topic_tags: vec![],
    }
}
