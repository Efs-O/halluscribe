use super::*;
use std::{fs, path::Path};
use tempfile::TempDir;

fn tmp() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// Write a minimal index.json from a slice of (id, title, date, tool) tuples.
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

#[test]
fn no_filters_returns_all_sorted_newest_first() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[
            ("a", "Auth fix", "2026-04-15", "Claude Code"),
            ("b", "Codex run", "2026-04-14", "Codex"),
        ],
    );
    let results = search_sessions(dir.path(), &SearchParams::default());
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].id, "a");
    assert_eq!(results[1].id, "b");
}

#[test]
fn query_matches_title() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[
            ("a", "Auth refactor", "2026-04-15", "Claude Code"),
            ("b", "Codex session", "2026-04-14", "Codex"),
        ],
    );
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            query: Some("auth".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "a");
}

#[test]
fn query_matches_tags() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[("a", "Something", "2026-04-15", "Claude Code")],
    );
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            query: Some("jwt".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
}

#[test]
fn date_from_filters_old_entries() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[
            ("a", "A", "2026-04-15", "Claude Code"),
            ("b", "B", "2026-04-10", "Claude Code"),
        ],
    );
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            date_from: Some("2026-04-12".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "a");
}

#[test]
fn date_to_filters_new_entries() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[
            ("a", "A", "2026-04-15", "Claude Code"),
            ("b", "B", "2026-04-10", "Claude Code"),
        ],
    );
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            date_to: Some("2026-04-12".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "b");
}

#[test]
fn tool_filter_claude_code() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[
            ("a", "A", "2026-04-15", "Claude Code"),
            ("b", "B", "2026-04-14", "Codex"),
        ],
    );
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            tool: Some("claude_code".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "a");
}

#[test]
fn tool_filter_codex() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[
            ("a", "A", "2026-04-15", "Claude Code"),
            ("b", "B", "2026-04-14", "Codex"),
        ],
    );
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            tool: Some("codex".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "b");
}

#[test]
fn tool_filter_forge() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[
            ("a", "A", "2026-04-15", "Claude Code"),
            ("b", "B", "2026-04-14", "Forge"),
        ],
    );
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            tool: Some("forge".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "b");
}

#[test]
fn project_filter_is_case_insensitive_substring() {
    let dir = tmp();
    make_index(dir.path(), &[("a", "A", "2026-04-15", "Claude Code")]);
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            project: Some("PRO".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
}

#[test]
fn tags_filter_requires_all_to_match() {
    let dir = tmp();
    make_index(dir.path(), &[("a", "A", "2026-04-15", "Claude Code")]);
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            tags: Some(vec!["jwt".into(), "rust".into()]),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);

    let no_match = search_sessions(
        dir.path(),
        &SearchParams {
            tags: Some(vec!["borrow-checker".into()]),
            ..Default::default()
        },
    );
    assert_eq!(no_match.len(), 0);
}

#[test]
fn limit_is_respected() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[
            ("a", "A", "2026-04-15", "Claude Code"),
            ("b", "B", "2026-04-14", "Claude Code"),
            ("c", "C", "2026-04-13", "Claude Code"),
        ],
    );
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            limit: Some(2),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 2);
}

#[test]
fn limit_capped_at_20() {
    let dir = tmp();
    let entries: Vec<_> = (0u8..25)
        .map(|i| {
            (
                Box::leak(i.to_string().into_boxed_str()) as &str,
                "T",
                "2026-04-15",
                "Claude Code",
            )
        })
        .collect();
    make_index(dir.path(), &entries);
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            limit: Some(100),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 20);
}

#[test]
fn empty_index_returns_empty_vec() {
    let dir = tmp();
    let results = search_sessions(dir.path(), &SearchParams::default());
    assert!(results.is_empty());
}

#[test]
fn read_session_error_when_not_in_index() {
    let dir = tmp();
    make_index(dir.path(), &[]);
    assert!(read_session(dir.path(), "ghost-id").is_err());
}

#[test]
fn read_session_reads_markdown_file() {
    let dir = tmp();
    make_index(dir.path(), &[("abc", "Title", "2026-04-15", "Claude Code")]);
    let md_path = dir
        .path()
        .join("sessions/proj/2026-04-15/12-00-00-claudecode-sweep.md");
    fs::create_dir_all(md_path.parent().unwrap()).unwrap();
    fs::write(&md_path, "# Title\n\nContent here.").unwrap();
    let content = read_session(dir.path(), "abc").unwrap();
    assert!(content.contains("Content here."));
}
