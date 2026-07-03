// HalluScribe - scope-specific end-to-end tests for the profile refresh
// orchestration (Persona Protocol Phase 2c: Work vs Personal). Split out of
// refresh_tests.rs to keep both files under the repo's 350-LOC file cap.

use super::*;
use std::fs;
use std::path::PathBuf;

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("halluscribe_profile_refresh_scope_{name}"));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn write_index(dir: &std::path::Path, sessions: &[serde_json::Value]) {
    let index = serde_json::json!({ "sessions": sessions });
    fs::write(
        dir.join("index.json"),
        serde_json::to_string_pretty(&index).unwrap(),
    )
    .unwrap();
}

fn session_json(id: &str, provider: &str, ts: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "project": "proj",
        "date": &ts[..10],
        "title": format!("Session {id}"),
        "tool": "Claude Code",
        "fill_pct": 90.0,
        "session_timestamp": ts,
        "updated_at": "",
        "session_type": "building",
        "error_tags": [],
        "topic_tags": [],
        "archive_path": format!("{id}.md"),
        "source_jsonl": "",
        "source_size_bytes": 0,
        "provider": provider,
        "fill_estimated": false,
        "transcript_hash": ""
    })
}

fn canned_response(tool: &serde_json::Value) -> serde_json::Value {
    match tool["function"]["name"].as_str().unwrap_or("") {
        "save_profile_facts" => serde_json::json!({
            "facts": [{
                "section": "projects",
                "fact": "Works on proj.",
                "evidence": ["s1"],
                "date": "2026-06-01"
            }]
        }),
        _ => serde_json::json!({
            "identity": "Dev.",
            "projects": "proj [s1]",
            "conventions": "",
            "recurring_problems": "",
            "communication_style": "",
            "timeline": "",
            "personal_context": ""
        }),
    }
}

#[test]
fn pending_session_count_respects_watermark_and_consent() {
    let dir = tmp_dir("pending_count");
    fs::write(dir.join("s1.md"), "body one").unwrap();
    write_index(
        &dir,
        &[
            session_json("s1", "claude_code", "2026-06-10T00:00:00+00:00"),
            session_json("x1", "chatgpt", "2026-06-20T00:00:00+00:00"),
        ],
    );
    let sources = vec!["claude_code".to_string()];
    assert_eq!(
        pending_session_count(&dir, ProfileScope::Work, &sources, false),
        1
    );

    let tool_call: &ToolCallFn = &|_sys, _user, tool, _max| Ok(canned_response(tool));
    run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        true,
        tool_call,
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(
        pending_session_count(&dir, ProfileScope::Work, &sources, false),
        0
    );
    assert_eq!(
        pending_session_count(&dir, ProfileScope::Work, &sources, true),
        1
    );
}

#[test]
fn personal_scope_includes_chat_exports_even_when_not_in_settings() {
    let dir = tmp_dir("personal_scope");
    fs::write(dir.join("s1.md"), "body one").unwrap();
    fs::write(dir.join("x1.md"), "chat export body").unwrap();
    write_index(
        &dir,
        &[
            session_json("s1", "claude_code", "2026-06-10T00:00:00+00:00"),
            session_json("x1", "chatgpt", "2026-06-20T00:00:00+00:00"),
        ],
    );
    // The user's settings only consent to claude_code; Personal scope still
    // folds in chatgpt/claude_ai/gemini per the Phase 2c consent rule.
    let sources = vec!["claude_code".to_string()];
    let tool_call: &ToolCallFn = &|_sys, _user, tool, _max| Ok(canned_response(tool));

    let outcome = run_refresh(
        &dir,
        ProfileScope::Personal,
        &sources,
        true,
        tool_call,
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(outcome.session_count, 2);

    let meta = load_meta(&dir, ProfileScope::Personal);
    assert!(meta.sources.contains(&"claude_code".to_string()));
    assert!(meta.sources.contains(&"chatgpt".to_string()));

    // Work scope is untouched by the Personal-scope run.
    assert!(read_profile_md(&dir, ProfileScope::Work).is_none());
    assert!(read_profile_md(&dir, ProfileScope::Personal).is_some());
}

#[test]
fn personal_scope_pending_count_and_watermark_are_independent_of_work() {
    let dir = tmp_dir("independent_watermarks");
    fs::write(dir.join("s1.md"), "body one").unwrap();
    write_index(
        &dir,
        &[session_json(
            "s1",
            "claude_code",
            "2026-06-10T00:00:00+00:00",
        )],
    );
    let sources = vec!["claude_code".to_string()];
    let tool_call: &ToolCallFn = &|_sys, _user, tool, _max| Ok(canned_response(tool));

    // Fully refresh Work only.
    run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        true,
        tool_call,
        |_, _, _| {},
    )
    .unwrap();

    // Personal has its own watermark: still has one pending session, since
    // its own scope was never refreshed.
    assert_eq!(
        pending_session_count(&dir, ProfileScope::Personal, &sources, false),
        1
    );
    assert_eq!(
        pending_session_count(&dir, ProfileScope::Work, &sources, false),
        0
    );
}
