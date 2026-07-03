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
fn limit_capped_at_30() {
    let dir = tmp();
    let entries: Vec<_> = (0u8..35)
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
    assert_eq!(results.len(), 30);
}

#[test]
fn page_reports_true_total_and_pages_via_offset() {
    let dir = tmp();
    let entries: Vec<_> = (0u8..35)
        .map(|i| {
            (
                Box::leak(i.to_string().into_boxed_str()) as &str,
                "Auth work",
                "2026-04-15",
                "Claude Code",
            )
        })
        .collect();
    make_index(dir.path(), &entries);

    // First page: default size 20, but the total is the full 35 matches.
    let page = search_sessions_page(
        dir.path(),
        &SearchParams {
            query: Some("auth".into()),
            ..Default::default()
        },
        None,
    );
    assert_eq!(page.searched, 35);
    assert_eq!(page.total_matches, 35);
    assert_eq!(page.returned, 20);
    assert_eq!(page.offset, 0);
    assert_eq!(page.results.len(), 20);

    // Second page via offset: remaining 5, total still reported as 35.
    let page2 = search_sessions_page(
        dir.path(),
        &SearchParams {
            query: Some("auth".into()),
            offset: Some(30),
            ..Default::default()
        },
        None,
    );
    assert_eq!(page2.total_matches, 35);
    assert_eq!(page2.returned, 5);
    assert_eq!(page2.offset, 30);
}

#[test]
fn page_searched_counts_full_scope_even_when_few_match() {
    let dir = tmp();
    // 10 sessions in scope; only 2 mention the query term.
    let mut entries: Vec<(&str, &str, &str, &str)> = (0u8..8)
        .map(|i| {
            (
                Box::leak(i.to_string().into_boxed_str()) as &str,
                "Unrelated",
                "2026-04-15",
                "Claude Code",
            )
        })
        .collect();
    entries.push(("m1", "ftp password rotate", "2026-04-15", "Claude Code"));
    entries.push(("m2", "old ftp password note", "2026-04-15", "Claude Code"));
    make_index(dir.path(), &entries);

    let page = search_sessions_page(
        dir.path(),
        &SearchParams {
            query: Some("ftp password".into()),
            ..Default::default()
        },
        None,
    );
    // "searched" is the whole scope; "total_matches" is the matching subset.
    assert_eq!(page.searched, 10);
    assert_eq!(page.total_matches, 2);
    assert_eq!(page.returned, 2);
}

#[test]
fn query_matches_md_body_when_not_in_metadata() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[("a", "Unrelated title", "2026-04-15", "Claude Code")],
    );
    // "password" is absent from title/error_tags/topic_tags; it lives only in the body.
    let md_path = dir
        .path()
        .join("sessions/proj/2026-04-15/12-00-00-claudecode-sweep.md");
    fs::create_dir_all(md_path.parent().unwrap()).unwrap();
    fs::write(
        &md_path,
        "# Notes\n\nRotated the database PASSWORD after the leak.",
    )
    .unwrap();

    let results = search_sessions(
        dir.path(),
        &SearchParams {
            query: Some("password".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "a");
}

#[test]
fn query_absent_from_metadata_and_body_returns_nothing() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[("a", "Unrelated title", "2026-04-15", "Claude Code")],
    );
    let md_path = dir
        .path()
        .join("sessions/proj/2026-04-15/12-00-00-claudecode-sweep.md");
    fs::create_dir_all(md_path.parent().unwrap()).unwrap();
    fs::write(&md_path, "# Notes\n\nNothing sensitive here.").unwrap();

    let results = search_sessions(
        dir.path(),
        &SearchParams {
            query: Some("password".into()),
            ..Default::default()
        },
    );
    assert!(results.is_empty());
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
