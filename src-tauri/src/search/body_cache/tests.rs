// HalluScribe - tests for the session body cache. The cache is process-global,
// so these must not assume a cold start: each seeds its own archive directory,
// and cross-archive isolation is itself one of the properties under test.

use super::*;
use std::fs;

/// The cache is one process-global slot holding one archive, so its tests
/// serialise their cache transitions. Production invalidation is archive-scoped,
/// so unrelated archive writes do not evict this test's corpus.
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn serialised() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tmp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "halluscribe_body_cache_{}_{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("sessions")).unwrap();
    dir
}

/// Write `bodies` as session files and an index listing them.
fn seed(dir: &Path, bodies: &[(&str, &str)]) {
    let mut sessions = Vec::new();
    for (id, body) in bodies {
        let rel = format!("sessions/{id}.md");
        fs::write(dir.join(&rel), body).unwrap();
        sessions.push(serde_json::json!({
            "id": id,
            "project": "proj",
            "date": "2026-06-01",
            "title": format!("Session {id}"),
            "tool": "Claude Code",
            "fill_pct": 10.0,
            "session_timestamp": "2026-06-01T00:00:00+00:00",
            "updated_at": "",
            "session_type": "building",
            "error_tags": [],
            "topic_tags": [],
            "archive_path": rel,
            "source_jsonl": "",
            "source_size_bytes": 0,
            "provider": "claude_code",
            "fill_estimated": false,
            "transcript_hash": ""
        }));
    }
    fs::write(
        dir.join("index.json"),
        serde_json::to_string_pretty(&serde_json::json!({ "sessions": sessions })).unwrap(),
    )
    .unwrap();
}

fn contains(dir: &Path, rel: &str, needle: &str) -> Option<bool> {
    with_body(dir, rel, |body| body.contains(needle))
}

/// The body reaches the caller already lowercased, so a search never has to
/// lowercase it again - that is the whole point of caching it in this form.
#[test]
fn bodies_are_served_lowercased() {
    let _serialised = serialised();
    let dir = tmp_dir("lowercased");
    seed(&dir, &[("a", "Deploying QWEN with MTP")]);

    assert_eq!(contains(&dir, "sessions/a.md", "qwen with mtp"), Some(true));
    assert_eq!(contains(&dir, "sessions/a.md", "QWEN"), Some(false));
}

/// A session listed in the index whose file is missing is absent from the
/// map rather than cached as empty - callers turn that into "matches nothing",
/// which is what the uncached read did on an IO error.
#[test]
fn a_missing_body_is_absent_rather_than_empty() {
    let _serialised = serialised();
    let dir = tmp_dir("missing");
    seed(&dir, &[("a", "present")]);
    fs::remove_file(dir.join("sessions/a.md")).unwrap();

    assert_eq!(contains(&dir, "sessions/a.md", "present"), None);
}

/// The failure this cache could plausibly introduce: a redaction rewrites a
/// body and the removed text keeps matching. `invalidate(&dir)` is what the two
/// in-process body writers call to prevent it.
#[test]
fn invalidate_drops_a_rewritten_body() {
    let _serialised = serialised();
    let dir = tmp_dir("invalidate");
    seed(&dir, &[("a", "the secret token lives here")]);
    assert_eq!(contains(&dir, "sessions/a.md", "secret token"), Some(true));

    fs::write(dir.join("sessions/a.md"), "the [REDACTED] lives here").unwrap();
    invalidate(&dir);

    assert_eq!(contains(&dir, "sessions/a.md", "secret token"), Some(false));
    assert_eq!(contains(&dir, "sessions/a.md", "[redacted]"), Some(true));
}

/// A sweep run by another process changes index.json without bumping this
/// process's generation counter; the size+mtime stamp is what catches it.
#[test]
fn a_changed_index_rebuilds_without_invalidate() {
    let _serialised = serialised();
    let dir = tmp_dir("stamp");
    seed(&dir, &[("a", "first")]);
    assert_eq!(contains(&dir, "sessions/a.md", "first"), Some(true));
    assert_eq!(contains(&dir, "sessions/b.md", "second"), None);

    // A second session appears, index.json grows - and no invalidate() call.
    seed(&dir, &[("a", "first"), ("b", "second")]);
    assert_eq!(contains(&dir, "sessions/b.md", "second"), Some(true));
}

/// Two workspaces have their own archives and their own `sessions/a.md`.
/// One archive's bodies must never answer the other's search.
#[test]
fn switching_archives_does_not_serve_the_previous_one() {
    let _serialised = serialised();
    let host = tmp_dir("scope_host");
    let guest = tmp_dir("scope_guest");
    seed(&host, &[("a", "host only text")]);
    seed(&guest, &[("a", "guest only text")]);

    assert_eq!(contains(&host, "sessions/a.md", "host only"), Some(true));
    assert_eq!(contains(&guest, "sessions/a.md", "host only"), Some(false));
    assert_eq!(contains(&guest, "sessions/a.md", "guest only"), Some(true));
    // ...and back, to prove the rebuild is not one-way.
    assert_eq!(contains(&host, "sessions/a.md", "host only"), Some(true));
}

/// Repeated lookups against an unchanged archive must not re-read the files.
/// Deleting the corpus behind the cache's back is the only way to observe
/// that from outside: a still-answering lookup proves no disk read happened.
#[test]
fn a_warm_cache_does_not_touch_the_disk_again() {
    let _serialised = serialised();
    let dir = tmp_dir("warm");
    seed(&dir, &[("a", "cached body text")]);
    assert_eq!(contains(&dir, "sessions/a.md", "cached body"), Some(true));

    fs::remove_file(dir.join("sessions/a.md")).unwrap();

    assert_eq!(contains(&dir, "sessions/a.md", "cached body"), Some(true));
}
