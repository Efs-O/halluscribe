// HalluScribe - unit tests for the map-step evidence block and the
// batch-local `S<n>` session labels (evidence.rs).

use super::*;
use crate::profile::types::ProfileSection;
use std::fs;

fn entry(id: &str, archive_path: &str) -> IndexEntry {
    IndexEntry {
        id: id.to_string(),
        project: "proj".to_string(),
        date: "2026-06-01".to_string(),
        // Deliberately not derived from `id`: the leak test below asserts the
        // real session id appears nowhere in the evidence block.
        title: "A session title".to_string(),
        tool: "Claude Code".to_string(),
        fill_pct: 90.0,
        session_timestamp: "2026-06-01T00:00:00+00:00".to_string(),
        updated_at: String::new(),
        session_type: "building".to_string(),
        error_tags: vec!["ECONNRESET".to_string()],
        topic_tags: vec!["rust".to_string()],
        archive_path: archive_path.to_string(),
        source_jsonl: String::new(),
        source_size_bytes: 0,
        provider: "claude_code".to_string(),
        fill_estimated: false,
        output_tokens: 0,
        tokens_estimated: true,
        transcript_hash: String::new(),
        secret_flags: Vec::new(),
        raw_path: String::new(),
    }
}

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "halluscribe_profile_evidence_{}_{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn fact(evidence: &[&str]) -> ProfileFact {
    ProfileFact {
        section: ProfileSection::Conventions,
        fact: "Uses Rust and Tauri.".to_string(),
        evidence: evidence.iter().map(|s| s.to_string()).collect(),
        date: "2026-06-01".to_string(),
    }
}

const UUID_A: &str = "68c16c37-0cfc-832c-a816-610fd8015cb4";
const UUID_B: &str = "d087b575-1234-4abc-9def-3bddaffa6ae1";

#[test]
fn labels_are_one_based() {
    assert_eq!(label_for(0), "S1");
    assert_eq!(label_for(17), "S18");
}

#[test]
fn label_map_pairs_each_label_with_its_real_id() {
    let a = entry(UUID_A, "a.md");
    let b = entry(UUID_B, "b.md");
    let batch = vec![&a, &b];
    let labels = label_map(&batch);
    assert_eq!(labels.get("S1").copied(), Some(UUID_A));
    assert_eq!(labels.get("S2").copied(), Some(UUID_B));
    assert_eq!(labels.len(), 2);
}

#[test]
fn canonical_label_accepts_the_wrappers_models_actually_emit() {
    for raw in [
        "S1",
        "s1",
        "1",
        "[S1]",
        "\"S1\"",
        "'S1'",
        " S1 ",
        "Session S1",
        "session id: 1",
        "`S1`",
    ] {
        assert_eq!(
            canonical_label(raw).as_deref(),
            Some("S1"),
            "failed to canonicalise {raw:?}"
        );
    }
}

#[test]
fn canonical_label_rejects_real_ids_and_nonsense() {
    // Real session ids contain dashes, so they can never be mistaken for a
    // label — the UUID path stays reachable for a model that emits one.
    assert_eq!(canonical_label(UUID_A), None);
    assert_eq!(
        canonical_label("6a0ec0a-e492-9654-83eb-b820-b8af29eb857e"),
        None
    );
    assert_eq!(canonical_label("0"), None);
    assert_eq!(canonical_label(""), None);
    assert_eq!(canonical_label("SX"), None);
}

#[test]
fn expand_labels_rewrites_labels_to_real_ids() {
    let a = entry(UUID_A, "a.md");
    let b = entry(UUID_B, "b.md");
    let batch = vec![&a, &b];
    let facts = expand_labels(vec![fact(&["S2", "s1"])], &label_map(&batch));
    assert_eq!(
        facts[0].evidence,
        vec![UUID_B.to_string(), UUID_A.to_string()]
    );
}

#[test]
fn expand_labels_leaves_a_real_id_untouched() {
    // Recall guard: a model that ignores the label contract and emits the real
    // id must still resolve, via the caller's exact/prefix validation.
    let a = entry(UUID_A, "a.md");
    let batch = vec![&a];
    let facts = expand_labels(vec![fact(&[UUID_A])], &label_map(&batch));
    assert_eq!(facts[0].evidence, vec![UUID_A.to_string()]);
}

#[test]
fn expand_labels_leaves_out_of_range_label_for_validation_to_drop() {
    let a = entry(UUID_A, "a.md");
    let batch = vec![&a];
    let facts = expand_labels(vec![fact(&["S7"])], &label_map(&batch));
    assert_eq!(facts[0].evidence, vec!["S7".to_string()]);
}

#[test]
fn build_evidence_block_labels_sessions_and_never_leaks_real_ids() {
    let dir = tmp_dir("block");
    fs::write(dir.join("a.md"), "body a").unwrap();
    fs::write(dir.join("b.md"), "body b").unwrap();
    let a = entry(UUID_A, "a.md");
    let b = entry(UUID_B, "b.md");
    let block = build_evidence_block(&dir, &[&a, &b]);
    assert!(block.contains("Session id: S1"));
    assert!(block.contains("Session id: S2"));
    assert!(
        !block.contains(UUID_A) && !block.contains(UUID_B),
        "real session id leaked into the evidence block: {block}"
    );
}

