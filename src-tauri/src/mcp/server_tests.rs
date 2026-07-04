// HalluScribe - tests for the MCP tool-backing logic. Exercises the plain
// `do_*` handler methods directly (see server.rs) against a small temp
// archive shaped like the fixtures in search::tests and profile::writer.

use super::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn tmp() -> TempDir {
    tempfile::tempdir().unwrap()
}

fn make_index(dir: &Path, entries: &[(&str, &str, &str, &str)]) {
    let sessions: Vec<serde_json::Value> = entries
        .iter()
        .map(|(id, title, date, tool)| {
            serde_json::json!({
                "id": id,
                "project": "proj",
                "date": date,
                "title": title,
                "tool": tool,
                "fill_pct": 60.0,
                "session_type": "debugging",
                "error_tags": ["jwt"],
                "topic_tags": ["rust"],
                "archive_path": format!("sessions/proj/{date}/12-00-00-claudecode-sweep.md"),
                "source_jsonl": "/fake/path.jsonl"
            })
        })
        .collect();
    let idx = serde_json::json!({ "sessions": sessions });
    fs::write(dir.join("index.json"), serde_json::to_string(&idx).unwrap()).unwrap();
}

fn write_session_md(dir: &Path, date: &str, body: &str) {
    let md_path = dir
        .join("sessions/proj")
        .join(date)
        .join("12-00-00-claudecode-sweep.md");
    fs::create_dir_all(md_path.parent().unwrap()).unwrap();
    fs::write(&md_path, body).unwrap();
}

#[test]
fn search_sessions_returns_json_page_with_honest_counts() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[
            ("a", "Auth fix", "2026-04-15", "Claude Code"),
            ("b", "Codex run", "2026-04-14", "Codex"),
        ],
    );
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    let json = server
        .do_search_sessions(SearchSessionsRequest {
            query: Some("auth".into()),
            date_from: None,
            date_to: None,
            tags: None,
            project: None,
            tool: None,
            limit: None,
            offset: None,
        })
        .unwrap();
    let page: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(page["searched"], 2);
    assert_eq!(page["total_matches"], 1);
    assert_eq!(page["returned"], 1);
    assert_eq!(page["results"][0]["id"], "a");
}

#[test]
fn read_session_returns_body_for_known_id() {
    let dir = tmp();
    make_index(dir.path(), &[("abc", "Title", "2026-04-15", "Claude Code")]);
    write_session_md(dir.path(), "2026-04-15", "# Title\n\nContent here.");
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    let content = server.do_read_session("abc").unwrap();
    assert!(content.contains("Content here."));
}

#[test]
fn read_session_errors_cleanly_for_unknown_id() {
    let dir = tmp();
    make_index(dir.path(), &[]);
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    let error = server.do_read_session("ghost-id").unwrap_err();
    // A proper MCP tool error, not a panic.
    assert!(!error.message.is_empty());
}

#[test]
fn get_profile_returns_placeholder_when_absent() {
    let dir = tmp();
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    assert_eq!(server.do_get_profile(), "No profile has been built yet.");
}

#[test]
fn get_profile_returns_work_scope_markdown_when_present() {
    let dir = tmp();
    let work_dir = dir.path().join("profile").join("work");
    fs::create_dir_all(&work_dir).unwrap();
    fs::write(work_dir.join("profile.md"), "# User Profile\n\nHello.").unwrap();
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    assert!(server.do_get_profile().contains("Hello."));
}

#[test]
fn get_profile_never_returns_personal_scope_content() {
    let dir = tmp();
    let work_dir = dir.path().join("profile").join("work");
    let personal_dir = dir.path().join("profile").join("personal");
    fs::create_dir_all(&work_dir).unwrap();
    fs::create_dir_all(&personal_dir).unwrap();
    fs::write(work_dir.join("profile.md"), "# Work profile").unwrap();
    fs::write(
        personal_dir.join("profile.md"),
        "# Personal profile - secret",
    )
    .unwrap();
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    let content = server.do_get_profile();
    assert!(content.contains("Work profile"));
    assert!(!content.contains("secret"));
}

#[test]
fn get_digest_returns_placeholder_when_absent() {
    let dir = tmp();
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    assert_eq!(server.do_get_digest(), "No digest has been generated yet.");
}

#[test]
fn get_digest_returns_latest_work_digest_when_present() {
    let dir = tmp();
    let work_dir = dir.path().join("profile").join("work");
    fs::create_dir_all(&work_dir).unwrap();
    fs::write(work_dir.join("digest-2026-W10.md"), "old week").unwrap();
    fs::write(work_dir.join("digest-2026-W12.md"), "newest week").unwrap();
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    assert_eq!(server.do_get_digest(), "newest week");
}
