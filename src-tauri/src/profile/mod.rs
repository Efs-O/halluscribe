// HalluScribe - profile distiller: map-reduce over the consented archive into
// `<archive_dir>/profile/profile.md` + `profile_meta.json` + a weekly digest.
// See docs/internal/PERSONA_PROTOCOL_PLAN.md Phase 2 for the full design.

mod distill;
mod merge;
mod parse_md;
mod pending;
mod scope;
mod select;
mod types;
mod writer;

pub use distill::ToolCallFn;
pub use pending::has_pending_facts;
pub use scope::{sources_for_scope, ProfileScope};
pub use select::{select_sources, BATCH_SIZE};
pub use types::{ProfileFact, ProfileMeta, ProfileSection, ProfileSections};
pub use writer::{
    all_digests, latest_digest, load_meta, migrate_legacy_profile_layout, read_profile_md,
};

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

    // A pending file only ever exists after a failed run: reuse its already
    // mapped facts and skip re-mapping those sessions (correct recovery for
    // both incremental and full runs). Corrupt file: warn and treat as absent.
    let mut errors = Vec::new();
    let resumed = match pending::load_pending(archive_dir, scope) {
        Ok(Some(resumed)) => resumed,
        Ok(None) => pending::PendingFacts::default(),
        Err(warning) => {
            errors.push(warning);
            pending::PendingFacts::default()
        }
    };

    // Nothing new to distill and nothing to recover: leave profile.md, the
    // meta, and the digests untouched rather than re-merging the old profile
    // against zero facts.
    if selected.is_empty() && resumed.facts.is_empty() {
        return Ok(RefreshOutcome {
            errors,
            ..Default::default()
        });
    }

    let to_map: Vec<&IndexEntry> = selected
        .iter()
        .filter(|entry| !resumed.session_ids.contains(&entry.id))
        .copied()
        .collect();
    // Distinct sessions this run folds in: newly mapped plus recovered ones.
    let covered_count = selected
        .iter()
        .map(|entry| entry.id.as_str())
        .chain(resumed.session_ids.iter().map(String::as_str))
        .collect::<std::collections::HashSet<&str>>()
        .len();
    let batches = select::chunk_batches(&to_map, select::BATCH_SIZE);
    // Progress totals count only the batches actually being mapped this run.
    let total_batches = batches.len().max(1);

    let mut snapshot = pending::PendingFacts {
        created_at: Utc::now().to_rfc3339(),
        ..resumed
    };
    // Only a whole failed map batch leaves sessions undistilled; the warnings
    // pushed into `errors` (skipped facts, fallbacks) must not be confused
    // with it, or a benign warning would hold the watermark back forever.
    let mut failed_batches = 0usize;
    for (idx, batch) in batches.iter().enumerate() {
        on_progress(idx + 1, total_batches, Stage::Mapping);
        match distill::distill_batch(archive_dir, batch, scope, tool_call) {
            Ok((mut facts, warnings)) => {
                for warning in warnings {
                    errors.push(format!("batch {}: {warning}", idx + 1));
                }
                snapshot.facts.append(&mut facts);
                for entry in batch {
                    snapshot.session_ids.push(entry.id.clone());
                    let ts = entry_timestamp(entry);
                    if ts > snapshot.last_ts.as_str() {
                        snapshot.last_ts = ts.to_string();
                    }
                }
                // Persist after every successful batch so a failed reduce (or
                // an interrupted run) never costs the mapping work done so far.
                if let Err(error) = pending::save_pending(archive_dir, scope, &snapshot) {
                    errors.push(format!("failed to save pending facts: {error}"));
                }
            }
            Err(error) => {
                failed_batches += 1;
                errors.push(error.to_string());
            }
        }
    }
    let all_facts = snapshot.facts;

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
                session_count: covered_count,
                facts_count: all_facts.len(),
                errors,
            });
        }
    };

    on_progress(1, 1, Stage::Writing);
    // The recovered snapshot's last_ts participates in the watermark max, so
    // sessions mapped by an earlier failed run still advance the watermark.
    let newest_selected = newest_timestamp(&selected).unwrap_or("");
    let newest_ts = if snapshot.last_ts.as_str() > newest_selected {
        snapshot.last_ts.as_str()
    } else {
        newest_selected
    };
    let new_meta = ProfileMeta {
        generated_at: Utc::now().to_rfc3339(),
        // A failed map batch left some selected sessions undistilled; keeping
        // the old watermark makes the next incremental run retry them instead
        // of skipping them forever. Non-fatal warnings in `errors` (skipped
        // facts, merge fallbacks) left nothing undistilled and must advance it.
        last_distilled_ts: if failed_batches > 0 || newest_ts.is_empty() {
            meta.last_distilled_ts.clone()
        } else {
            newest_ts.to_string()
        },
        session_count: covered_count,
        sources: effective_sources,
        facts_count: all_facts.len(),
    };
    writer::write_profile(archive_dir, scope, &sections, &new_meta)?;
    writer::write_digest(archive_dir, scope, &selected, &all_facts, Utc::now())?;
    // The mapping run is safely folded into the written profile: the pending
    // snapshot has served its purpose. When a batch failed the watermark was
    // held back, so keep the snapshot too — the retry run then re-maps only
    // the failed sessions instead of everything since the old watermark.
    if failed_batches == 0 {
        pending::clear_pending(archive_dir, scope);
    }

    Ok(RefreshOutcome {
        session_count: covered_count,
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

/// The timestamp used for watermark bookkeeping: `session_timestamp`
/// (RFC3339, sortable lexicographically) with a fallback to the coarser
/// `date` field for legacy entries that predate that column.
fn entry_timestamp(entry: &IndexEntry) -> &str {
    if !entry.session_timestamp.is_empty() {
        entry.session_timestamp.as_str()
    } else {
        entry.date.as_str()
    }
}

fn newest_timestamp<'a>(selected: &[&'a IndexEntry]) -> Option<&'a str> {
    selected.first().map(|entry| entry_timestamp(entry))
}

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod refresh_tests;

#[cfg(test)]
#[path = "refresh_scope_tests.rs"]
mod refresh_scope_tests;

#[cfg(test)]
#[path = "refresh_pending_tests.rs"]
mod refresh_pending_tests;
