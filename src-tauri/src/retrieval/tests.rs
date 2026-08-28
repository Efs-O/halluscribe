// HalluScribe - semantic retrieval tests over archive/index fixtures.

use super::{semantic_scope_ids_from_results, semantic_search_for_embedding};
use crate::archive::IndexEntry;
use crate::retrieval::types::{EmbeddingRecord, SemanticSearchResult};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn tmp() -> TempDir {
    tempfile::tempdir().unwrap()
}

fn make_index(dir: &Path, entries: &[(&str, &str, &str)]) {
    let sessions: Vec<serde_json::Value> = entries
        .iter()
        .map(|(id, title, date)| {
            serde_json::json!({
                "id": id,
                "project": "proj",
                "date": date,
                "title": title,
                "tool": "Claude Code",
                "fill_pct": 60.0,
                "session_type": "debugging",
                "error_tags": ["network"],
                "topic_tags": ["rust"],
                "archive_path": format!("sessions/proj/{date}/{id}.md"),
                "source_jsonl": "/fake/path.jsonl"
            })
        })
        .collect();
    let idx = serde_json::json!({ "sessions": sessions });
    fs::write(dir.join("index.json"), serde_json::to_string(&idx).unwrap()).unwrap();
}

fn save_embeddings(dir: &Path, records: &[EmbeddingRecord]) {
    let payload = serde_json::json!({ "records": records });
    fs::write(
        dir.join("embeddings.json"),
        serde_json::to_string_pretty(&payload).unwrap(),
    )
    .unwrap();
}

#[test]
fn semantic_search_filters_records_to_active_model_name() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[
            ("match-a", "Port bug fix", "2026-04-20"),
            ("match-b", "Refactor notes", "2026-04-19"),
        ],
    );
    save_embeddings(
        dir.path(),
        &[
            EmbeddingRecord {
                session_id: "match-a".into(),
                model_name: "embeddinggemma-good".into(),
                summary_hash: "1".into(),
                embedding: vec![1.0, 0.0],
            },
            EmbeddingRecord {
                session_id: "match-b".into(),
                model_name: "embeddinggemma-stale".into(),
                summary_hash: "2".into(),
                embedding: vec![1.0, 0.0],
            },
        ],
    );

    let results =
        semantic_search_for_embedding(dir.path(), &[1.0, 0.0], "embeddinggemma-good", 5, None)
            .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entry.id, "match-a");
}

#[test]
fn semantic_scope_ids_returns_ranked_ids() {
    let results = vec![
        SemanticSearchResult {
            entry: IndexEntry {
                id: "b".into(),
                project: "proj".into(),
                date: "2026-04-21".into(),
                title: "Second".into(),
                tool: "Claude Code".into(),
                fill_pct: 55.0,
                session_timestamp: String::new(),
                updated_at: String::new(),
                session_type: "debugging".into(),
                error_tags: vec![],
                topic_tags: vec![],
                archive_path: "sessions/proj/2026-04-21/b.md".into(),
                source_jsonl: "/fake/b.jsonl".into(),
                source_size_bytes: 0,
                provider: String::new(),
                fill_estimated: false,
                output_tokens: 0,
                tokens_estimated: true,
                transcript_hash: String::new(),
                secret_flags: Vec::new(),
                raw_path: String::new(),
            },
            score: 0.91,
        },
        SemanticSearchResult {
            entry: IndexEntry {
                id: "a".into(),
                project: "proj".into(),
                date: "2026-04-20".into(),
                title: "First".into(),
                tool: "Claude Code".into(),
                fill_pct: 52.0,
                session_timestamp: String::new(),
                updated_at: String::new(),
                session_type: "debugging".into(),
                error_tags: vec![],
                topic_tags: vec![],
                archive_path: "sessions/proj/2026-04-20/a.md".into(),
                source_jsonl: "/fake/a.jsonl".into(),
                source_size_bytes: 0,
                provider: String::new(),
                fill_estimated: false,
                output_tokens: 0,
                tokens_estimated: true,
                transcript_hash: String::new(),
                secret_flags: Vec::new(),
                raw_path: String::new(),
            },
            score: 0.88,
        },
    ];

    let ids = semantic_scope_ids_from_results(results).unwrap();
    assert_eq!(ids, vec!["b".to_string(), "a".to_string()]);
}

#[test]
fn semantic_scope_ids_errors_on_empty_results() {
    let error = semantic_scope_ids_from_results(Vec::new()).unwrap_err();
    assert!(error.contains("Semantic search found no relevant archived sessions"));
}
