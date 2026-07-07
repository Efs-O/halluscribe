// HalluScribe - unit tests for the profile distiller map step (distill.rs).

use super::*;
use std::fs;

fn entry(id: &str, archive_path: &str) -> IndexEntry {
    IndexEntry {
        id: id.to_string(),
        project: "proj".to_string(),
        date: "2026-06-01".to_string(),
        title: format!("Session {id}"),
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
        transcript_hash: String::new(),
        secret_flags: Vec::new(),
        raw_path: String::new(),
    }
}

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("halluscribe_profile_distill_{name}"));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn ok_facts_response(section: &str) -> Value {
    serde_json::json!({
        "facts": [
            {
                "section": section,
                "fact": "Uses Rust and Tauri.",
                "evidence": ["s1"],
                "date": "2026-06-01"
            }
        ]
    })
}

#[test]
fn distill_batch_parses_facts_from_fake_tool_call() {
    let dir = tmp_dir("basic");
    fs::write(dir.join("s1.md"), "session body").unwrap();
    let e = entry("s1", "s1.md");
    let batch = vec![&e];
    let tool_call: &ToolCallFn = &|_sys, _user, _tool, _max| Ok(ok_facts_response("conventions"));
    let (facts, warnings) = distill_batch(&dir, &batch, ProfileScope::Work, tool_call).unwrap();
    assert_eq!(facts.len(), 1);
    assert!(warnings.is_empty());
    assert_eq!(facts[0].section, ProfileSection::Conventions);
    assert_eq!(facts[0].evidence, vec!["s1".to_string()]);
}

#[test]
fn distill_batch_surfaces_malformed_tool_call_as_error() {
    let dir = tmp_dir("bad_json");
    fs::write(dir.join("s1.md"), "body one").unwrap();
    let e = entry("s1", "s1.md");
    let batch = vec![&e];
    let tool_call: &ToolCallFn =
        &|_sys, _user, _tool, _max| Ok(serde_json::json!({"not_facts": true}));
    let error = distill_batch(&dir, &batch, ProfileScope::Work, tool_call).unwrap_err();
    assert!(error.to_string().contains("missing facts array"));
}

#[test]
fn parse_facts_maps_preferences_alias_to_conventions() {
    let value = ok_facts_response("preferences");
    let (facts, warnings) = parse_facts(ProfileScope::Work, &value).unwrap();
    assert_eq!(facts.len(), 1);
    assert!(warnings.is_empty());
    assert_eq!(facts[0].section, ProfileSection::Conventions);
}

#[test]
fn parse_facts_alias_table_covers_documented_keys() {
    let cases = [
        ("convention", ProfileSection::Conventions),
        ("conventions_preferences", ProfileSection::Conventions),
        ("problems", ProfileSection::RecurringProblems),
        ("recurring", ProfileSection::RecurringProblems),
        ("communication", ProfileSection::CommunicationStyle),
        ("style", ProfileSection::CommunicationStyle),
        ("project", ProfileSection::Projects),
        ("active_projects", ProfileSection::Projects),
        ("context", ProfileSection::Identity),
        ("highlights", ProfileSection::Timeline),
    ];
    for (alias, expected) in cases {
        let (facts, warnings) = parse_facts(ProfileScope::Work, &ok_facts_response(alias)).unwrap();
        assert_eq!(facts.len(), 1, "alias {alias}");
        assert!(warnings.is_empty(), "alias {alias}");
        assert_eq!(facts[0].section, expected, "alias {alias}");
    }
    // Personal-only aliases resolve under the Personal scope.
    for alias in ["personal", "interests", "life_context"] {
        let (facts, _) = parse_facts(ProfileScope::Personal, &ok_facts_response(alias)).unwrap();
        assert_eq!(
            facts[0].section,
            ProfileSection::PersonalContext,
            "alias {alias}"
        );
    }
}

#[test]
fn parse_facts_skips_unknown_section_with_warning() {
    let value = serde_json::json!({
        "facts": [
            {
                "section": "not_a_real_section",
                "fact": "Something.",
                "evidence": ["s1"],
                "date": "2026-06-01"
            },
            {
                "section": "conventions",
                "fact": "Prefers Rust.",
                "evidence": ["s1"],
                "date": "2026-06-01"
            }
        ]
    });
    let (facts, warnings) = parse_facts(ProfileScope::Work, &value).unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].section, ProfileSection::Conventions);
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0].contains("skipped fact with unknown section 'not_a_real_section'"),
        "got: {}",
        warnings[0]
    );
}

#[test]
fn parse_facts_skips_personal_context_fact_under_work_scope() {
    let value = ok_facts_response("personal_context");
    let (facts, warnings) = parse_facts(ProfileScope::Work, &value).unwrap();
    assert!(facts.is_empty());
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0].contains("'personal_context'"),
        "got: {}",
        warnings[0]
    );

    // The same fact is accepted under the Personal scope.
    let (facts, warnings) = parse_facts(ProfileScope::Personal, &value).unwrap();
    assert_eq!(facts.len(), 1);
    assert!(warnings.is_empty());
}

