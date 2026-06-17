// HalluScribe - cosine similarity ranking for semantic retrieval.

use super::types::{EmbeddingRecord, SemanticSearchResult};
use crate::archive;
use std::collections::HashSet;
use std::path::Path;

pub fn rank_matches(
    query_embedding: &[f32],
    records: Vec<EmbeddingRecord>,
    limit: usize,
    allowed_ids: Option<&HashSet<String>>,
) -> Vec<(String, f32)> {
    let mut matches = records
        .into_iter()
        .filter(|record| {
            allowed_ids
                .map(|ids| ids.contains(&record.session_id))
                .unwrap_or(true)
        })
        .filter_map(|record| {
            cosine_similarity(query_embedding, &record.embedding)
                .map(|score| (record.session_id, score))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|a, b| b.1.total_cmp(&a.1));
    matches.truncate(limit.max(1));
    matches
}

pub fn attach_entries(
    archive_dir: &Path,
    matches: Vec<(String, f32)>,
) -> Vec<SemanticSearchResult> {
    matches
        .into_iter()
        .filter_map(|(session_id, score)| {
            archive::find_session(archive_dir, &session_id)
                .map(|entry| SemanticSearchResult { entry, score })
        })
        .collect()
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for (lhs, rhs) in left.iter().zip(right.iter()) {
        dot += lhs * rhs;
        left_norm += lhs * lhs;
        right_norm += rhs * rhs;
    }
    if left_norm <= f32::EPSILON || right_norm <= f32::EPSILON {
        return None;
    }
    Some(dot / (left_norm.sqrt() * right_norm.sqrt()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_matches_orders_by_similarity() {
        let records = vec![
            EmbeddingRecord {
                session_id: "a".into(),
                model_name: "m".into(),
                summary_hash: "1".into(),
                embedding: vec![1.0, 0.0],
            },
            EmbeddingRecord {
                session_id: "b".into(),
                model_name: "m".into(),
                summary_hash: "2".into(),
                embedding: vec![0.0, 1.0],
            },
        ];

        let matches = rank_matches(&[1.0, 0.0], records, 5, None);
        assert_eq!(matches[0].0, "a");
        assert!(matches[0].1 > matches[1].1);
    }
}
