// HalluScribe - unit tests for the profile distiller map step (distill.rs).
// Evidence-block and session-label tests live in evidence_tests.rs.

use super::*;
use std::fs;

const UUID_A: &str = "68c16c37-0cfc-832c-a816-610fd8015cb4";

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
    facts_response(section, &["S1"])
}

fn facts_response(section: &str, evidence: &[&str]) -> Value {
    serde_json::json!({
        "facts": [
            {
                "section": section,
                "fact": "Uses Rust and Tauri.",
                "evidence": evidence,
                "date": "2026-06-01"
            }
        ]
    })
}

#[test]
fn distill_batch_parses_facts_from_fake_tool_call() {
    let dir = tmp_dir("basic");
    fs::write(dir.join("s1.md"), "session body").unwrap();
    let e = entry(UUID_A, "s1.md");
    let batch = vec![&e];
    let tool_call: &ToolCallFn = &|_sys, _user, _tool, _max| Ok(ok_facts_response("conventions"));
    let (facts, warnings) = distill_batch(&dir, &batch, ProfileScope::Work, tool_call).unwrap();
    assert_eq!(facts.len(), 1);
    assert!(warnings.is_empty());
    assert_eq!(facts[0].section, ProfileSection::Conventions);
    // The model cited the label; downstream sees the real session id.
    assert_eq!(facts[0].evidence, vec![UUID_A.to_string()]);
}

#[test]
fn distill_batch_still_accepts_a_real_session_id_as_evidence() {
    // Recall guard for the label contract: a model that emits the real id
    // instead of the label must not lose its fact.
    let dir = tmp_dir("real_id_evidence");
    fs::write(dir.join("s1.md"), "session body").unwrap();
    let e = entry(UUID_A, "s1.md");
    let batch = vec![&e];
    let tool_call: &ToolCallFn =
        &|_sys, _user, _tool, _max| Ok(facts_response("conventions", &[UUID_A]));
    let (facts, warnings) = distill_batch(&dir, &batch, ProfileScope::Work, tool_call).unwrap();
    assert_eq!(facts.len(), 1);
    assert!(warnings.is_empty());
    assert_eq!(facts[0].evidence, vec![UUID_A.to_string()]);
}

#[test]
fn distill_batch_drops_fact_citing_a_label_outside_the_batch() {
    let dir = tmp_dir("out_of_range_label");
    fs::write(dir.join("s1.md"), "session body").unwrap();
    let e = entry(UUID_A, "s1.md");
    let batch = vec![&e];
    let tool_call: &ToolCallFn =
        &|_sys, _user, _tool, _max| Ok(facts_response("conventions", &["S9"]));
    let (facts, warnings) = distill_batch(&dir, &batch, ProfileScope::Work, tool_call).unwrap();
    assert!(facts.is_empty());
    assert!(warnings.iter().any(|w| w.contains("no valid evidence")));
}

#[test]
fn distill_batch_surfaces_malformed_tool_call_as_error() {
    let dir = tmp_dir("bad_json");
    fs::write(dir.join("s1.md"), "body one").unwrap();
    let e = entry(UUID_A, "s1.md");
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
                "evidence": ["S1"],
                "date": "2026-06-01"
            },
            {
                "section": "conventions",
                "fact": "Prefers Rust.",
                "evidence": ["S1"],
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
            { "section": "conventions", "evidence": ["S1"], "date": "2026-06-01" }
        ]
    });
    let error = parse_facts(ProfileScope::Work, &value).unwrap_err();
    assert!(error.to_string().contains("missing string fact"));
}

#[test]
fn both_map_prompts_state_the_session_label_contract() {
    // Regression guard for the 2026-08-04 fix: the model must be told to cite
    // the short `S<n>` labels, never a raw session id it would have to copy.
    for scope in [ProfileScope::Work, ProfileScope::Personal] {
        let prompt = map_system_prompt(scope);
        assert!(prompt.contains("Session id:"), "scope {scope:?}");
        assert!(prompt.contains("\"S1\""), "scope {scope:?}");
    }
}

#[test]
fn work_map_prompt_covers_domain_agnostic_recurring_problems() {
    // Phase 3a regression guard: the domain-agnostic recurring_problems
    // clause must not silently vanish from the Work map prompt.
    let prompt = map_system_prompt(ProfileScope::Work);
    assert!(prompt.contains("regardless of domain"));
    assert!(prompt.contains("recurring_problems"));
}

#[test]
fn personal_map_prompt_never_names_a_work_only_section() {
    // The 2026-08-04 fix: the shared prompt used to instruct the model to file
    // facts under recurring_problems even for Personal, whose skeleton rejects
    // that key — every such fact was then discarded by parse_facts.
    let prompt = map_system_prompt(ProfileScope::Personal);
    for key in ["recurring_problems", "conventions", "projects"] {
        assert!(
            !prompt.contains(key),
            "Personal map prompt names Work-only section {key}: {prompt}"
        );
    }
}

#[test]
fn personal_map_prompt_targets_personal_context() {
    let prompt = map_system_prompt(ProfileScope::Personal);
    assert!(prompt.contains("personal_context"));
}

#[test]
fn both_map_prompts_demand_tool_call_even_with_no_facts() {
    // Regression guard: a batch with nothing to extract (e.g. pure coding
    // sessions under the life-only Personal scope) must still produce a
    // save_profile_facts call; without this clause the model answers in
    // prose and the whole batch hard-fails (seen live 2026-07-08).
    for scope in [ProfileScope::Work, ProfileScope::Personal] {
        let prompt = map_system_prompt(scope);
        assert!(prompt.contains("empty facts array"), "scope {scope:?}");
        assert!(
            prompt.contains("never reply in plain text"),
            "scope {scope:?}"
        );
    }
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
