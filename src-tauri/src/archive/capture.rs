// HalluScribe - startup raw capture (coding sources only).
//
// The sweep only preserves a raw transcript copy for sessions it summarises
// (`scheduler::runner::run_sweep`), so anything a coding tool prunes before
// its first sweep is lost forever. This pass runs once at app launch, reuses
// the sweep's own source discovery (`scanner::scan_sessions`), and preserves
// a raw copy of every coding-tool session file regardless of fill_pct or
// recency - unconditionally, with no settings gate. Chat imports (ChatGPT,
// Gemini, Grok, Claude.ai exports, Ollama's chat DB) are deliberately excluded:
// they are user-imported files or persistent DBs that don't get pruned out
// from under us, so their raws keep coming from the sweep as today.

use super::captured_manifest::{self, CapturedManifest, CapturedRecord};
use super::{
    ensure_index_readable, preserve_raw, raw_rel_path, read_sessions, session_id, set_raw_path,
};
use crate::scanner::{self, ScanTargetKind};
use crate::settings::HalluScribeSettings;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Progress/outcome of one capture pass, mirrored to the UI as managed Tauri
/// state and as the `raw-capture-progress` event payload.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CaptureStatus {
    #[default]
    Idle,
    Running {
        done: usize,
        total: usize,
        captured: usize,
        failed: usize,
    },
    Done {
        done: usize,
        total: usize,
        captured: usize,
    },
    Failed {
        done: usize,
        total: usize,
        captured: usize,
        errors: Vec<String>,
    },
    Cancelled,
}

fn mtime_secs(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |dur| dur.as_secs() as i64)
}

