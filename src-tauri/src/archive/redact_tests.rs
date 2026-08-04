// HalluScribe - unit tests for durable session redaction.

use super::*;
use crate::archive::writer::write_session;
use crate::archive::SessionMeta;
use crate::gemma::{GemmaOutput, SessionType};
use chrono::{DateTime, TimeZone, Utc};
use std::path::PathBuf;

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 15, 14, 30, 0).unwrap()
}

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("halluscribe_redact_test_{name}"));
    let _ = fs::remove_dir_all(&d);
    d
}

fn sample_meta(source: &Path, id: &str) -> SessionMeta {
    SessionMeta {
        id: id.to_string(),
        source: source.to_path_buf(),
        project: "my-project".into(),
        tool: "Claude Code".into(),
        provider: "claude_code".into(),
        fill_pct: 78.5,
        fill_estimated: false,
        backend: "llama.cpp".into(),
        session_timestamp: fixed_now(),
        updated_at: None,
        transcript_hash: "abc123".into(),
        raw_path: None,
    }
}

fn output_with_summary(summary: &str) -> GemmaOutput {
    GemmaOutput {
        title: "Test session".into(),
        summary: summary.to_string(),
        session_type: SessionType::Debugging,
        error_tags: vec![],
        topic_tags: vec![],
        verbatim_highlights: vec![],
    }
}

// -- ledger --

#[test]
fn load_rules_missing_file_is_empty() {
    let dir = tmp_dir("ledger_missing");
    assert!(load_rules(&dir).is_empty());
}

#[test]
fn save_then_load_roundtrips() {
    let dir = tmp_dir("ledger_roundtrip");
    fs::create_dir_all(&dir).unwrap();
    let rule = RedactionRule {
        session_id: "abc".into(),
        find: "secretpass".into(),
        replace: "[REDACTED]".into(),
    };
    let mut ledger = Ledger::default();
    ledger.rules.push(rule.clone());
    save_rules(&dir, &ledger).unwrap();

    let loaded = load_rules(&dir);
    assert_eq!(loaded, vec![rule]);
}

#[test]
fn rules_for_session_filters_by_id() {
    let dir = tmp_dir("ledger_filter");
    fs::create_dir_all(&dir).unwrap();
    let ledger = Ledger {
        rules: vec![
            RedactionRule {
                session_id: "a".into(),
                find: "one-secret".into(),
                replace: "x".into(),
            },
            RedactionRule {
                session_id: "b".into(),
                find: "two-secret".into(),
                replace: "y".into(),
            },
        ],
    };
    save_rules(&dir, &ledger).unwrap();

    let for_a = rules_for_session(&dir, "a");
    assert_eq!(for_a.len(), 1);
    assert_eq!(for_a[0].find, "one-secret");
}

// -- apply_rules --

#[test]
fn apply_rules_replaces_all_occurrences_in_order() {
    let rules = vec![
        RedactionRule {
            session_id: "s".into(),
            find: "alpha".into(),
            replace: "AAA".into(),
        },
        RedactionRule {
            session_id: "s".into(),
            find: "beta".into(),
            replace: "BBB".into(),
        },
    ];
    let text = "alpha and beta and alpha again";
    let out = apply_rules(text, &rules);
    assert_eq!(out, "AAA and BBB and AAA again");
}

// -- preview --

#[test]
fn preview_counts_occurrences_and_marks_matches() {
    let dir = tmp_dir("preview_basic");
    let src = Path::new("/fake/preview-session.jsonl");
    let meta = sample_meta(src, "preview-session");
    write_session(
        &dir,
        &meta,
        &output_with_summary("ftp password is hunter2hunter2, reused twice: hunter2hunter2"),
        fixed_now(),
    )
    .unwrap();

    let preview = preview_redaction(&dir, "preview-session", "hunter2hunter2").unwrap();
    assert_eq!(preview.occurrences, 2);
    assert!(!preview.excerpts.is_empty());
    assert!(preview.excerpts[0].contains(">>>hunter2hunter2<<<"));
}

#[test]
fn preview_rejects_short_find() {
    let dir = tmp_dir("preview_short");
    let src = Path::new("/fake/short.jsonl");
    let meta = sample_meta(src, "short");
    write_session(&dir, &meta, &output_with_summary("hi there"), fixed_now()).unwrap();

    let result = preview_redaction(&dir, "short", "abc");
    assert!(matches!(result, Err(ArchiveError::Invalid(_))));
}

#[test]
fn preview_rejects_empty_find() {
    let dir = tmp_dir("preview_empty");
    let src = Path::new("/fake/empty.jsonl");
    let meta = sample_meta(src, "empty");
    write_session(&dir, &meta, &output_with_summary("hi there"), fixed_now()).unwrap();

    let result = preview_redaction(&dir, "empty", "   ");
    assert!(matches!(result, Err(ArchiveError::Invalid(_))));
}

