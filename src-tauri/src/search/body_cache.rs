// HalluScribe - in-memory cache of lowercased session `.md` bodies.
//
// Full-text search reads a session body only for the terms that metadata could
// not answer, but on a 2,000+ session archive that is still ~2,000 individual
// file opens per search. The bytes are trivial (the whole corpus is under
// 10 MB); the cost is the syscalls, which on Windows each go through the
// virus scanner. So the corpus is read once and held lowercased in memory.
//
// Staleness is caught two ways, because there are two ways bodies change:
//   - `invalidate()`, called by the two places in this process that write a
//     session body (`archive::writer` on sweep, `archive::redact` on redact);
//   - the `index.json` size+mtime stamp, which catches a sweep run by another
//     process (the MCP server, a second window) without a per-file `stat`.
// Either one changing drops the whole map; it is rebuilt on the next search.

use crate::archive::{index_stamp, read_sessions, IndexStamp};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Bumped by every in-process write of a session body. Part of the cache key,
/// so a write during a search can never be papered over by a same-second
/// mtime: the generation differs regardless of the clock.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Drop the cached bodies. Call after writing or rewriting any session `.md`.
pub fn invalidate() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

struct Cached {
    archive_dir: PathBuf,
    stamp: IndexStamp,
    generation: u64,
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
            bodies.insert(entry.archive_path, markdown.to_lowercase());
        }
    }
    bodies
}

/// Run `test` against one session's lowercased body, loading the corpus first
/// if the cache is cold or stale. `None` when that session has no readable
/// body. Switching workspace archives rebuilds rather than merging: one
/// archive's bodies must never answer another's search.
pub(super) fn with_body<R>(
    archive_dir: &Path,
    archive_path: &str,
    test: impl FnOnce(&str) -> R,
) -> Option<R> {
    let stamp = index_stamp(archive_dir);
    let generation = GENERATION.load(Ordering::SeqCst);
    let mut guard = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let fresh = guard.as_ref().is_some_and(|cached| {
        cached.archive_dir == archive_dir
            && cached.stamp == stamp
            && cached.generation == generation
    });
    if !fresh {
        *guard = Some(Cached {
            archive_dir: archive_dir.to_path_buf(),
            stamp,
            generation,
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
