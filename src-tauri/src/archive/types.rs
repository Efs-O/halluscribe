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
    pub output_tokens: u64,
    pub tokens_estimated: bool,
    pub backend: String,
    pub session_timestamp: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub transcript_hash: String,
    /// Relative path (e.g. `raw/<id>.jsonl.zst`) to the preserved raw transcript,
    /// set unconditionally by the sweep runner. `None` when the copy failed;
    /// recorded into the index entry.
    pub raw_path: Option<String>,
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
    /// Tokens the model generated in this session. Zero on entries written
    /// before the column existed; they report a real value once re-swept.
    #[serde(default)]
    pub output_tokens: u64,
    /// True when `output_tokens` was inferred from character counts rather than
    /// reported by the server. Defaults to true so a pre-existing entry's zero
    /// is never mistaken for a measured zero.
    #[serde(default = "default_true")]
    pub tokens_estimated: bool,
    #[serde(default)]
    pub transcript_hash: String,
    #[serde(default)]
    pub secret_flags: Vec<String>,
    /// Relative path to the preserved raw transcript (`raw/<id>.jsonl.zst`), or
    /// empty when raw preservation is off/failed for this session.
    #[serde(default)]
    pub raw_path: String,
}

fn default_true() -> bool {
    true
}

/// Result of writing one session's markdown + index entry to the archive.
pub struct WrittenSession {
    pub path: PathBuf,
    pub secret_flags: Vec<String>,
}