// -- apply_redaction --

#[test]
fn apply_redaction_creates_backup_with_original_content() {
    let dir = tmp_dir("apply_backup");
    let src = Path::new("/fake/backup-session.jsonl");
    let meta = sample_meta(src, "backup-session");
    let path = write_session(
        &dir,
        &meta,
        &output_with_summary("ftp password: swordfish123"),
        fixed_now(),
    )
    .unwrap()
    .path;
    let original = fs::read_to_string(&path).unwrap();

    let outcome = apply_redaction(&dir, "backup-session", "swordfish123", "[REDACTED]").unwrap();
    assert_eq!(outcome.replacements, 1);

    let backup_content = fs::read_to_string(&outcome.backup_path).unwrap();
    assert_eq!(backup_content, original);
    assert!(backup_content.contains("swordfish123"));
}

#[test]
fn apply_redaction_rewrites_file_and_persists_rule() {
    let dir = tmp_dir("apply_rewrite");
    let src = Path::new("/fake/rewrite-session.jsonl");
    let meta = sample_meta(src, "rewrite-session");
    let path = write_session(
        &dir,
        &meta,
        &output_with_summary("smtp password: correcthorse"),
        fixed_now(),
    )
    .unwrap()
    .path;

    apply_redaction(&dir, "rewrite-session", "correcthorse", "[REDACTED]").unwrap();

    let updated = fs::read_to_string(&path).unwrap();
    assert!(!updated.contains("correcthorse"));
    assert!(updated.contains("[REDACTED]"));

    let rules = rules_for_session(&dir, "rewrite-session");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].find, "correcthorse");
    assert_eq!(rules[0].replace, "[REDACTED]");
}

#[test]
fn apply_redaction_default_replace_when_empty() {
    let dir = tmp_dir("apply_default_replace");
    let src = Path::new("/fake/default-replace.jsonl");
    let meta = sample_meta(src, "default-replace");
    write_session(
        &dir,
        &meta,
        &output_with_summary("token abcd1234efgh"),
        fixed_now(),
    )
    .unwrap();

    apply_redaction(&dir, "default-replace", "abcd1234efgh", "").unwrap();
    let rules = rules_for_session(&dir, "default-replace");
    assert_eq!(rules[0].replace, "[REDACTED]");
}

#[test]
fn apply_redaction_errors_when_text_not_found() {
    let dir = tmp_dir("apply_not_found");
    let src = Path::new("/fake/not-found.jsonl");
    let meta = sample_meta(src, "not-found");
    write_session(
        &dir,
        &meta,
        &output_with_summary("nothing to see here"),
        fixed_now(),
    )
    .unwrap();

    let result = apply_redaction(&dir, "not-found", "missingtext", "[REDACTED]");
    match result {
        Err(ArchiveError::Invalid(msg)) => assert!(msg.contains("not found")),
        other => panic!("expected Invalid error, got {other:?}"),
    }
}

#[test]
fn redaction_survives_resweep_via_ledger() {
    let dir = tmp_dir("durability");
    let src = Path::new("/fake/durable-session.jsonl");
    let meta = sample_meta(src, "durable-session");
    write_session(
        &dir,
        &meta,
        &output_with_summary("ftp password is topsecret99"),
        fixed_now(),
    )
    .unwrap();

    apply_redaction(&dir, "durable-session", "topsecret99", "[REDACTED]").unwrap();

    // Simulate the nightly sweep re-summarizing the same session with the secret
    // still present in the freshly generated Gemma output.
    let path = write_session(
        &dir,
        &meta,
        &output_with_summary("ftp password is topsecret99 (again)"),
        fixed_now(),
    )
    .unwrap()
    .path;

    let resweep_content = fs::read_to_string(&path).unwrap();
    assert!(!resweep_content.contains("topsecret99"));
    assert!(resweep_content.contains("[REDACTED]"));
}

#[test]
fn apply_redaction_clears_secret_flags_after_rewrite() {
    let dir = tmp_dir("secret_flags_clear");
    let src = Path::new("/fake/secret-flags-session.jsonl");
    let meta = sample_meta(src, "secret-flags-session");
    let written = write_session(
        &dir,
        &meta,
        &output_with_summary("password: hunter42secret99"),
        fixed_now(),
    )
    .unwrap();
    assert!(!written.secret_flags.is_empty());
    let before = crate::archive::find_session(&dir, "secret-flags-session").unwrap();
    assert!(!before.secret_flags.is_empty());

    apply_redaction(
        &dir,
        "secret-flags-session",
        "hunter42secret99",
        "[REDACTED]",
    )
    .unwrap();

    let after = crate::archive::find_session(&dir, "secret-flags-session").unwrap();
    assert!(after.secret_flags.is_empty());
}