/// Run one capture pass. Reuses `scanner::scan_sessions` (the exact discovery
/// the sweep uses) with the widest possible window (`u64::MAX` lookback, 0.0
/// min fill) so capture sees every coding session on disk, then keeps only
/// `ScanTargetKind::Coding` targets - chat imports are scanned by the same
/// call but filtered out here rather than re-implemented.
///
/// `cancel` is polled between files (mirrors `SweepCancel`). `on_progress` is
/// called after every file so the caller can update managed state and emit a
/// Tauri event; it is also called once at the very end with the final status.
pub fn run_capture(
    archive_dir: &Path,
    settings: &HalluScribeSettings,
    import_only: bool,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(&CaptureStatus),
) -> CaptureStatus {
    if let Err(error) = ensure_index_readable(archive_dir) {
        let status = CaptureStatus::Failed {
            done: 0,
            total: 0,
            captured: 0,
            errors: vec![format!("archive index: {error}")],
        };
        on_progress(&status);
        return status;
    }
    let mut manifest: CapturedManifest = captured_manifest::load_captured(archive_dir);

    // Load the index ONCE for membership checks: `is_archived` re-parses the
    // whole index.json per call, which on a first launch (empty manifest,
    // ~thousands of session files) would mean thousands of full-index parses.
    // Membership is all the loop needs, and `set_raw_path` re-reads the index
    // itself, so a snapshot taken here never goes stale in a harmful way.
    let indexed_ids: std::collections::HashSet<String> = read_sessions(archive_dir)
        .into_iter()
        .map(|entry| entry.id)
        .collect();

    let targets: Vec<_> = scanner::scan_sessions(archive_dir, settings, u64::MAX, 0.0, import_only)
        .into_iter()
        .filter(|target| matches!(target.kind, ScanTargetKind::Coding(_)))
        .collect();

    let total = targets.len();
    let mut done = 0usize;
    let mut captured = 0usize;
    let mut errors = Vec::new();

    for target in &targets {
        if cancel.load(Ordering::Relaxed) {
            let status = CaptureStatus::Cancelled;
            on_progress(&status);
            return status;
        }

        done += 1;
        let id = session_id(&target.path);

        let Ok(meta) = std::fs::metadata(&target.path) else {
            // Source vanished between discovery and capture (pruned mid-scan) -
            // not an error, just nothing to preserve this pass.
            on_progress(&CaptureStatus::Running {
                done,
                total,
                captured,
                failed: errors.len(),
            });
            continue;
        };
        let size = meta.len();
        let mtime = meta.modified().map(mtime_secs).unwrap_or(0);

        let unchanged = manifest
            .get(&id)
            .is_some_and(|record| record.size == size && record.mtime_secs == mtime)
            || (indexed_ids.contains(&id) && archive_dir.join(raw_rel_path(&id)).is_file());

        if !unchanged {
            match preserve_raw(archive_dir, &id, &target.path) {
                Ok(rel) => {
                    if indexed_ids.contains(&id) {
                        if let Err(error) = set_raw_path(archive_dir, &id, rel) {
                            errors.push(format!("{id}: failed to update archive index: {error}"));
                        }
                    }
                    manifest.insert(
                        id,
                        CapturedRecord {
                            source_path: target.path.to_string_lossy().to_string(),
                            size,
                            mtime_secs: mtime,
                            captured_at: chrono::Utc::now().to_rfc3339(),
                        },
                    );
                    captured += 1;
                }
                Err(error) => {
                    errors.push(format!("{id}: failed to preserve raw transcript: {error}"))
                }
            }
        }

        on_progress(&CaptureStatus::Running {
            done,
            total,
            captured,
            failed: errors.len(),
        });
    }

    if let Err(error) = captured_manifest::save_captured(archive_dir, &manifest) {
        errors.push(format!("failed to save capture manifest: {error}"));
    }

    let status = if errors.is_empty() {
        CaptureStatus::Done {
            done,
            total,
            captured,
        }
    } else {
        CaptureStatus::Failed {
            done,
            total,
            captured,
            errors,
        }
    };
    on_progress(&status);
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("halluscribe_capture_test_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_claude_session(sources_root: &Path, project: &str, session_name: &str) -> String {
        let dir = sources_root.join(".claude").join("projects").join(project);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{session_name}.jsonl"));
        let line = serde_json::json!({
            "message": {
                "model": "claude-sonnet-4-5",
                "usage": {"input_tokens": 100, "output_tokens": 50}
            }
        });
        std::fs::write(&path, format!("{line}\n")).unwrap();
        session_id(&path)
    }

    /// Point the scanner at an isolated fake HOME so tests never touch the
    /// real user's `.claude/projects` directory. Serialized via a mutex since
    /// env vars are process-global and tests run on multiple threads.
    static HOME_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_fake_home<T>(home: &Path, f: impl FnOnce() -> T) -> T {
        let _guard = HOME_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let key = if cfg!(target_os = "windows") {
            "USERPROFILE"
        } else {
            "HOME"
        };
        let previous = std::env::var(key).ok();
        std::env::set_var(key, home);
        let result = f();
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
        result
    }

    #[test]
    fn captures_new_session_and_records_manifest_entry() {
        let archive_dir = tmp_dir("new_session_archive");
        let home = tmp_dir("new_session_home");
        let id = with_fake_home(&home, || {
            let id = write_claude_session(&home, "proj", "session-a");
            let settings = HalluScribeSettings::default();
            let cancel = AtomicBool::new(false);
            let status = run_capture(&archive_dir, &settings, false, &cancel, |_| {});
            assert!(matches!(status, CaptureStatus::Done { captured: 1, .. }));
            id
        });

        assert!(archive_dir.join(raw_rel_path(&id)).is_file());
        let manifest = captured_manifest::load_captured(&archive_dir);
        assert!(manifest.contains_key(&id));

        let _ = std::fs::remove_dir_all(&archive_dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn unchanged_size_and_mtime_is_skipped_on_second_pass() {
        let archive_dir = tmp_dir("skip_archive");
        let home = tmp_dir("skip_home");
        with_fake_home(&home, || {
            write_claude_session(&home, "proj", "session-b");
            let settings = HalluScribeSettings::default();

            let cancel = AtomicBool::new(false);
            let first = run_capture(&archive_dir, &settings, false, &cancel, |_| {});
            assert!(matches!(first, CaptureStatus::Done { captured: 1, .. }));

            let second = run_capture(&archive_dir, &settings, false, &cancel, |_| {});
            assert!(matches!(second, CaptureStatus::Done { captured: 0, .. }));
        });

        let _ = std::fs::remove_dir_all(&archive_dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn corrupt_index_is_reported_as_a_failed_capture() {
        let archive_dir = tmp_dir("corrupt_index");
        std::fs::write(archive_dir.join("index.json"), b"{ not valid json").unwrap();
        let settings = HalluScribeSettings::default();
        let cancel = AtomicBool::new(false);

        let status = run_capture(&archive_dir, &settings, false, &cancel, |_| {});

        assert!(matches!(
            status,
            CaptureStatus::Failed { ref errors, .. }
                if errors.iter().any(|error| error.contains("archive index"))
        ));
    }

    #[test]
    fn index_listed_session_gets_raw_path_set() {
        let archive_dir = tmp_dir("index_archive");
        let home = tmp_dir("index_home");
        with_fake_home(&home, || {
            let id = write_claude_session(&home, "proj", "session-c");
            let idx = serde_json::json!({
                "sessions": [{
                    "id": id,
                    "project": "proj",
                    "date": "2026-07-15",
                    "title": "Title",
                    "tool": "Claude Code",
                    "fill_pct": 60.0,
                    "session_type": "debugging",
                    "error_tags": [],
                    "topic_tags": [],
                    "archive_path": "sessions/proj/2026-07-15/session.md",
                    "source_jsonl": "unused.jsonl",
                    "raw_path": ""
                }]
            });
            std::fs::write(
                archive_dir.join("index.json"),
                serde_json::to_string_pretty(&idx).unwrap(),
            )
            .unwrap();

            let settings = HalluScribeSettings::default();
            let cancel = AtomicBool::new(false);
            run_capture(&archive_dir, &settings, false, &cancel, |_| {});

            let entry = super::super::find_session(&archive_dir, &id).unwrap();
            assert!(!entry.raw_path.is_empty());
            assert!(archive_dir.join(&entry.raw_path).is_file());
        });

        let _ = std::fs::remove_dir_all(&archive_dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn cancel_mid_pass_is_consistent_and_resumable() {
        let archive_dir = tmp_dir("cancel_archive");
        let home = tmp_dir("cancel_home");
        with_fake_home(&home, || {
            write_claude_session(&home, "proj", "session-d");
            write_claude_session(&home, "proj", "session-e");
            let settings = HalluScribeSettings::default();

            let cancel = AtomicBool::new(true);
            let status = run_capture(&archive_dir, &settings, false, &cancel, |_| {});
            assert!(matches!(status, CaptureStatus::Cancelled));

            // Nothing captured yet, so a resumed (non-cancelled) pass captures
            // everything - the cancelled pass left no partial state behind.
            let cancel = AtomicBool::new(false);
            let resumed = run_capture(&archive_dir, &settings, false, &cancel, |_| {});
            assert!(matches!(resumed, CaptureStatus::Done { captured: 2, .. }));
        });

        let _ = std::fs::remove_dir_all(&archive_dir);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn second_run_is_idempotent() {
        let archive_dir = tmp_dir("idempotent_archive");
        let home = tmp_dir("idempotent_home");
        with_fake_home(&home, || {
            write_claude_session(&home, "proj", "session-f");
            let settings = HalluScribeSettings::default();
            let cancel = AtomicBool::new(false);

            run_capture(&archive_dir, &settings, false, &cancel, |_| {});
            let manifest_after_first = captured_manifest::load_captured(&archive_dir);

            run_capture(&archive_dir, &settings, false, &cancel, |_| {});
            let manifest_after_second = captured_manifest::load_captured(&archive_dir);

            assert_eq!(manifest_after_first.len(), manifest_after_second.len());
        });

        let _ = std::fs::remove_dir_all(&archive_dir);
        let _ = std::fs::remove_dir_all(&home);
    }
}
