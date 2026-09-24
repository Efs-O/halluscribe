// HalluScribe - scheduler configuration and sweep result types.

use crate::gemma::InferenceBackend;
use crate::settings::HalluScribeSettings;
use serde::Serialize;
use std::path::PathBuf;

/// Configuration for one sweep pass.
/// Phase 6 (Settings) will wrap this; for now callers construct it directly.
pub struct SweepConfig {
    pub archive_dir: PathBuf,
    pub backend: InferenceBackend,
    pub settings: HalluScribeSettings,
    pub ctx_size: u32,
    pub max_tokens: u32,
    pub min_fill_pct: f64,
    pub lookback_secs: u64,
    /// When true, this is a guest/import-only workspace: the sweep skips the
    /// host's local coding-tool scan and ingests only configured chat imports
    /// + recorded in-app chats.
    pub import_only: bool,
}

/// Emitted after each session is processed during a sweep.
#[derive(Debug, Clone, Serialize)]
pub struct SweepProgress {
    pub current: usize,
    pub total: usize,
    pub session_id: String,
    pub status: String,
}

/// Outcome of one sweep pass.
#[derive(Debug, Default)]
pub struct SweepResult {
    pub ran: bool,
    /// True when the sweep was refused because another inference job
    /// (a concurrent sweep, briefing, chat, or embedding run) held the
    /// process-wide inference lock. Distinct from a quiet out-of-window skip.
    pub busy: bool,
    /// The caller stopped the run before all eligible work completed.
    pub cancelled: bool,
    pub processed: u32,
    pub skipped: u32,
    /// Skipped Codex transcripts that contain only a greeting or an external
    /// capability probe after preprocessing, rather than archive-worthy work.
    pub low_signal_skipped: u32,
    /// Skipped phone chat sessions the owner never wrote in, or with fewer
    /// than three messages (`eligibility::is_low_signal_business_session`).
    pub business_filtered: u32,
    pub deferred: u32,
    /// Number of sessions written this sweep whose archived markdown matched
    /// at least one high-confidence secret shape (Phase 0b scan).
    pub flagged: u32,
    /// Number of chat-import sessions whose previous raw copy held different
    /// content and was moved to `raw/superseded/` rather than overwritten.
    /// Non-zero means a re-imported export changed conversations you had
    /// already archived — the earlier copies are still on disk.
    pub superseded: u32,
    pub errors: Vec<String>,
    /// The subset of `errors` that came from a business-messaging source or
    /// session (or affected every source). A business import runs the full
    /// sweep, so only these may be reported as the business import's error.
    pub business_errors: Vec<String>,
}

impl SweepResult {
    /// Record a sweep error; `business` also records it as a business error.
    pub fn push_error(&mut self, message: String, business: bool) {
        if business {
            self.business_errors.push(message.clone());
        }
        self.errors.push(message);
    }

    /// Daily success markers may advance only after a fully successful run.
    pub fn completed_successfully(&self) -> bool {
        self.ran && !self.busy && !self.cancelled && self.deferred == 0 && self.errors.is_empty()
    }
}