#[test]
fn build_evidence_entry_includes_metadata() {
    let dir = tmp_dir("metadata");
    fs::write(dir.join("s1.md"), "hello").unwrap();
    let e = entry("s1", "s1.md");
    let block = build_evidence_entry(&dir, &e, "S1");
    assert!(block.contains("Session id: S1"));
    assert!(block.contains("Project: proj"));
    assert!(block.contains("ECONNRESET"));
    assert!(block.contains("rust"));
}

#[test]
fn truncate_chars_is_multibyte_safe() {
    let text = "café".repeat(1000); // multi-byte 'é' repeated well past the limit
    let truncated = truncate_chars(&text, 10);
    assert_eq!(truncated.chars().count(), 10);
    // Must still be valid UTF-8 (guaranteed by String) and start correctly.
    assert!(truncated.starts_with("café"));
}

#[test]
fn build_evidence_entry_truncates_long_bodies() {
    let dir = tmp_dir("truncate");
    let long_body = "x".repeat(5000);
    fs::write(dir.join("s1.md"), &long_body).unwrap();
    let e = entry("s1", "s1.md");
    let block = build_evidence_entry(&dir, &e, "S1");
    // Body section should be head + snip marker + tail, not the full 5000.
    let body_start = block.find("Body:\n").unwrap() + "Body:\n".len();
    let body = &block[body_start..];
    assert_eq!(
        body.chars().count(),
        HEAD_CHARS + SNIP_MARKER.chars().count() + TAIL_CHARS
    );
}

#[test]
fn head_tail_window_short_body_passes_through_verbatim() {
    let short = "hello world, short body";
    assert_eq!(head_tail_window(short), short);
}

#[test]
fn head_tail_window_boundary_body_passes_through_verbatim() {
    // Exactly HEAD_CHARS + TAIL_CHARS chars: still verbatim (<=, not <).
    let body = "y".repeat(HEAD_CHARS + TAIL_CHARS);
    let windowed = head_tail_window(&body);
    assert_eq!(windowed, body);
}

#[test]
fn head_tail_window_long_body_keeps_head_marker_and_exact_tail_count() {
    let head_part = "A".repeat(HEAD_CHARS);
    let middle = "M".repeat(2000);
    let tail_part = "Z".repeat(TAIL_CHARS);
    let body = format!("{head_part}{middle}{tail_part}");
    let windowed = head_tail_window(&body);

    assert!(windowed.starts_with(&head_part));
    assert!(windowed.contains(SNIP_MARKER));
    assert!(windowed.ends_with(&tail_part));
    assert_eq!(
        windowed.chars().count(),
        HEAD_CHARS + SNIP_MARKER.chars().count() + TAIL_CHARS
    );
}

#[test]
fn head_tail_window_sentinel_at_end_of_oversized_body_survives() {
    let filler = "x".repeat(10_000);
    let sentinel = "SENTINEL_KEY_DECISION_MARKER";
    let body = format!("{filler}{sentinel}");
    let windowed = head_tail_window(&body);
    assert!(
        windowed.ends_with(sentinel),
        "sentinel dropped from tail: {windowed}"
    );
}

#[test]
fn head_tail_window_greek_multibyte_never_splits_a_codepoint() {
    // Greek content around both the head and tail cut points; must not
    // panic (byte-slice mid-codepoint) and must preserve valid chars only.
    let greek_head = "Καλημέρα κόσμε, ας δούμε πώς πάει η δουλειά σήμερα. ".repeat(50);
    let greek_middle = "Ενδιάμεσο κείμενο που θα κοπεί εντελώς. ".repeat(200);
    let greek_tail = "Οι αποφάσεις που πάρθηκαν και τα επόμενα βήματα είναι εδώ. ".repeat(60);
    let body = format!("{greek_head}{greek_middle}{greek_tail}");
    let windowed = head_tail_window(&body);
    // No panic reaching here is itself the primary assertion; also assert
    // the exact char budget and that the tail's final content survives.
    assert_eq!(
        windowed.chars().count(),
        HEAD_CHARS + SNIP_MARKER.chars().count() + TAIL_CHARS
    );
    assert!(windowed.ends_with("εδώ. "));
}

#[test]
fn split_evidence_ids_unpacks_comma_joined_citations() {
    // The exact live 2026-08-04 shape: every citation packed into one string,
    // which matched neither a label nor a real id, so the fact lost all of its
    // evidence and was then dropped entirely.
    let packed = vec!["S1, S3, S4, S6, S7, S18".to_string()];
    assert_eq!(
        split_evidence_ids(packed),
        vec!["S1", "S3", "S4", "S6", "S7", "S18"]
    );
}

#[test]
fn split_evidence_ids_leaves_well_formed_input_unchanged() {
    let clean = vec!["S1".to_string(), "S2".to_string()];
    assert_eq!(split_evidence_ids(clean.clone()), clean);
}

#[test]
fn split_evidence_ids_preserves_real_session_ids() {
    // Real ids are UUIDs — dashes and hex only, no separator characters — so
    // splitting must never break one apart.
    let id = "3d13a20e-e043-4946-b015-2396b073a37e".to_string();
    assert_eq!(split_evidence_ids(vec![id.clone()]), vec![id]);
}

#[test]
fn split_evidence_ids_dedupes_and_drops_empties() {
    let messy = vec!["S1,,S2 ; S1".to_string(), "  ".to_string()];
    assert_eq!(split_evidence_ids(messy), vec!["S1", "S2"]);
}
