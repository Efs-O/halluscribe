// HalluScribe - pending map facts: persist the map step's output after every
// successful batch so a failed/interrupted reduce never costs the mapping run.
// Stored at `<archive_dir>/profile/<scope>/pending_facts.json` and deleted
// once a refresh writes profile.md successfully.

use super::scope::ProfileScope;
use super::types::ProfileFact;
use super::writer::scope_dir;
use super::ProfileError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// On-disk snapshot of the map progress of one (possibly failed) refresh run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct PendingFacts {
    /// Ids of every session already mapped (their facts are in `facts`).
    pub session_ids: Vec<String>,
    /// All facts extracted so far, in map order.
    pub facts: Vec<ProfileFact>,
    /// Max session timestamp mapped so far; participates in the final
    /// watermark max when the resumed run completes.
    pub last_ts: String,
    /// RFC3339 time the snapshot was first/last written.
    pub created_at: String,
}

fn pending_path(archive_dir: &Path, scope: ProfileScope) -> PathBuf {
    scope_dir(archive_dir, scope).join("pending_facts.json")
}

/// Rewrite the pending file with the run's cumulative map progress.
pub(super) fn save_pending(
    archive_dir: &Path,
    scope: ProfileScope,
    pending: &PendingFacts,
) -> Result<(), ProfileError> {
    let dir = scope_dir(archive_dir, scope);
    fs::create_dir_all(&dir)?;
    fs::write(
        pending_path(archive_dir, scope),
        serde_json::to_string_pretty(pending)?,
    )?;
    Ok(())
}

/// Load the pending file if present. `Ok(None)` when absent; `Err(warning)`
/// when present but corrupt/unreadable — the caller treats that as absent
/// and records the warning, never crashes.
pub(super) fn load_pending(
    archive_dir: &Path,
    scope: ProfileScope,
) -> Result<Option<PendingFacts>, String> {
    let path = pending_path(archive_dir, scope);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("pending facts file unreadable, ignoring it: {error}"))?;
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|error| format!("pending facts file corrupt, ignoring it: {error}"))
}

/// Delete the pending file after a successful profile write. Missing file is
/// a no-op.
pub(super) fn clear_pending(archive_dir: &Path, scope: ProfileScope) {
    let _ = fs::remove_file(pending_path(archive_dir, scope));
}

/// Whether a pending file with at least one fact exists for `scope`. Used by
/// the refresh command alongside `pending_session_count` so a recovery run
/// with zero new sessions still loads the model and runs the reduce.
pub fn has_pending_facts(archive_dir: &Path, scope: ProfileScope) -> bool {
    matches!(
        load_pending(archive_dir, scope),
        Ok(Some(pending)) if !pending.facts.is_empty()
    )
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;
