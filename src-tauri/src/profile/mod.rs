// HalluScribe - profile distiller: map-reduce over the consented archive into
// `<archive_dir>/profile/profile.md` + `profile_meta.json` + a weekly digest.
// See docs/internal/PERSONA_PROTOCOL_PLAN.md Phase 2 for the full design.

mod distill;
mod merge;
mod scope;
mod select;
mod types;
mod writer;

pub use distill::ToolCallFn;
pub use scope::{sources_for_scope, ProfileScope};
pub use select::{select_sources, BATCH_SIZE};
pub use types::{ProfileFact, ProfileMeta, ProfileSection, ProfileSections};
pub use writer::{latest_digest, load_meta, migrate_legacy_profile_layout, read_profile_md};

use crate::archive::{self, IndexEntry};
use crate::gemma::GemmaError;
use chrono::Utc;
use std::fmt;
use std::path::Path;

#[derive(Debug)]
pub enum ProfileError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Gemma(GemmaError),
    BadToolCall(String),
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Json(error) => write!(f, "JSON error: {error}"),
            Self::Gemma(error) => write!(f, "inference error: {error}"),
            Self::BadToolCall(msg) => write!(f, "tool-call response invalid: {msg}"),
        }
    }
}

impl From<std::io::Error> for ProfileError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ProfileError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<GemmaError> for ProfileError {
    fn from(error: GemmaError) -> Self {
        Self::Gemma(error)
    }
}

/// Progress stages reported during a refresh, mirrored to the frontend as the
/// `profile-progress` event's `stage` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Mapping,
    Merging,
    Writing,
}

impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mapping => "mapping",
            Self::Merging => "merging",
            Self::Writing => "writing",
        }
    }
}

/// Outcome of one `run_refresh` call.
#[derive(Debug, Default)]
pub struct RefreshOutcome {
    pub session_count: usize,
    pub facts_count: usize,
    pub errors: Vec<String>,
}

/// Run one profile refresh for `scope`: select consented (and, if
/// incremental, unwatermarked) sessions, map them into facts, reduce facts
/// into profile sections, then write `profile/<scope>/profile.md`,
/// `profile_meta.json`, and this week's digest.
///
/// `profile_sources` is the user's raw `settings.profile_sources` list; the
/// scope's effective source list (Personal adds the chat-export providers —
/// see `scope::sources_for_scope`) is derived here and recorded in
/// `profile_meta.json`'s `sources` field.
///
/// `tool_call` is the caller's inference entry point (wraps a warm
/// `gemma::ToolSession` in production, a canned closure in tests) so this
/// function never touches the model lifecycle itself. `on_progress` is called
/// at each map batch and once per reduce/write stage.
pub fn run_refresh(
    archive_dir: &Path,
    scope: ProfileScope,
    profile_sources: &[String],
    full: bool,
    tool_call: &ToolCallFn,
    mut on_progress: impl FnMut(usize, usize, Stage),
) -> Result<RefreshOutcome, ProfileError> {
    let effective_sources = scope::sources_for_scope(profile_sources, scope);
    let meta = writer::load_meta(archive_dir, scope);
    let watermark = if full || meta.last_distilled_ts.is_empty() {
        None
    } else {
        Some(meta.last_distilled_ts.as_str())
    };

    let entries = archive::read_sessions(archive_dir);
    let selected = select::select_sources(&entries, &effective_sources, watermark);
    // Nothing new to distill: leave profile.md, the meta, and the digests
    // untouched rather than re-merging the old profile against zero facts.
    if selected.is_empty() {
        return Ok(RefreshOutcome::default());
    }
    let batches = select::chunk_batches(&selected, select::BATCH_SIZE);
    let total_batches = batches.len().max(1);

    let mut all_facts = Vec::new();
    let mut errors = Vec::new();
    for (idx, batch) in batches.iter().enumerate() {
        on_progress(idx + 1, total_batches, Stage::Mapping);
        match distill::distill_batch(archive_dir, batch, scope, tool_call) {
            Ok(mut facts) => all_facts.append(&mut facts),
            Err(error) => errors.push(error.to_string()),
        }
    }

    on_progress(1, 1, Stage::Merging);
    let previous_md = writer::read_profile_md(archive_dir, scope);
    // A failed merge must not discard the outcome (and with it the collected
    // map-batch errors): record it, skip the write, and keep the watermark so
    // the next run retries — the UI then shows every error, not a bare abort.
    let sections = match merge::run_reduce(
        scope,
        previous_md.as_deref(),
        &all_facts,
        tool_call,
        &mut errors,
    ) {
        Ok(sections) => sections,
        Err(error) => {
            errors.push(format!("final merge failed: {error}"));
            return Ok(RefreshOutcome {
                session_count: selected.len(),
                facts_count: all_facts.len(),
                errors,
            });
        }
    };

    on_progress(1, 1, Stage::Writing);
    let newest_ts = newest_timestamp(&selected).unwrap_or(&meta.last_distilled_ts);
    let new_meta = ProfileMeta {
        generated_at: Utc::now().to_rfc3339(),
        // A failed map batch left some selected sessions undistilled; keeping
        // the old watermark makes the next incremental run retry them instead
        // of skipping them forever.
        last_distilled_ts: if !errors.is_empty() || newest_ts.is_empty() {
            meta.last_distilled_ts.clone()
        } else {
            newest_ts.to_string()
        },
        session_count: selected.len(),
        sources: effective_sources,
        facts_count: all_facts.len(),
    };
    writer::write_profile(archive_dir, scope, &sections, &new_meta)?;
    writer::write_digest(archive_dir, scope, &selected, &all_facts, Utc::now())?;

    Ok(RefreshOutcome {
        session_count: selected.len(),
        facts_count: all_facts.len(),
        errors,
    })
}

/// How many sessions the next refresh of `scope` would distill. Lets the
/// command skip loading a model at all when an incremental run has nothing
/// new.
pub fn pending_session_count(
    archive_dir: &Path,
    scope: ProfileScope,
    profile_sources: &[String],
    full: bool,
) -> usize {
    let effective_sources = scope::sources_for_scope(profile_sources, scope);
    let meta = writer::load_meta(archive_dir, scope);
    let watermark = if full || meta.last_distilled_ts.is_empty() {
        None
    } else {
        Some(meta.last_distilled_ts.as_str())
    };
    let entries = archive::read_sessions(archive_dir);
    select::select_sources(&entries, &effective_sources, watermark).len()
}

fn newest_timestamp<'a>(selected: &[&'a IndexEntry]) -> Option<&'a str> {
    selected.first().map(|entry| {
        if !entry.session_timestamp.is_empty() {
            entry.session_timestamp.as_str()
        } else {
            entry.date.as_str()
        }
    })
}

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod refresh_tests;

#[cfg(test)]
#[path = "refresh_scope_tests.rs"]
mod refresh_scope_tests;
