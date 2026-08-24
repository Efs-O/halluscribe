// HalluScribe - unit tests for the markdown archive writer.

use super::*;
use crate::archive::{
    archived_source_size, delete_sessions, is_archived, read_sessions, session_id,
};
use chrono::{Local, TimeZone};

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
        raw_path: None,
    }
}

fn sample_output() -> GemmaOutput {
    GemmaOutput {
        title: "Fix JWT bug".into(),
        summary: "The session fixed a JWT expiry issue.".into(),
        session_type: SessionType::Debugging,
        error_tags: vec!["JWT".into()],
        topic_tags: vec!["auth".into(), "Rust".into()],
        verbatim_highlights: vec![],
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
    let local_now = fixed_now().with_timezone(&Local);
    assert!(s.contains(&local_now.format("%Y-%m-%d").to_string()));
    assert!(s.contains(&local_now.format("%H-%M-%S").to_string()));
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
fn write_session_refuses_to_overwrite_a_corrupt_index() {
    let dir = tmp_dir("corrupt_index");
    fs::create_dir_all(&dir).unwrap();
    let index_path = dir.join("index.json");
    fs::write(&index_path, b"{ not valid json").unwrap();

    let source = Path::new("/fake/corrupt-index.jsonl");
    let error = match write_session(&dir, &sample_meta(source), &sample_output(), fixed_now()) {
        Ok(_) => panic!("a corrupt existing index must not be replaced"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("JSON error"));
    assert_eq!(fs::read(&index_path).unwrap(), b"{ not valid json");
    assert!(!dir.join("sessions").exists());
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
    assert!(content.contains(
        &fixed_now()
            .with_timezone(&Local)
            .format("%Y-%m-%d")
            .to_string()
    ));
}

#[test]
fn markdown_displays_local_dates_and_utc_index_timestamp() {
    let dir = tmp_dir("local_dates");
    let meta = sample_meta(Path::new("/fake/local-dates.jsonl"));
    let written = write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();
    let content = fs::read_to_string(written.path).unwrap();
    let expected = fixed_now()
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M %:z")
        .to_string();

    assert!(content.contains(&format!("**Date created:** {expected}")));
    assert!(content.contains(&format!("**Archived:** {expected}")));
    assert!(!content.contains("UTC"));

    let sessions = read_sessions(&dir);
    assert_eq!(sessions[0].session_timestamp, "2026-04-15T14:30:00+00:00");
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
        raw_path: None,
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
        verbatim_highlights: vec![],
    }
}

#[test]
fn highlights_section_is_omitted_when_there_are_none() {
    assert_eq!(highlights_section(&[]), "");
}

#[test]
fn highlights_section_renders_each_highlight() {
    let rendered = highlights_section(&[
        "| task | who wins |".to_string(),
        "gemma beats me".to_string(),
    ]);
    assert!(rendered.starts_with("## Highlights\n\n"));
    assert!(rendered.contains("| task | who wins |"));
    assert!(rendered.contains("gemma beats me"));
}

#[test]
fn written_markdown_carries_highlights_for_body_search() {
    // The whole point of the section: `search/content.rs::body_find` reads this
    // `.md`, so a verdict that lands here becomes findable by `search_sessions`.
    let mut output = sample_output();
    output.verbatim_highlights = vec!["| task | who wins | confidence |".to_string()];
    let meta = sample_meta(Path::new("/tmp/abc-123.jsonl"));
    let markdown = build_markdown(
        "T",
        &meta,
        &output,
        chrono::Utc::now().with_timezone(&Local),
    );
    assert!(markdown.contains("## Highlights"));
    assert!(markdown.contains("who wins"));
}

#[test]
fn highlights_written_by_the_writer_are_found_by_search_sessions() {
    // End-to-end proof of the retrieval fix. The 2026-08-04 failure was a
    // comparison board that existed in the archive but was invisible to
    // `search_sessions`, because the summariser dropped it and the body matcher
    // only ever reads the distilled `.md`. Verdicts now land in that `.md` via
    // verbatim_highlights, so the same query must find the session.
    let dir = tmp_dir("highlights_searchable");
    let src = Path::new("/fake/project/board-session.jsonl");
    let meta = sample_meta(src);
    let mut out = sample_output();
    // Deliberately a phrase that appears in NEITHER the title nor the tags, so a
    // pass can only come from the body matcher reading the Highlights section.
    out.verbatim_highlights = vec!["| task | who wins | confidence |".to_string()];

    write_session(&dir, &meta, &out, fixed_now()).unwrap();

    let params = crate::search::SearchParams {
        query: Some("who wins".to_string()),
        ..Default::default()
    };
    let hits = crate::search::search_sessions(&dir, &params);
    assert_eq!(
        hits.len(),
        1,
        "board phrase should match exactly one session"
    );
    assert_eq!(hits[0].id, meta.id);
}

#[test]
fn highlights_make_the_board_findable_by_the_users_paraphrase() {
    // The 2026-08-04 question was "where gemma wins and where not". The board's
    // heading reads "| task | who wins |" — the words "gemma" and "wins" are
    // never adjacent in it, so `search_raw_transcripts` (literal substring)
    // cannot find it from that phrasing, which is exactly how the live failure
    // happened. `search_sessions` is tokenized and AND-of-keywords, so once the
    // board is in the .md body the paraphrase matches. This is the test that
    // shows WHY putting highlights in the summary is the fix.
    let dir = tmp_dir("paraphrase_finds_board");
    let src = Path::new("/fake/project/board-paraphrase.jsonl");
    let meta = sample_meta(src);
    let mut out = sample_output();
    out.verbatim_highlights = vec!["| task | who wins | confidence |\n\
         | Reading one frame accurately | **Gemma at 1120** | measured, but thin |\n\
         | Cost and repeatability | **Gemma, by orders of magnitude** | obvious |"
        .to_string()];
    write_session(&dir, &meta, &out, fixed_now()).unwrap();

    let find = |query: &str| {
        crate::search::search_sessions(
            &dir,
            &crate::search::SearchParams {
                query: Some(query.to_string()),
                ..Default::default()
            },
        )
    };

    // The user's actual 2026-08-04 phrasing, plus the wordings around it.
    assert_eq!(
        find("where gemma wins").len(),
        1,
        "user's paraphrase must match"
    );
    assert_eq!(find("gemma wins").len(), 1, "short paraphrase must match");
    assert_eq!(find("who wins").len(), 1, "literal heading must match");
    assert_eq!(
        find("gemma orders of magnitude").len(),
        1,
        "verdict text must match"
    );
}
