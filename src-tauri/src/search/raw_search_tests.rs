// HalluScribe - archive traversal tests for raw transcript search.

use super::raw::{search_raw, RawSearchError, MAX_SESSION_GROUPS};
use crate::archive::read_raw_at;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

struct TestEntry<'a> {
    id: &'a str,
    date: &'a str,
    raw_path: Option<&'a str>,
}

fn tmp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("halluscribe_raw_search_test_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("raw")).unwrap();
    dir
}

fn write_index(dir: &Path, entries: &[TestEntry<'_>]) {
    let sessions: Vec<Value> = entries
        .iter()
        .map(|entry| {
            let mut value = json!({
                "id": entry.id,
                "project": "fixture-project",
                "date": entry.date,
                "title": format!("Session {}", entry.id),
                "tool": "codex",
                "fill_pct": 42.0,
                "session_type": "coding",
                "error_tags": [],
                "topic_tags": ["fixture"],
                "archive_path": format!("sessions/{}.md", entry.id),
                "source_jsonl": format!("sources/{}.jsonl", entry.id)
            });
            if let Some(raw_path) = entry.raw_path {
                value["raw_path"] = json!(raw_path);
            }
            value
        })
        .collect();
    std::fs::write(
        dir.join("index.json"),
        serde_json::to_string(&json!({ "sessions": sessions })).unwrap(),
    )
    .unwrap();
}

fn write_raw(dir: &Path, rel_path: &str, fixture: &str) {
    let compressed = zstd::encode_all(fixture.as_bytes(), 3).unwrap();
    let path = dir.join(rel_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, compressed).unwrap();
}

#[test]
fn groups_hits_newest_first_with_truthful_per_session_counts() {
    let dir = tmp_dir("grouped");
    write_index(
        &dir,
        &[
            TestEntry {
                id: "older",
                date: "2026-01-01",
                raw_path: Some("raw/older.jsonl.zst"),
            },
            TestEntry {
                id: "newer",
                date: "2026-07-01",
                raw_path: Some("raw/newer.jsonl.zst"),
            },
        ],
    );
    write_raw(
        &dir,
        "raw/older.jsonl.zst",
        concat!(
            r#"{"type":"user","message":{"content":"find TOKEN here"}}"#,
            "\n",
            r#"{"type":"tool_use","input":{"command":"echo token"}}"#
        ),
    );
    write_raw(
        &dir,
        "raw/newer.jsonl.zst",
        r#"{"type":"assistant","message":{"content":"one token"}}"#,
    );

    let result = search_raw(&dir, "token", None).unwrap();
    assert_eq!(result.sessions_scanned, 2);
    assert_eq!(result.total_hits, 3);
    assert_eq!(result.sessions.len(), 2);
    assert_eq!(result.sessions[0].session_id, "newer");
    assert_eq!(result.sessions[0].total_hits, 1);
    assert_eq!(result.sessions[1].session_id, "older");
    assert_eq!(result.sessions[1].total_hits, 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn counts_entries_without_raw_and_returns_other_matches() {
    let dir = tmp_dir("without_raw");
    write_index(
        &dir,
        &[
            TestEntry {
                id: "missing-copy",
                date: "2026-07-02",
                raw_path: None,
            },
            TestEntry {
                id: "present",
                date: "2026-07-01",
                raw_path: Some("raw/present.jsonl.zst"),
            },
        ],
    );
    write_raw(
        &dir,
        "raw/present.jsonl.zst",
        r#"{"type":"user","message":{"content":"needle"}}"#,
    );

    let result = search_raw(&dir, "needle", None).unwrap();
    assert_eq!(result.sessions_without_raw, 1);
    assert_eq!(result.sessions_scanned, 1);
    assert_eq!(result.sessions[0].session_id, "present");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn records_missing_and_corrupt_raws_then_continues() {
    let dir = tmp_dir("failed");
    write_index(
        &dir,
        &[
            TestEntry {
                id: "missing",
                date: "2026-07-03",
                raw_path: Some("raw/missing.jsonl.zst"),
            },
            TestEntry {
                id: "corrupt",
                date: "2026-07-02",
                raw_path: Some("raw/corrupt.jsonl.zst"),
            },
            TestEntry {
                id: "good",
                date: "2026-07-01",
                raw_path: Some("raw/good.jsonl.zst"),
            },
        ],
    );
    std::fs::write(dir.join("raw/corrupt.jsonl.zst"), b"not zstd data").unwrap();
    write_raw(
        &dir,
        "raw/good.jsonl.zst",
        r#"{"type":"tool_use","input":{"command":"run needle"}}"#,
    );

    let result = search_raw(&dir, "needle", None).unwrap();
    assert_eq!(result.sessions_failed, ["missing", "corrupt"]);
    assert_eq!(result.sessions_scanned, 1);
    assert_eq!(result.sessions[0].session_id, "good");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn allowed_ids_limit_matches_and_all_coverage_counters() {
    let dir = tmp_dir("scope");
    write_index(
        &dir,
        &[
            TestEntry {
                id: "allowed",
                date: "2026-07-02",
                raw_path: Some("raw/allowed.jsonl.zst"),
            },
            TestEntry {
                id: "outside-no-raw",
                date: "2026-07-01",
                raw_path: None,
            },
        ],
    );
    write_raw(
        &dir,
        "raw/allowed.jsonl.zst",
        r#"{"type":"user","message":{"content":"needle"}}"#,
    );
    let allowed_ids = HashSet::from(["allowed".to_string()]);

    let result = search_raw(&dir, "needle", Some(&allowed_ids)).unwrap();
    assert_eq!(result.sessions_scanned, 1);
    assert_eq!(result.sessions_without_raw, 0);
    assert!(result.sessions_failed.is_empty());
    assert_eq!(result.sessions.len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_empty_query_before_reading_the_archive() {
    let missing_dir = std::env::temp_dir().join("halluscribe_raw_search_missing_archive");
    let result = search_raw(&missing_dir, " \t\n", None);
    assert!(matches!(result, Err(RawSearchError::EmptyQuery)));
}

#[test]
fn absent_needle_returns_no_groups_and_truthful_scan_count() {
    let dir = tmp_dir("absent");
    write_index(
        &dir,
        &[TestEntry {
            id: "only",
            date: "2026-07-01",
            raw_path: Some("raw/only.jsonl.zst"),
        }],
    );
    write_raw(
        &dir,
        "raw/only.jsonl.zst",
        r#"{"type":"assistant","message":{"content":"nothing relevant"}}"#,
    );

    let result = search_raw(&dir, "needle", None).unwrap();
    assert_eq!(result.sessions_scanned, 1);
    assert_eq!(result.total_hits, 0);
    assert!(result.sessions.is_empty());
    assert!(!result.results_truncated);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn read_raw_at_rejects_parent_and_absolute_paths() {
    let dir = tmp_dir("unsafe_paths");
    assert!(read_raw_at(&dir, "../escape.zst").is_err());
    assert!(read_raw_at(&dir, &dir.join("absolute.zst").to_string_lossy()).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn global_group_cap_retains_newest_but_counts_overflow_hits() {
    let dir = tmp_dir("global_cap");
    let ids: Vec<String> = (0..=MAX_SESSION_GROUPS)
        .map(|index| format!("session-{index:03}"))
        .collect();
    let dates: Vec<String> = (0..=MAX_SESSION_GROUPS)
        .map(|index| format!("2026-{index:03}"))
        .collect();
    let entries: Vec<TestEntry<'_>> = ids
        .iter()
        .zip(&dates)
        .map(|(id, date)| TestEntry {
            id,
            date,
            raw_path: Some(id),
        })
        .collect();
    write_index(&dir, &entries);
    for id in &ids {
        write_raw(
            &dir,
            id,
            r#"{"type":"user","message":{"content":"needle"}}"#,
        );
    }

    let result = search_raw(&dir, "needle", None).unwrap();
    assert_eq!(result.sessions_scanned, MAX_SESSION_GROUPS + 1);
    assert_eq!(result.sessions.len(), MAX_SESSION_GROUPS);
    assert_eq!(result.total_hits, MAX_SESSION_GROUPS + 1);
    assert!(result.results_truncated);
    assert_eq!(result.sessions[0].session_id, "session-200");
    assert_eq!(result.sessions.last().unwrap().session_id, "session-001");
    std::fs::remove_dir_all(dir).unwrap();
}
