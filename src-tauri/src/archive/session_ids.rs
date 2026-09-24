// HalluScribe - session id proposal and collision-safe, stable id resolution.

use super::IndexEntry;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

pub fn session_id(source: &Path) -> String {
    if let Some(stem) = source.file_stem().and_then(|s| s.to_str()) {
        if !stem.is_empty() {
            return stem.to_string();
        }
    }
    // A path with no usable stem (trailing dot, non-UTF-8 filename, a path
    // ending in `..`) must not collapse onto a single literal id: `append_index`
    // does `retain(|e| e.id != entry.id)` before pushing, so every such session
    // would evict the previous one. Fall back to a stable per-path hash so the
    // ids stay distinct and idempotent.
    format!("unknown-{}", path_hash(source))
}

/// Resolve a proposed session ID against durable archive ownership. A legacy
/// bare stem remains its existing owner's ID; a different source requesting
/// that stem receives a hashed suffix. A source that already holds a hashed id
/// keeps it, so an existing session is never re-identified, whatever hash
/// produced its suffix.
pub fn resolve_session_id(archive_dir: &Path, source: &Path, proposed: &str) -> String {
    SessionLookup::load(archive_dir).resolve_session_id(source, proposed)
}

/// The archive's ownership data (the index and the captured manifest), read
/// once. The sweep checks every discovered session against it; re-reading and
/// re-parsing `index.json` per session made a 6,574-session phone import spend
/// many minutes before inference started, and every later sweep paid it again.
/// Nothing is written while the worklist is built, so one snapshot is exact.
pub struct SessionLookup {
    entries: HashMap<String, IndexEntry>,
    captured: super::CapturedManifest,
    deleted: BTreeMap<String, super::Tombstone>,
    /// Every id each source already holds, sorted, across all three records.
    ids_by_source: HashMap<String, Vec<String>>,
}

impl SessionLookup {
    /// An unreadable index reads as empty, as `resolve_session_id` always did;
    /// the sweep checks `ensure_index_readable` before building its worklist.
    pub fn load(archive_dir: &Path) -> Self {
        let mut entries = HashMap::new();
        if let Ok(index) = super::index::load_index(archive_dir) {
            for entry in index.sessions {
                // The first row wins, matching the old linear `find`.
                entries.entry(entry.id.clone()).or_insert(entry);
            }
        }
        Self::new(
            entries,
            super::captured_manifest::load_captured(archive_dir),
            // Unreadable records read as none here; the sweep and capture
            // refuse to start on them via `ensure_deleted_readable`.
            super::tombstones::load_deleted(archive_dir).unwrap_or_default(),
        )
    }

    fn new(
        entries: HashMap<String, IndexEntry>,
        captured: super::CapturedManifest,
        deleted: BTreeMap<String, super::Tombstone>,
    ) -> Self {
        let owned = entries
            .values()
            .map(|entry| (&entry.id, &entry.source_jsonl))
            .chain(
                captured
                    .iter()
                    .map(|(id, record)| (id, &record.source_path)),
            )
            .chain(deleted.iter().map(|(id, stone)| (id, &stone.source_path)));
        let mut ids_by_source: HashMap<String, Vec<String>> = HashMap::new();
        for (id, source) in owned.filter(|(_, source)| !source.is_empty()) {
            ids_by_source
                .entry(source.clone())
                .or_default()
                .push(id.clone());
        }
        for ids in ids_by_source.values_mut() {
            ids.sort();
            ids.dedup();
        }
        Self {
            entries,
            captured,
            deleted,
            ids_by_source,
        }
    }

    /// Whether the user permanently deleted the session `id` read from `source`.
    pub fn is_deleted(&self, id: &str, source: &Path) -> bool {
        self.deleted
            .get(id)
            .is_some_and(|tombstone| tombstone.covers(source))
    }

    /// The index row for `id`, if archived.
    pub fn find(&self, id: &str) -> Option<&IndexEntry> {
        self.entries.get(id)
    }

    /// See the free function `resolve_session_id`.
    pub fn resolve_session_id(&self, source: &Path, proposed: &str) -> String {
        let source_text = source.to_string_lossy();
        let index_owner = self
            .entries
            .get(proposed)
            .map(|entry| entry.source_jsonl.as_str());
        let manifest_owner = self
            .captured
            .get(proposed)
            .map(|record| record.source_path.as_str());
        // A deleted session still owns its id, so another source proposing
        // the same id gets its own instead of inheriting the delete.
        let deleted_owner = self
            .deleted
            .get(proposed)
            .map(|tombstone| tombstone.source_path.as_str())
            .filter(|owner| !owner.is_empty());
        let owners = [index_owner, manifest_owner, deleted_owner];
        if owners.iter().flatten().any(|owner| *owner == source_text) {
            return proposed.to_string();
        }
        if let Some(stored) = self.stored_hashed_id(&source_text, proposed) {
            return stored;
        }
        if owners.iter().flatten().next().is_some() {
            format!("{proposed}-{}", path_hash(source))
        } else {
            proposed.to_string()
        }
    }

    /// The hashed id `source` already holds for `proposed`: `{proposed}-{hash}`,
    /// or for a stemless `unknown-{hash}` proposal any `unknown-{hash}`. These
    /// hashes were computed by an earlier build, possibly with a different
    /// algorithm, so the stored id is the authority rather than a recomputed one.
    fn stored_hashed_id(&self, source: &str, proposed: &str) -> Option<String> {
        let proposed_unknown = proposed.strip_prefix("unknown-").is_some_and(is_path_hash);
        self.ids_by_source.get(source)?.iter().find_map(|id| {
            let suffixed = id
                .strip_prefix(proposed)
                .and_then(|rest| rest.strip_prefix('-'))
                .is_some_and(is_path_hash);
            let unknown = proposed_unknown && id.strip_prefix("unknown-").is_some_and(is_path_hash);
            (suffixed || unknown).then(|| id.clone())
        })
    }
}

fn is_path_hash(text: &str) -> bool {
    text.len() == 16 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// A 64-bit FNV-1a hash of the path's bytes. Written out rather than taken from
/// `DefaultHasher`, whose algorithm Rust may change between releases, so a new
/// toolchain cannot give the same source a different id.
fn path_hash(source: &Path) -> String {
    let hash = source
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    format!("{hash:016x}")
}

#[cfg(test)]
pub(super) fn lookup_for_tests(
    entries: Vec<IndexEntry>,
    captured: super::CapturedManifest,
    deleted: BTreeMap<String, super::Tombstone>,
) -> SessionLookup {
    SessionLookup::new(
        entries
            .into_iter()
            .map(|entry| (entry.id.clone(), entry))
            .collect(),
        captured,
        deleted,
    )
}
