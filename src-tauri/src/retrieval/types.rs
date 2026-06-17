// HalluScribe - semantic retrieval data types.

use crate::archive::IndexEntry;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingRecord {
    pub session_id: String,
    pub model_name: String,
    pub summary_hash: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticSearchResult {
    pub entry: IndexEntry,
    pub score: f32,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct EmbeddingRebuildResult {
    pub indexed: u32,
    pub skipped: u32,
    pub failed: u32,
}