#[test]
fn parse_facts_missing_fact_string_still_fails_batch() {
    let value = serde_json::json!({
        "facts": [
            { "section": "conventions", "evidence": ["s1"], "date": "2026-06-01" }
        ]
    });
    let error = parse_facts(ProfileScope::Work, &value).unwrap_err();
    assert!(error.to_string().contains("missing string fact"));
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
    let block = build_evidence_entry(&dir, &e);
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
fn build_evidence_entry_includes_metadata() {
    let dir = tmp_dir("metadata");
    fs::write(dir.join("s1.md"), "hello").unwrap();
    let e = entry("s1", "s1.md");
    let block = build_evidence_entry(&dir, &e);
    assert!(block.contains("Session id: s1"));
    assert!(block.contains("Project: proj"));
    assert!(block.contains("ECONNRESET"));
    assert!(block.contains("rust"));
}

#[test]
fn work_map_prompt_is_byte_identical_to_original() {
    assert_eq!(map_system_prompt(ProfileScope::Work), MAP_SYSTEM_PROMPT);
}

#[test]
fn personal_map_prompt_adds_personal_context_instruction() {
    let prompt = map_system_prompt(ProfileScope::Personal);
    assert!(prompt.starts_with(MAP_SYSTEM_PROMPT));
    assert!(prompt.contains("personal_context"));
}

#[test]
fn map_system_prompt_covers_domain_agnostic_recurring_problems() {
    // Phase 3a regression guard: the domain-agnostic recurring_problems
    // clause must not silently vanish from the map prompt.
    assert!(MAP_SYSTEM_PROMPT.contains("regardless of domain"));
    assert!(MAP_SYSTEM_PROMPT.contains("recurring_problems"));
}

#[test]
fn personal_map_prompt_covers_hedged_household_structure() {
    // Phase 3b regression guard: the household/relationship structure
    // instruction must not silently vanish from the Personal map prompt.
    let prompt = map_system_prompt(ProfileScope::Personal);
    assert!(prompt.contains("household and relationship structure"));
    assert!(prompt.contains("likely"));
}

#[test]
fn work_tool_schema_excludes_personal_context_section() {
    let tool = save_profile_facts_tool(ProfileScope::Work);
    let enum_values = tool["function"]["parameters"]["properties"]["facts"]["items"]["properties"]
        ["section"]["enum"]
        .as_array()
        .unwrap();
    assert_eq!(enum_values.len(), 6);
    assert!(!enum_values.iter().any(|v| v == "personal_context"));
}

#[test]
fn personal_tool_schema_includes_personal_context_section() {
    let tool = save_profile_facts_tool(ProfileScope::Personal);
    let enum_values = tool["function"]["parameters"]["properties"]["facts"]["items"]["properties"]
        ["section"]["enum"]
        .as_array()
        .unwrap();
    assert_eq!(enum_values.len(), 4);
    assert!(enum_values.iter().any(|v| v == "personal_context"));
}

#[test]
fn parse_facts_skips_conventions_fact_under_personal_scope() {
    // Conventions is a coding-only section, dropped from the Personal
    // life/character skeleton (Phase 0 refocus) even though it is still
    // valid under Work scope.
    let value = ok_facts_response("conventions");
    let (facts, warnings) = parse_facts(ProfileScope::Personal, &value).unwrap();
    assert!(facts.is_empty());
    assert_eq!(warnings.len(), 1);
}

fn fact(evidence: &[&str]) -> ProfileFact {
    ProfileFact {
        section: ProfileSection::Conventions,
        fact: "Uses Rust and Tauri.".to_string(),
        evidence: evidence.iter().map(|s| s.to_string()).collect(),
        date: "2026-06-01".to_string(),
    }
}

#[test]
fn validate_evidence_keeps_exact_match() {
    let known: HashSet<&str> = ["abc12345", "def67890"].into_iter().collect();
    let (facts, warnings) = validate_evidence(vec![fact(&["abc12345"])], &known);
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].evidence, vec!["abc12345".to_string()]);
    assert!(warnings.is_empty());
}

#[test]
fn validate_evidence_repairs_unique_prefix() {
    let known: HashSet<&str> = ["d087b575-1234-4abc-9def-3bddaffa6ae1", "other-id-99999999"]
        .into_iter()
        .collect();
    let (facts, warnings) = validate_evidence(vec![fact(&["d087b575-1234"])], &known);
    assert_eq!(facts.len(), 1);
    assert_eq!(
        facts[0].evidence,
        vec!["d087b575-1234-4abc-9def-3bddaffa6ae1".to_string()]
    );
    assert!(warnings.is_empty());
}

#[test]
fn validate_evidence_drops_unknown_id_but_keeps_fact() {
    let known: HashSet<&str> = ["abc12345", "def67890"].into_iter().collect();
    let (facts, warnings) = validate_evidence(vec![fact(&["abc12345", "totally-fake-id"])], &known);
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].evidence, vec!["abc12345".to_string()]);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("totally-fake-id"));
}

#[test]
fn validate_evidence_drops_fact_when_all_evidence_invalid() {
    let known: HashSet<&str> = ["abc12345", "def67890"].into_iter().collect();
    let (facts, warnings) = validate_evidence(vec![fact(&["nope-not-real"])], &known);
    assert!(facts.is_empty());
    assert_eq!(warnings.len(), 2); // one for the dropped id, one for the dropped fact
    assert!(warnings.iter().any(|w| w.contains("no valid evidence")));
}

#[test]
fn validate_evidence_ambiguous_prefix_is_dropped_not_repaired() {
    let known: HashSet<&str> = ["abcd1234-aaaa", "abcd1234-bbbb"].into_iter().collect();
    let (facts, warnings) = validate_evidence(vec![fact(&["abcd1234"])], &known);
    assert!(facts.is_empty());
    assert!(warnings.iter().any(|w| w.contains("abcd1234")));
}
