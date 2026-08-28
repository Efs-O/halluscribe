// HalluScribe - embedding store and archive-to-embedding text helpers.

use super::types::EmbeddingRecord;
use crate::archive::IndexEntry;
use crate::search;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// EmbeddingGemma's GGUF advertises a 2,048-token training context. A byte
/// cap stays safely below it even for text that tokenizes poorly, while leaving
/// enough space for the semantic metadata and a substantial summary prefix.
pub(crate) const MAX_EMBEDDING_INPUT_BYTES: usize = 1_800;

#[derive(Debug, Default, Serialize, Deserialize)]
struct EmbeddingStore {
    records: Vec<EmbeddingRecord>,
}

pub fn load_embeddings(archive_dir: &Path) -> Result<Vec<EmbeddingRecord>, String> {
    let path = embeddings_path(archive_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let store: EmbeddingStore = serde_json::from_str(&raw)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    Ok(store.records)
}

pub fn upsert_embedding(archive_dir: &Path, record: EmbeddingRecord) -> Result<(), String> {
    let mut records = load_embeddings(archive_dir)?;
    records.retain(|existing| existing.session_id != record.session_id);
    records.push(record);
    save_embeddings(archive_dir, records)
}

pub fn replace_embeddings(
    archive_dir: &Path,
    mut records: Vec<EmbeddingRecord>,
) -> Result<(), String> {
    records.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    save_embeddings(archive_dir, records)
}

pub fn remove_embeddings(archive_dir: &Path, session_ids: &[String]) -> Result<(), String> {
    if session_ids.is_empty() {
        return Ok(());
    }
    let id_set = session_ids.iter().cloned().collect::<HashSet<_>>();
    let records = load_embeddings(archive_dir)?
        .into_iter()
        .filter(|record| !id_set.contains(&record.session_id))
        .collect::<Vec<_>>();
    save_embeddings(archive_dir, records)
}

pub fn has_current_embedding(
    archive_dir: &Path,
    session_id: &str,
    model_name: &str,
    summary_hash: &str,
) -> Result<bool, String> {
    Ok(load_embeddings(archive_dir)?.into_iter().any(|record| {
        record.session_id == session_id
            && record.model_name == model_name
            && record.summary_hash == summary_hash
    }))
}

pub fn build_embedding_input(archive_dir: &Path, entry: &IndexEntry) -> Result<String, String> {
    let markdown = search::read_session(archive_dir, &entry.id)?;
    let summary = normalize_whitespace(&extract_summary_body(&markdown));
    let error_tags = if entry.error_tags.is_empty() {
        "none".to_string()
    } else {
        entry.error_tags.join(", ")
    };
    let topic_tags = if entry.topic_tags.is_empty() {
        "none".to_string()
    } else {
        entry.topic_tags.join(", ")
    };
    Ok(format!(
        "title: {}\nproject: {}\ntool: {}\nsession type: {}\nerror tags: {}\ntopic tags: {}\nsummary:\n{}",
        entry.title,
        entry.project,
        entry.tool,
        entry.session_type,
        error_tags,
        topic_tags,
        summary,
    ))
}

/// Keep an embedding request inside the model's actual context limit without
/// splitting a UTF-8 character. The caller applies this after adding its query
/// or document prefix, so this is the final byte budget sent to llama.cpp.
pub(crate) fn limit_embedding_input(text: String) -> String {
    if text.len() <= MAX_EMBEDDING_INPUT_BYTES {
        return text;
    }
    let mut end = MAX_EMBEDDING_INPUT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].trim_end().to_string()
}

pub fn summary_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn save_embeddings(archive_dir: &Path, records: Vec<EmbeddingRecord>) -> Result<(), String> {
    fs::create_dir_all(archive_dir)
        .map_err(|error| format!("failed to create {}: {error}", archive_dir.display()))?;
    let store = EmbeddingStore { records };
    let json = serde_json::to_string_pretty(&store)
        .map_err(|error| format!("failed to serialize embeddings store: {error}"))?;
    let path = embeddings_path(archive_dir);
    crate::atomic_file::write_atomic(&path, json)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn embeddings_path(archive_dir: &Path) -> PathBuf {
    archive_dir.join("embeddings.json")
}

fn extract_summary_body(markdown: &str) -> String {
    let Some(after_header) = markdown.split("\n---\n\n").nth(1) else {
        return markdown.to_string();
    };
    after_header
        .split("\n\n---\n")
        .next()
        .unwrap_or(after_header)
        .trim()
        .to_string()
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("halluscribe_retrieval_{name}_{nanos}"))
    }

    #[test]
    fn summary_hash_is_stable() {
        assert_eq!(summary_hash("abc"), summary_hash("abc"));
        assert_ne!(summary_hash("abc"), summary_hash("abd"));
    }

    #[test]
    fn embedding_input_limit_is_byte_bounded_and_utf8_safe() {
        let text = format!("prefix {}", "α".repeat(MAX_EMBEDDING_INPUT_BYTES));
        let limited = limit_embedding_input(text);

        assert!(limited.len() <= MAX_EMBEDDING_INPUT_BYTES);
        assert!(limited.is_char_boundary(limited.len()));
        assert!(limited.starts_with("prefix "));
    }

    #[test]
    fn upsert_embedding_replaces_existing_session_record() {
        let dir = temp_dir("upsert");
        upsert_embedding(
            &dir,
            EmbeddingRecord {
                session_id: "a".into(),
                model_name: "m".into(),
                summary_hash: "1".into(),
                embedding: vec![0.1],
            },
        )
        .unwrap();
        upsert_embedding(
            &dir,
            EmbeddingRecord {
                session_id: "a".into(),
                model_name: "m".into(),
                summary_hash: "2".into(),
                embedding: vec![0.2],
            },
        )
        .unwrap();

        let records = load_embeddings(&dir).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].summary_hash, "2");
    }

    #[test]
    fn has_current_embedding_requires_matching_model_name() {
        let dir = temp_dir("current_model");
        upsert_embedding(
            &dir,
            EmbeddingRecord {
                session_id: "a".into(),
                model_name: "m1".into(),
                summary_hash: "1".into(),
                embedding: vec![0.1],
            },
        )
        .unwrap();

        assert!(has_current_embedding(&dir, "a", "m1", "1").unwrap());
        assert!(!has_current_embedding(&dir, "a", "m2", "1").unwrap());
    }

    #[test]
    fn remove_embeddings_drops_matching_ids() {
        let dir = temp_dir("remove");
        upsert_embedding(
            &dir,
            EmbeddingRecord {
                session_id: "a".into(),
                model_name: "m".into(),
                summary_hash: "1".into(),
                embedding: vec![0.1],
            },
        )
        .unwrap();
        upsert_embedding(
            &dir,
            EmbeddingRecord {
                session_id: "b".into(),
                model_name: "m".into(),
                summary_hash: "2".into(),
                embedding: vec![0.2],
            },
        )
        .unwrap();

        remove_embeddings(&dir, &["a".to_string()]).unwrap();
        let records = load_embeddings(&dir).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session_id, "b");
    }
}
