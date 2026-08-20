// HalluScribe - archive writer and index access for processed sessions.

mod backfill;
#[cfg(test)]
mod backfill_tests;
mod capture;
mod captured_manifest;
mod index;
mod raw;
#[cfg(test)]
mod raw_tests;
mod read_raw_session;
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

pub use backfill::{backfill_raw, BackfillResult};
pub use capture::{run_capture, CaptureStatus};
pub use captured_manifest::{load_captured, save_captured, CapturedManifest, CapturedRecord};
pub use index::{
    archived_source_size, delete_sessions, ensure_index_readable, find_session, index_stamp,
    is_archived, read_sessions, session_id, set_raw_path, set_secret_flags, IndexStamp,
};
pub use raw::{
    preserve_raw, preserve_raw_bytes, raw_rel_path, read_raw, read_raw_at, PreservedRaw, RAW_DIR,
    SUPERSEDED_DIR,
};
pub use read_raw_session::{read_raw_session, RawSessionPage};
pub use redact::{
    apply_redaction, load_rules, preview_redaction, rules_for_session, RedactionOutcome,
    RedactionPreview, RedactionRule,
};
pub use writer::write_session;
