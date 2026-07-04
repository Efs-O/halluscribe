// HalluScribe - archive writer and index access for processed sessions.

mod index;
mod raw;
mod redact;
mod types;
mod writer;

use std::fmt;

pub use types::{IndexEntry, SessionMeta, WrittenSession};

#[derive(Debug)]
pub enum ArchiveError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Invalid(String),
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Json(e) => write!(f, "JSON error: {e}"),
            Self::Invalid(msg) => write!(f, "{msg}"),
        }
    }
}

impl From<std::io::Error> for ArchiveError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for ArchiveError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

pub use index::{
    archived_source_size, delete_sessions, find_session, is_archived, read_sessions, session_id,
    set_secret_flags,
};
pub use raw::{preserve_raw, raw_rel_path, read_raw, RAW_DIR};
pub use redact::{
    apply_redaction, load_rules, preview_redaction, rules_for_session, RedactionOutcome,
    RedactionPreview, RedactionRule,
};
pub use writer::write_session;
