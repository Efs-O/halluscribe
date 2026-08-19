// HalluScribe - one-time raw transcript backfill (Persona Parity Phase A).
//
// Raw preservation (`archive::raw`) only ever runs during a sweep, so the
// ~1300 sessions archived before raw preservation existed have no
// raw copy. This is a non-destructive, no-inference recovery pass: for every
// already-archived session missing a raw copy, if its original source file can
// still be located, compress and record it exactly as the sweep would have.
// Sessions whose source is gone are counted, never treated as an error - the
// backfill always completes.

use super::ArchiveError;
use crate::settings::HalluScribeSettings;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Per-session raw slices for one source file, or `None` when that source maps
/// 1:1 to a session and the whole file is the correct raw.
type SourceSlices = Option<HashMap<String, String>>;

/// Outcome of one backfill pass, returned to the UI.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct BackfillResult {
    pub recovered: usize,
    pub already_had: usize,
    pub source_missing: usize,
    /// Sessions that HAD a raw, but the wrong one: archives written before
    /// per-session slicing stored the whole export for every conversation in
    /// it. Replaced in place with the correct slice.
    pub repaired: usize,
    pub total: usize,
}

/// Recover raw transcripts for archived sessions that don't have one yet,
/// using the active archive's own settings to locate user-owned imports.
pub fn backfill_raw(archive_dir: &Path) -> Result<BackfillResult, ArchiveError> {
    super::ensure_index_readable(archive_dir)?;
    let settings = crate::settings::load_settings(archive_dir)
        .map_err(|error| ArchiveError::Invalid(error.to_string()))?;
    backfill_raw_with(archive_dir, &settings)
}

/// Where a session's source file is *now*.
///
/// Ownership decides, per docs/internal/IMPORT_PATHS_PLAN.md § 2: user-owned
/// chat imports are always located through the configured import path, and
/// tool-owned coding sources always through the absolute path recorded when
/// the session was archived. There is deliberately no second attempt - a
/// source its owner cannot produce is simply gone, and reported as such.
///
/// For chat imports the recorded path still decides *which* file: its file name
/// is matched against the import folder's contents, so a split export resolves
/// to its own chunk rather than to whichever chunk sorts first.
fn resolve_source(settings: &HalluScribeSettings, entry: &super::IndexEntry) -> Option<PathBuf> {
    if crate::readers::is_multi_session_provider(&entry.provider) {
        let recorded_name = Path::new(&entry.source_jsonl).file_name()?;
        return crate::scanner::chat_import_sources(settings, &entry.provider)
            .into_iter()
            .find(|candidate| candidate.file_name() == Some(recorded_name));
    }
    let recorded = PathBuf::from(&entry.source_jsonl);
    recorded.is_file().then_some(recorded)
}

/// Never aborts on a single session's missing/unreadable source - that session
/// is simply counted under `source_missing` and the pass continues.
pub(crate) fn backfill_raw_with(
    archive_dir: &Path,
    settings: &HalluScribeSettings,
) -> Result<BackfillResult, ArchiveError> {
    let entries = super::read_sessions(archive_dir);
    let mut result = BackfillResult {
        total: entries.len(),
        ..Default::default()
    };

    // One parse per multi-session source file, shared by every session that
    // came out of it: an 81-conversation export is read once, not 81 times.
    // Keyed on the RESOLVED path, so entries recording different stale paths
    // that now resolve to the same file share one parse.
    let mut slice_cache: HashMap<(PathBuf, String), SourceSlices> = HashMap::new();

    for entry in entries {
        let has_raw = !entry.raw_path.is_empty() && archive_dir.join(&entry.raw_path).is_file();
        let multi_session = crate::readers::is_multi_session_provider(&entry.provider);

        // A raw stored for a 1:1 source is correct by construction. Only
        // multi-session sources need their existing raw re-checked, because
        // pre-slicing builds stored the whole export under every session id.
        if has_raw && !multi_session {
            result.already_had += 1;
            continue;
        }

        let Some(source) = resolve_source(settings, &entry) else {
            // Without the source there is nothing to compare against or
            // rebuild from; a wrong-but-present raw is left untouched rather
            // than discarded.
            if has_raw {
                result.already_had += 1;
            } else {
                result.source_missing += 1;
            }
            continue;
        };

        let slices = slice_cache
            .entry((source.clone(), entry.provider.clone()))
            .or_insert_with(|| crate::readers::raw_slices_for_source(&source, &entry.provider));

        let preserved = match slices {
            // Multi-session source: only this session's own slice may be
            // preserved. An id absent from the current export (conversation
            // deleted upstream, or the export failed to parse) has nothing to
            // recover — never copy the whole file here.
            Some(by_id) => match by_id.get(&entry.id) {
                Some(slice) => {
                    // Already correct: leave the stored bytes alone.
                    if has_raw && stored_raw_matches(archive_dir, &entry, slice) {
                        result.already_had += 1;
                        continue;
                    }
                    // `repaired` below is exactly the count of raws replaced,
                    // and every replacement parks the previous copy under
                    // `raw/superseded/` — nothing is discarded here.
                    super::preserve_raw_bytes(archive_dir, &entry.id, slice.as_bytes())
                        .map(|preserved| preserved.rel)
                }
                None => {
                    if has_raw {
                        result.already_had += 1;
                    } else {
                        result.source_missing += 1;
                    }
                    continue;
                }
            },
            None => super::preserve_raw(archive_dir, &entry.id, &source),
        };

        match preserved {
            Ok(rel) => {
                super::set_raw_path(archive_dir, &entry.id, rel)?;
                if has_raw {
                    result.repaired += 1;
                } else {
                    result.recovered += 1;
                }
            }
            Err(_) => result.source_missing += 1,
        }
    }

    Ok(result)
}

/// Whether the raw already on disk for `entry` is byte-identical to `slice`.
/// An unreadable stored raw counts as a mismatch, so the repair rewrites it.
fn stored_raw_matches(archive_dir: &Path, entry: &super::IndexEntry, slice: &str) -> bool {
    super::read_raw_at(archive_dir, &entry.raw_path)
        .map(|stored| stored == slice)
        .unwrap_or(false)
}
