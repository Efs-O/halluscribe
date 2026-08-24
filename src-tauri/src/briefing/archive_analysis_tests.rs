// HalluScribe - regression tests for deterministic archive analysis chat tools.

use super::*;
use serde_json::json;
use std::fs;

fn write_archive() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let sessions = json!({ "sessions": [
        { "id": "forge-july", "project": "Forge", "date": "2026-07-05", "title": "One", "tool": "Forge", "fill_pct": 1.0, "session_type": "building", "error_tags": [], "topic_tags": [], "archive_path": "sessions/forge-july.md", "source_jsonl": "sources/forge-july.jsonl", "raw_path": "raw/forge-july.zst" },
        { "id": "codex-july", "project": "App", "date": "2026-07-10", "title": "Two", "tool": "Codex", "fill_pct": 1.0, "session_type": "debugging", "error_tags": [], "topic_tags": [], "archive_path": "sessions/codex-july.md", "source_jsonl": "sources/codex-july.jsonl", "raw_path": "" },
        { "id": "gemini-august", "project": "App", "date": "2026-08-05", "title": "Three", "tool": "Gemini", "fill_pct": 1.0, "session_type": "debugging", "error_tags": [], "topic_tags": [], "archive_path": "sessions/gemini-august.md", "source_jsonl": "sources/gemini-august.jsonl", "raw_path": "raw/gemini-august.zst" },
        { "id": "forge-august", "project": "Forge", "date": "2026-08-10", "title": "Four", "tool": "Forge", "fill_pct": 1.0, "session_type": "building", "error_tags": [], "topic_tags": [], "archive_path": "sessions/forge-august.md", "source_jsonl": "sources/forge-august.jsonl", "raw_path": "raw/forge-august.zst" }
    ]});
    fs::write(
        dir.path().join("index.json"),
        serde_json::to_string(&sessions).unwrap(),
    )
    .unwrap();
    for path in [
        "sessions/forge-july.md",
        "sessions/codex-july.md",
        "sessions/forge-august.md",
        "raw/forge-july.zst",
        "raw/gemini-august.zst",
        "raw/forge-august.zst",
    ] {
        let full_path = dir.path().join(path);
        fs::create_dir_all(full_path.parent().unwrap()).unwrap();
        fs::write(full_path, "fixture").unwrap();
    }
    dir
}

#[test]
fn analysis_tool_schemas_expose_the_expected_toolset() {
    let tools = tool_schemas();
    let names = tools
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "count_sessions_by_provider",
            "list_archive_facets",
            "compare_session_periods",
            "archive_health"
        ]
    );
}

#[test]
fn analysis_tools_return_exact_archive_metadata() {
    let dir = write_archive();
    let scope = ChatScope::ArchiveWide;

    let facets: serde_json::Value = serde_json::from_str(
        &execute(
            dir.path(),
            &scope,
            "list_archive_facets",
            &json!({ "date_from": "2026-07-01", "date_to": "2026-07-31" }),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(facets["total_sessions"], 2);
    assert_eq!(
        facets["providers"][0],
        json!({ "value": "Codex", "session_count": 1 })
    );
    assert_eq!(
        facets["providers"][1],
        json!({ "value": "Forge", "session_count": 1 })
    );

    let comparison: serde_json::Value = serde_json::from_str(
        &execute(
            dir.path(),
            &scope,
            "compare_session_periods",
            &json!({
                "first_date_from": "2026-07-01", "first_date_to": "2026-07-31",
                "second_date_from": "2026-08-01", "second_date_to": "2026-08-31"
            }),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(comparison["first_period"]["total_sessions"], 2);
    assert_eq!(comparison["second_period"]["total_sessions"], 2);
    assert_eq!(comparison["groups"][0]["value"], "Codex");
    assert_eq!(comparison["groups"][0]["absolute_change"], -1);

    let health: serde_json::Value =
        serde_json::from_str(&execute(dir.path(), &scope, "archive_health", &json!({})).unwrap())
            .unwrap();
    assert_eq!(health["indexed_sessions"], 4);
    assert_eq!(health["summary_files_present"], 3);
    assert_eq!(health["raw_copies_present"], 3);
}
