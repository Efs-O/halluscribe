// HalluScribe - archive traversal tests for raw transcript search.

use super::raw::{search_raw, RawSearchError, MAX_SESSION_GROUPS};
use crate::archive::{read_raw_at, save_captured, CapturedManifest, CapturedRecord};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

struct TestEntry<'a> {
    id: &'a str,
    date: &'a str,
    raw_path: Option<&'a str>,
}

fn tmp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "halluscribe_raw_search_test_{}_{}",
        std::process::id(),
        name
    ));
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

/// Write a `raw/captured.json` recording one captured-but-unsummarised
/// session, mirroring what `archive::capture::run_capture` would have
/// written for a coding-tool source file with no index entry yet.
fn write_captured(dir: &Path, id: &str, source_filename: &str, mtime_secs: i64) {
    let mut manifest = CapturedManifest::new();
    manifest.insert(
        id.to_string(),
        CapturedRecord {
            source_path: format!("C:/fake/home/.claude/projects/proj/{source_filename}"),
            size: 123,
            mtime_secs,
            captured_at: "2026-07-15T00:00:00Z".to_string(),
        },
    );
    save_captured(dir, &manifest).unwrap();
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
fn greek_query_finds_accented_raw() {
    let dir = tmp_dir("greek");
    write_index(
        &dir,
        &[TestEntry {
            id: "greek",
            date: "2026-07-01",
            raw_path: Some("raw/greek.zst"),
        }],
    );
    // The raw holds the accented word Καλημέρα (tonos on the epsilon); the
    // query is the plain accent-free form. D9 folding must make them match.
    // Codepoints are explicit so the accented vowel is unambiguous.
    let accented = "\u{039A}\u{03B1}\u{03BB}\u{03B7}\u{03BC}\u{03AD}\u{03C1}\u{03B1}"; // Καλημέρα
    write_raw(
        &dir,
        "raw/greek.zst",
        format!("line one\n{accented} world\n").as_str(),
    );
    let plain = "\u{03BA}\u{03B1}\u{03BB}\u{03B7}\u{03BC}\u{03B5}\u{03C1}\u{03B1}"; // καλημερα
    let result = search_raw(&dir, plain, None).unwrap();
    assert_eq!(result.sessions_scanned, 1);
    assert_eq!(result.sessions[0].session_id, "greek");
    assert_eq!(result.total_hits, 1);
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
    let missing_dir = std::env::temp_dir().join(format!(
        "halluscribe_raw_search_missing_archive_{}",
        std::process::id()
    ));
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

#[test]
fn captured_only_session_appears_flagged_unsummarised() {
    let dir = tmp_dir("captured_only");
    write_index(
        &dir,
        &[TestEntry {
            id: "indexed",
            date: "2026-07-10",
            raw_path: Some("raw/indexed.jsonl.zst"),
        }],
    );
    write_raw(
        &dir,
        "raw/indexed.jsonl.zst",
        r#"{"type":"user","message":{"content":"needle in indexed session"}}"#,
    );
    // Captured-only session: present in captured.json, absent from index.json.
    // Uses the standard raw_rel_path layout, exactly like preserve_raw writes.
    write_captured(&dir, "captured-only", "session-x.jsonl", 1_752_000_000);
    write_raw(
        &dir,
        "raw/captured-only.jsonl.zst",
        r#"{"type":"user","message":{"content":"needle in captured-only session"}}"#,
    );

    let result = search_raw(&dir, "needle", None).unwrap();
    assert_eq!(result.sessions.len(), 2);

    let indexed = result
        .sessions
        .iter()
        .find(|s| s.session_id == "indexed")
        .unwrap();
    assert!(indexed.summarised);
    assert!(indexed.title.is_empty());
    assert!(indexed.date.is_empty());

    let captured = result
        .sessions
        .iter()
        .find(|s| s.session_id == "captured-only")
        .unwrap();
    assert!(!captured.summarised);
    assert_eq!(captured.title, "session-x.jsonl");
    assert!(!captured.date.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn captured_only_session_hidden_when_allowed_ids_scopes_the_search() {
    let dir = tmp_dir("captured_scoped");
    write_index(
        &dir,
        &[TestEntry {
            id: "indexed",
            date: "2026-07-10",
            raw_path: Some("raw/indexed.jsonl.zst"),
        }],
    );
    write_raw(
        &dir,
        "raw/indexed.jsonl.zst",
        r#"{"type":"user","message":{"content":"needle"}}"#,
    );
    write_captured(&dir, "captured-only", "session-x.jsonl", 1_752_000_000);
    write_raw(
        &dir,
        "raw/captured-only.jsonl.zst",
        r#"{"type":"user","message":{"content":"needle"}}"#,
    );

    let allowed_ids = HashSet::from(["indexed".to_string()]);
    let result = search_raw(&dir, "needle", Some(&allowed_ids)).unwrap();
    assert_eq!(result.sessions.len(), 1);
    assert_eq!(result.sessions[0].session_id, "indexed");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn captured_only_session_not_duplicated_when_also_indexed() {
    let dir = tmp_dir("captured_also_indexed");
    write_index(
        &dir,
        &[TestEntry {
            id: "both",
            date: "2026-07-10",
            raw_path: Some("raw/both.jsonl.zst"),
        }],
    );
    write_raw(
        &dir,
        "raw/both.jsonl.zst",
        r#"{"type":"user","message":{"content":"needle"}}"#,
    );
    // Same id recorded in captured.json (as capture would after a sweep later
    // summarises it) - must not produce a second, unsummarised group.
    write_captured(&dir, "both", "session-both.jsonl", 1_752_000_000);

    let result = search_raw(&dir, "needle", None).unwrap();
    assert_eq!(result.sessions.len(), 1);
    assert!(result.sessions[0].summarised);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn captured_only_session_matches_utf8_greek_needle() {
    let dir = tmp_dir("captured_greek");
    write_captured(&dir, "greek-session", "session-greek.jsonl", 1_752_000_000);
    write_raw(
        &dir,
        "raw/greek-session.jsonl.zst",
        r#"{"type":"user","message":{"content":"αυτή είναι μια δοκιμή στα ελληνικά"}}"#,
    );

    let result = search_raw(&dir, "δοκιμή", None).unwrap();
    assert_eq!(result.sessions.len(), 1);
    assert_eq!(result.sessions[0].session_id, "greek-session");
    assert!(!result.sessions[0].summarised);
    assert_eq!(result.total_hits, 1);
    std::fs::remove_dir_all(dir).unwrap();
}

// Manual benchmark against a real archive (plan step 3 — decides whether
// rayon is worth asking for). Never runs in CI: gated on an env var AND
// `#[ignore]`. Usage:
//   HALLUSCRIBE_BENCH_ARCHIVE=<archive dir> \
//     cargo test --release bench_search_raw -- --ignored --nocapture
#[test]
#[ignore = "manual benchmark; set HALLUSCRIBE_BENCH_ARCHIVE"]
fn bench_search_raw_real_archive() {
    let Ok(dir) = std::env::var("HALLUSCRIBE_BENCH_ARCHIVE") else {
        eprintln!("HALLUSCRIBE_BENCH_ARCHIVE not set; skipping");
        return;
    };
    let dir = PathBuf::from(dir);
    for needle in ["netsh winsock reset", "cargo clippy", "zzz_no_such_token"] {
        let started = std::time::Instant::now();
        let result = search_raw(&dir, needle, None).unwrap();
        eprintln!(
            "needle {needle:?}: {:?} — scanned {}, without_raw {}, failed {}, hits {} in {} sessions{}",
            started.elapsed(),
            result.sessions_scanned,
            result.sessions_without_raw,
            result.sessions_failed.len(),
            result.total_hits,
            result.sessions.len(),
            if result.results_truncated { " (truncated)" } else { "" },
        );
    }
}
