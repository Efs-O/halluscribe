// HalluScribe - in-memory cache of lowercased session `.md` bodies.
//
// Full-text search reads a session body only for the terms that metadata could
// not answer, but on a 2,000+ session archive that is still ~2,000 individual
// file opens per search. The bytes are trivial (the whole corpus is under
// 10 MB); the cost is the syscalls, which on Windows each go through the
// virus scanner. So the corpus is read once and held lowercased in memory.
//
// Staleness is caught two ways, because there are two ways bodies change:
//   - `invalidate(archive_dir)`, called by the two places in this process that write a
//     session body (`archive::writer` on sweep, `archive::redact` on redact);
//   - the `index.json` size+mtime stamp, which catches a sweep run by another
//     process (the MCP server, a second window) without a per-file `stat`.
// Either one changing drops the whole map; it is rebuilt on the next search.

use super::fold::fold_for_search;
use crate::archive::{index_stamp, read_sessions, IndexStamp};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Drop cached bodies for `archive_dir`. A write in another workspace cannot
/// affect this cache's contents, so it must not evict them. The mutex makes an
/// in-flight lookup and its writer atomic with respect to one another: either
/// the lookup finishes before the write, or the next lookup rebuilds.
pub fn invalidate(archive_dir: &Path) {
    let mut guard = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard
        .as_ref()
        .is_some_and(|cached| cached.archive_dir == archive_dir)
    {
        *guard = None;
    }
}

struct Cached {
    archive_dir: PathBuf,
    stamp: IndexStamp,
    /// Keyed by `IndexEntry::archive_path` (index-relative, forward slashes),
    /// holding the body already lowercased so a search never re-lowercases.
    bodies: HashMap<String, String>,
}

fn cache() -> &'static Mutex<Option<Cached>> {
    static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Read every session body listed in the index, lowercased. Sessions whose
/// file is missing or unreadable are simply absent - callers treat an absent
/// body as "matches nothing", which is what the uncached read did on error.
fn load(archive_dir: &Path) -> HashMap<String, String> {
    let mut bodies = HashMap::new();
    for entry in read_sessions(archive_dir) {
        if bodies.contains_key(&entry.archive_path) {
            continue;
        }
        if let Ok(markdown) = std::fs::read_to_string(archive_dir.join(&entry.archive_path)) {
            bodies.insert(entry.archive_path, fold_for_search(&markdown));
        }
    }
    bodies
}

/// Run `test` against one session's lowercased body, loading the corpus first
/// if the cache is cold or stale. `None` when that session has no readable
/// body. Switching workspace archives rebuilds rather than merging: one
/// archive's bodies must never answer another's search.
#[cfg(test)]
pub(super) fn with_body<R>(
    archive_dir: &Path,
    archive_path: &str,
    test: impl FnOnce(&str) -> R,
) -> Option<R> {
    let stamp = index_stamp(archive_dir);
    with_body_for_stamp(archive_dir, archive_path, &stamp, test)
}

/// Return one content stamp for an entire search pass. Callers that inspect N
/// entries pass this to `with_body_for_stamp`, amortising a full index read to
/// one read rather than N reads.
pub(super) fn stamp(archive_dir: &Path) -> IndexStamp {
    index_stamp(archive_dir)
}

pub(super) fn with_body_for_stamp<R>(
    archive_dir: &Path,
    archive_path: &str,
    stamp: &IndexStamp,
    test: impl FnOnce(&str) -> R,
) -> Option<R> {
    let mut guard = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let fresh = guard.as_ref().is_some_and(|cached| {
        stamp.is_some() && cached.archive_dir == archive_dir && cached.stamp == *stamp
    });
    if !fresh {
        *guard = Some(Cached {
            archive_dir: archive_dir.to_path_buf(),
            stamp: stamp.clone(),
            bodies: load(archive_dir),
        });
    }

    guard
        .as_ref()
        .and_then(|cached| cached.bodies.get(archive_path))
        .map(|body| test(body))
}

#[cfg(test)]
mod tests;
