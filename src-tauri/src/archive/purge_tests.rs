// HalluScribe - tests for purging a session's stale summaries and private files.

use super::purge::summary_owner;
use super::{delete_sessions, raw_rel_path, read_sessions, write_session, SessionMeta};
use crate::gemma::{GemmaOutput, SessionType};
use chrono::{DateTime, TimeZone, Utc};
use std::fs;
use std::path::{Path, PathBuf};

fn tmp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "halluscribe_purge_test_{}_{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 15, hour, minute, 0).unwrap()
}

fn meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        source: PathBuf::from(format!("/fake/{id}.jsonl")),
        project: "proj".into(),
        tool: "Claude Code".into(),
        provider: "claude_code".into(),
        fill_pct: 50.0,
        fill_estimated: false,
        output_tokens: 0,
        tokens_estimated: true,
        backend: "llama.cpp".into(),
        model: "model.gguf".into(),
        session_timestamp: at(9, 0),
        updated_at: None,
        transcript_hash: "hash".into(),
        raw_path: None,
    }
}

fn output() -> GemmaOutput {
    GemmaOutput {
        title: "Title".into(),
        summary: "Summary.".into(),
        session_type: SessionType::Building,
        error_tags: vec![],
        topic_tags: vec![],
        verbatim_highlights: vec![],
    }
}

/// Every file under `sessions/`, as names.
fn summary_files(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending = vec![dir.join("sessions")];
    while let Some(next) = pending.pop() {
        let Ok(entries) = fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    names.sort();
    names
}

#[test]
fn summary_owner_reads_the_id_from_summary_and_backup_names() {
    assert_eq!(
        summary_owner("14-30-00-claudecode-abc-uuid-sweep.md"),
        Some("abc-uuid")
    );
    assert_eq!(
        summary_owner("14-30-00-whatsapp_business-a-b-sweep.md.bak-20260415143000"),
        Some("a-b")
    );
    assert_eq!(summary_owner("notes.md"), None);
    assert_eq!(summary_owner("14-30-00-claudecode--sweep.md"), None);
    assert_eq!(summary_owner("1a-30-00-claudecode-abc-sweep.md"), None);
    assert_eq!(summary_owner("14-30-00-claudecode-abc-sweep.txt"), None);
}

#[test]
fn a_resweep_under_a_new_name_removes_the_previous_summary() {
    let dir = tmp_dir("resweep");
    let first = write_session(&dir, &meta("abc"), &output(), at(10, 0)).unwrap();
    let second = write_session(&dir, &meta("abc"), &output(), at(11, 0)).unwrap();

    assert!(!first.path.exists());
    assert!(second.path.exists());
    assert!(second.warnings.is_empty());
    assert_eq!(summary_files(&dir).len(), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_resweep_under_the_same_name_keeps_the_summary() {
    let dir = tmp_dir("resweep_same");
    write_session(&dir, &meta("abc"), &output(), at(10, 0)).unwrap();
    let again = write_session(&dir, &meta("abc"), &output(), at(10, 0)).unwrap();

    assert!(again.path.exists());
    assert_eq!(summary_files(&dir).len(), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn delete_removes_every_summary_and_backup_the_session_owns() {
    let dir = tmp_dir("delete_orphans");
    let current = write_session(&dir, &meta("abc"), &output(), at(10, 0)).unwrap();
    let other = write_session(&dir, &meta("abc-2"), &output(), at(10, 0)).unwrap();
    // An orphan left by an older build, in another sweep-date folder, with a
    // redaction backup; plus a backup of the current summary.
    let orphan_dir = dir.join("sessions/proj/2026-01-01");
    fs::create_dir_all(&orphan_dir).unwrap();
    fs::write(orphan_dir.join("08-00-00-claudecode-abc-sweep.md"), "old").unwrap();
    fs::write(
        orphan_dir.join("08-00-00-claudecode-abc-sweep.md.bak-20260101080000"),
        "older",
    )
    .unwrap();
    let current_backup = format!("{}.bak-20260415100000", current.path.display());
    fs::write(&current_backup, "backup").unwrap();

    let result = delete_sessions(&dir, &["abc".to_string()]).unwrap();

    assert_eq!(result.deleted_ids, vec!["abc".to_string()]);
    assert!(result.failures.is_empty());
    let remaining = summary_files(&dir);
    assert_eq!(
        remaining,
        vec![other
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()]
    );
    assert_eq!(read_sessions(&dir).len(), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn delete_removes_only_the_exact_sessions_superseded_raws() {
    let dir = tmp_dir("delete_superseded");
    write_session(&dir, &meta("abc"), &output(), at(10, 0)).unwrap();
    let superseded = dir.join(super::SUPERSEDED_DIR);
    fs::create_dir_all(&superseded).unwrap();
    let own = superseded.join("abc.20260415T100000.000Z.jsonl.zst");
    let own_numbered = superseded.join("abc.20260415T100000.000Z-1.jsonl.zst");
    let lookalike = superseded.join("abc.1.20260415T100000.000Z.jsonl.zst");
    for file in [&own, &own_numbered, &lookalike] {
        fs::write(file, "raw").unwrap();
    }
    let raw = dir.join(raw_rel_path("abc"));
    fs::create_dir_all(raw.parent().unwrap()).unwrap();
    fs::write(&raw, "raw").unwrap();

    delete_sessions(&dir, &["abc".to_string()]).unwrap();

    assert!(!own.exists());
    assert!(!own_numbered.exists());
    assert!(!raw.exists());
    assert!(
        lookalike.exists(),
        "another session's raw copy must survive"
    );
    let _ = fs::remove_dir_all(&dir);
}
