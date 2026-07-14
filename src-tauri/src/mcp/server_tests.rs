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
fn search_sessions_multiword_query_finds_forge_bridge_session() {
    // Mirrors SEARCH_TOKENIZATION_PLAN.md's synthetic c47cc0fa-shaped entry:
    // the title splits "MCP" and "Bridge" as separate words, and only the
    // body contains the contiguous "mcpBridge" identifier / word "client".
    // A pre-tokenization matcher (plain substring) finds zero hits for this
    // query; the AND-of-tokens matcher must find it.
    let dir = tmp();
    make_index(
        dir.path(),
        &[(
            "c47cc0fa",
            "Forge MCP Bridge Implementation and HalluScribe Sync",
            "2026-07-06",
            "Forge",
        )],
    );
    write_session_md(
        dir.path(),
        "2026-07-06",
        "Implemented Forge's MCP client integration; the bridge (mcpBridge.ts) now connects to \
         HalluScribe's read-only tools. Part B implemented and released as Forge v0.12.27.",
    );
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    let json = server
        .do_search_sessions(SearchSessionsRequest {
            query: Some("Forge MCP client bridge".into()),
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
    assert_eq!(page["total_matches"], 1);
    assert_eq!(page["results"][0]["id"], "c47cc0fa");
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

/// Build an index whose entries carry a `raw_path`, so `search_raw` has raws
/// to scan. Entries: (id, date, raw_path relative to the archive dir).
fn make_raw_index(dir: &Path, entries: &[(&str, &str, &str)]) {
    let sessions: Vec<serde_json::Value> = entries
        .iter()
        .map(|(id, date, raw_path)| {
            serde_json::json!({
                "id": id,
                "project": "proj",
                "date": date,
                "title": format!("Session {id}"),
                "tool": "codex",
                "fill_pct": 42.0,
                "session_type": "coding",
                "error_tags": [],
                "topic_tags": ["fixture"],
                "archive_path": format!("sessions/{id}.md"),
                "source_jsonl": format!("sources/{id}.jsonl"),
                "raw_path": raw_path,
            })
        })
        .collect();
    let idx = serde_json::json!({ "sessions": sessions });
    fs::write(dir.join("index.json"), serde_json::to_string(&idx).unwrap()).unwrap();
}

/// Write a zstd-compressed raw transcript where `read_raw_at` expects it.
fn write_raw(dir: &Path, rel_path: &str, fixture: &str) {
    let compressed = zstd::encode_all(fixture.as_bytes(), 3).unwrap();
    let path = dir.join(rel_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, compressed).unwrap();
}

#[test]
fn search_raw_transcripts_groups_newest_first_and_honors_limit() {
    let dir = tmp();
    make_raw_index(
        dir.path(),
        &[
            ("older", "2026-01-01", "raw/older.jsonl.zst"),
            ("newer", "2026-07-01", "raw/newer.jsonl.zst"),
        ],
    );
    write_raw(
        dir.path(),
        "raw/older.jsonl.zst",
        "the widget failed to load\n",
    );
    write_raw(
        dir.path(),
        "raw/newer.jsonl.zst",
        "widget rendered\nwidget clicked\n",
    );
    // Default settings preserve raws, so no settings.json is needed here.
    let server = HalluscribeServer::new(dir.path().to_path_buf());

    let json = server
        .do_search_raw_transcripts(SearchRawTranscriptsRequest {
            query: "widget".into(),
            limit: None,
        })
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(result["total_hits"], 3);
    assert_eq!(result["sessions"][0]["session_id"], "newer"); // newest first
    assert_eq!(result["sessions"][0]["total_hits"], 2);
    assert_eq!(result["sessions"][1]["session_id"], "older");
    assert_eq!(result["results_truncated"], false);

    // limit drops the older group but total_hits stays honest across the archive.
    let json = server
        .do_search_raw_transcripts(SearchRawTranscriptsRequest {
            query: "widget".into(),
            limit: Some(1),
        })
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(result["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(result["sessions"][0]["session_id"], "newer");
    assert_eq!(result["results_truncated"], true);
    assert_eq!(result["total_hits"], 3);
}

#[test]
fn search_raw_transcripts_refuses_when_preserve_disabled() {
    let dir = tmp();
    make_raw_index(dir.path(), &[("s", "2026-07-01", "raw/s.jsonl.zst")]);
    write_raw(dir.path(), "raw/s.jsonl.zst", "widget\n");
    let settings = crate::settings::HalluScribeSettings {
        preserve_raw_transcripts: false,
        ..Default::default()
    };
    crate::settings::save_settings(dir.path(), &settings).unwrap();
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    let error = server
        .do_search_raw_transcripts(SearchRawTranscriptsRequest {
            query: "widget".into(),
            limit: None,
        })
        .unwrap_err();
    assert!(error.message.contains("Preserve raw transcripts"));
}

#[test]
fn get_profile_returns_placeholder_when_absent() {
    let dir = tmp();
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    assert_eq!(
        server.do_get_profile(ProfileScope::Work),
        "No profile has been built yet."
    );
}

#[test]
fn get_profile_returns_work_scope_markdown_when_present() {
    let dir = tmp();
    let work_dir = dir.path().join("profile").join("work");
    fs::create_dir_all(&work_dir).unwrap();
    fs::write(work_dir.join("profile.md"), "# User Profile\n\nHello.").unwrap();
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    assert!(server.do_get_profile(ProfileScope::Work).contains("Hello."));
}

#[test]
fn get_profile_returns_personal_scope_when_requested() {
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

    let work_content = server.do_get_profile(ProfileScope::Work);
    assert!(work_content.contains("Work profile"));
    assert!(!work_content.contains("secret"));

    let personal_content = server.do_get_profile(ProfileScope::Personal);
    assert!(personal_content.contains("secret"));
}

#[test]
fn get_digest_returns_placeholder_when_absent() {
    let dir = tmp();
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    assert_eq!(
        server.do_get_digest(ProfileScope::Work, 0, None),
        "No digest has been generated yet."
    );
}

#[test]
fn get_digest_returns_latest_work_digest_when_present() {
    let dir = tmp();
    let work_dir = dir.path().join("profile").join("work");
    fs::create_dir_all(&work_dir).unwrap();
    fs::write(work_dir.join("digest-2026-W10.md"), "old week").unwrap();
    fs::write(work_dir.join("digest-2026-W12.md"), "newest week").unwrap();
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    // Small digest, default call: verbatim, no slice header (back-compat).
    assert_eq!(
        server.do_get_digest(ProfileScope::Work, 0, None),
        "newest week"
    );
}

#[test]
fn get_digest_returns_personal_scope_when_requested() {
    let dir = tmp();
    let personal_dir = dir.path().join("profile").join("personal");
    fs::create_dir_all(&personal_dir).unwrap();
    fs::write(
        personal_dir.join("digest-2026-W12.md"),
        "personal newest week",
    )
    .unwrap();
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    assert_eq!(
        server.do_get_digest(ProfileScope::Personal, 0, None),
        "personal newest week"
    );
}

/// Writes a Work digest of exactly `content` and returns a server over it.
fn server_with_work_digest(dir: &Path, content: &str) -> HalluscribeServer {
    let work_dir = dir.join("profile").join("work");
    fs::create_dir_all(&work_dir).unwrap();
    fs::write(work_dir.join("digest-2026-W12.md"), content).unwrap();
    HalluscribeServer::new(dir.to_path_buf())
}

#[test]
fn get_digest_pages_oversized_digest_and_names_continuation_offset() {
    let dir = tmp();
    let big = "x".repeat(25);
    let server = server_with_work_digest(dir.path(), &big);

    let first = server.do_get_digest(ProfileScope::Work, 0, Some(10));
    assert!(first.starts_with("[digest slice bytes 0..10 of 25; continue with offset=10]\n"));
    assert!(first.ends_with(&"x".repeat(10)));

    let last = server.do_get_digest(ProfileScope::Work, 20, Some(10));
    assert!(last.starts_with("[digest slice bytes 20..25 of 25; end of digest]\n"));
    assert!(last.ends_with(&"x".repeat(5)));
}

#[test]
fn get_digest_slices_on_utf8_boundaries() {
    let dir = tmp();
    // "αβγδε" — every char is 2 bytes, so byte index 5 is mid-character.
    let server = server_with_work_digest(dir.path(), "αβγδε");

    let sliced = server.do_get_digest(ProfileScope::Work, 0, Some(5));
    // max_chars=5 snaps down to the 4-byte boundary: two whole chars, no panic.
    assert!(sliced.contains("αβ"));
    assert!(!sliced.contains('γ'));

    let offset_snapped = server.do_get_digest(ProfileScope::Work, 3, Some(50));
    // offset=3 snaps down to 2, so the slice starts at a whole char.
    assert!(offset_snapped.contains("βγδε"));
}

#[test]
fn get_digest_offset_past_end_returns_empty_slice_not_panic() {
    let dir = tmp();
    let server = server_with_work_digest(dir.path(), "short");
    let out = server.do_get_digest(ProfileScope::Work, 999, Some(10));
    assert!(out.starts_with("[digest slice bytes 5..5 of 5; end of digest]"));
}

#[test]
fn get_profile_tool_stamps_archive_identity_and_scope() {
    let dir = tmp();
    let work_dir = dir.path().join("profile").join("work");
    fs::create_dir_all(&work_dir).unwrap();
    fs::write(work_dir.join("profile.md"), "# User Profile").unwrap();
    let server = HalluscribeServer::new(dir.path().to_path_buf());
    let out = server
        .get_profile(Parameters(ScopeRequest { scope: None }))
        .unwrap();
    assert!(out.starts_with("[HalluScribe archive: "));
    assert!(out.contains("scope: work]"));
    assert!(out.contains("# User Profile"));
}

#[test]
fn get_digest_tool_stamps_first_page_only() {
    let dir = tmp();
    let server = server_with_work_digest(dir.path(), &"x".repeat(25));

    let first = server
        .get_digest(Parameters(DigestRequest {
            scope: None,
            offset: None,
            max_chars: Some(10),
        }))
        .unwrap();
    assert!(first.starts_with("[HalluScribe archive: "));
    assert!(first.contains("scope: work]"));

    let continuation = server
        .get_digest(Parameters(DigestRequest {
            scope: None,
            offset: Some(10),
            max_chars: Some(10),
        }))
        .unwrap();
    assert!(!continuation.contains("[HalluScribe archive: "));
    assert!(continuation.starts_with("[digest slice bytes 10..20 of 25"));
}

#[test]
fn scope_or_work_defaults_and_falls_back_to_work() {
    assert_eq!(scope_or_work(None), ProfileScope::Work);
    assert_eq!(
        scope_or_work(Some("nonsense".to_string())),
        ProfileScope::Work
    );
    assert_eq!(
        scope_or_work(Some("personal".to_string())),
        ProfileScope::Personal
    );
}
