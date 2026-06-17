// HalluScribe - archive metadata and index entry types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub struct SessionMeta {
    pub id: String,
    pub source: PathBuf,
    pub project: String,
    pub tool: String,
    pub provider: String,
    pub fill_pct: f64,
    pub fill_estimated: bool,
    pub backend: String,
    pub session_timestamp: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub transcript_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub id: String,
    pub project: String,
    pub date: String,
    pub title: String,
    pub tool: String,
    pub fill_pct: f64,
    #[serde(default)]
    pub session_timestamp: String,
    #[serde(default)]
    pub updated_at: String,
    pub session_type: String,
    pub error_tags: Vec<String>,
    pub topic_tags: Vec<String>,
    pub archive_path: String,
    pub source_jsonl: String,
    #[serde(default)]
    pub source_size_bytes: u64,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub fill_estimated: bool,
    #[serde(default)]
    pub transcript_hash: String,
}
