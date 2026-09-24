// HalluScribe - tests for the briefing keyword filter: case- and
// Greek-accent-insensitive, in the index fields and the session body alike.
// Synthetic archive only.

use super::{collect_briefing_sessions, BriefingFilters, BriefingScope};
use std::fs;
use std::path::Path;

/// Two sessions: `older` (2026-01-01) and `newer` (2026-02-01), each with its
/// own title and markdown body.
fn make_archive(dir: &Path, older: (&str, &str), newer: (&str, &str)) {
    let sessions: Vec<serde_json::Value> =
        [("old", "2026-01-01", older), ("new", "2026-02-01", newer)]
            .iter()
            .map(|(id, date, (title, body))| {
                let rel = format!("sessions/proj/{date}/12-00-00-{id}.md");
                let md = dir.join(&rel);
                fs::create_dir_all(md.parent().unwrap()).unwrap();
                fs::write(&md, body).unwrap();
                serde_json::json!({
                    "id": id, "project": "proj", "date": date, "title": title,
                    "tool": "Forge", "fill_pct": 50.0, "session_type": "chat",
                    "error_tags": [], "topic_tags": [], "archive_path": rel,
                    "source_jsonl": "/fake/path.jsonl"
                })
            })
            .collect();
    let idx = serde_json::json!({ "sessions": sessions });
    fs::write(dir.join("index.json"), idx.to_string()).unwrap();
}

/// How many sessions a keyword-only filter selects (the fallback takes both).
fn selected(dir: &Path, keyword: &str) -> usize {
    let scope = BriefingScope::FilterBased(BriefingFilters {
        date_from: None,
        date_to: None,
        fill_min: None,
        fill_max: None,
        keyword: Some(keyword.to_string()),
    });
    collect_briefing_sessions(dir, &scope, 10)
        .expect("collect")
        .2
}

#[test]
fn an_accented_keyword_matches_the_folded_body() {
    // The body cache is folded, so an accented keyword must be folded too.
    let dir = tempfile::tempdir().unwrap();
    make_archive(
        dir.path(),
        ("Old", "Καλημέρα, στείλτε την προσφορά."),
        ("New", "nothing relevant"),
    );
    assert_eq!(selected(dir.path(), "Καλημέρα"), 1);
}

#[test]
fn an_uppercase_keyword_matches_an_accented_title() {
    let dir = tempfile::tempdir().unwrap();
    make_archive(dir.path(), ("Οδός Πανεπιστημίου", "x"), ("New", "y"));
    assert_eq!(selected(dir.path(), "ΟΔΟΣ"), 1);
}
