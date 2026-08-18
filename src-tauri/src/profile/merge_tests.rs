// HalluScribe - unit tests for the profile distiller reduce step (merge.rs):
// per-section bounded merge calls, keep-previous policy, and fallbacks. The
// two-pass consolidation path lives in merge_progress_tests.rs.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

fn fact(section: ProfileSection, text: &str, date: &str) -> ProfileFact {
    ProfileFact {
        section,
        fact: text.to_string(),
        evidence: vec!["s1".to_string()],
        date: date.to_string(),
    }
}

fn section_response(text: &str) -> Value {
    serde_json::json!({ "content": text })
}

#[test]
fn run_reduce_merges_each_section_with_facts_via_one_call_each() {
    let facts = vec![
        fact(
            ProfileSection::Projects,
            "Building HalluScribe.",
            "2026-06-01",
        ),
        fact(ProfileSection::Identity, "Windows developer.", "2026-06-02"),
    ];
    let calls = Mutex::new(Vec::<String>::new());
    let tool_call: &ToolCallFn = &|_sys, user, tool, max| {
        assert_eq!(
            tool["function"]["name"].as_str(),
            Some("save_profile_section")
        );
        assert_eq!(max, SECTION_MAX_TOKENS);
        calls.lock().unwrap().push(user.to_string());
        Ok(section_response("merged prose [s1]"))
    };
    let sections = run_reduce(
        ProfileScope::Work,
        None,
        &facts,
        tool_call,
        &mut Vec::new(),
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    // Exactly one call per section that has facts; other sections untouched.
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls
        .iter()
        .any(|c| c.starts_with("Section: Identity & Context")));
    assert!(calls
        .iter()
        .any(|c| c.starts_with("Section: Active Projects")));
    assert!(calls
        .iter()
        .all(|c| c.contains("Previous content:\n(none)")));
    assert!(calls
        .iter()
        .all(|c| c.contains("New candidate facts (newest first):")));
    assert_eq!(sections.identity, "merged prose [s1]");
    assert_eq!(sections.projects, "merged prose [s1]");
    assert_eq!(sections.conventions, "");
    assert_eq!(sections.timeline, "");
}

#[test]
fn section_with_previous_text_but_no_new_facts_is_kept_without_a_call() {
    let previous_md = "# User Profile — old\n\n## Conventions & Preferences\n\n\
                       Prefers rebase over merge [old-1]\n\n## Active Projects\n\nOld project\n\n";
    let facts = vec![fact(ProfileSection::Projects, "New project.", "2026-06-01")];
    let calls = AtomicUsize::new(0);
    let tool_call: &ToolCallFn = &|_sys, _user, _tool, _max| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(section_response("merged projects [s1]"))
    };
    let sections = run_reduce(
        ProfileScope::Work,
        Some(previous_md),
        &facts,
        tool_call,
        &mut Vec::new(),
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    // Only Projects had new facts → exactly one model call.
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(sections.conventions, "Prefers rebase over merge [old-1]");
    assert_eq!(sections.projects, "merged projects [s1]");
    assert_eq!(sections.identity, "");
}

#[test]
fn merge_call_receives_previous_section_content_not_whole_profile() {
    let previous_md = "## Active Projects\n\nOld project text [old-1]\n\n\
                       ## Timeline Highlights\n\nUnrelated timeline\n\n";
    let facts = vec![fact(ProfileSection::Projects, "New work.", "2026-06-01")];
    let seen = Mutex::new(String::new());
    let tool_call: &ToolCallFn = &|_sys, user, _tool, _max| {
        *seen.lock().unwrap() = user.to_string();
        Ok(section_response("merged"))
    };
    run_reduce(
        ProfileScope::Work,
        Some(previous_md),
        &facts,
        tool_call,
        &mut Vec::new(),
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    let seen = seen.lock().unwrap();
    assert!(seen.contains("Previous content:\nOld project text [old-1]"));
    assert!(!seen.contains("Unrelated timeline"));
}

#[test]
fn projects_section_call_appends_stale_projects_note_others_do_not() {
    let facts = vec![
        fact(
            ProfileSection::Projects,
            "Building HalluScribe.",
            "2026-06-01",
        ),
        fact(ProfileSection::Identity, "Windows developer.", "2026-06-02"),
    ];
    let calls = Mutex::new(Vec::<String>::new());
    let tool_call: &ToolCallFn = &|_sys, user, _tool, _max| {
        calls.lock().unwrap().push(user.to_string());
        Ok(section_response("merged"))
    };
    run_reduce(
        ProfileScope::Work,
        None,
        &facts,
        tool_call,
        &mut Vec::new(),
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    let calls = calls.lock().unwrap();
    let projects_call = calls
        .iter()
        .find(|c| c.starts_with("Section: Active Projects"))
        .unwrap();
    let identity_call = calls
        .iter()
        .find(|c| c.starts_with("Section: Identity & Context"))
        .unwrap();
    assert!(projects_call.contains("more than 12 months"));
    assert!(!identity_call.contains("more than 12 months"));
}

#[test]
fn failed_section_call_falls_back_to_previous_plus_raw_bullets_with_warning() {
    let previous_md = "## Active Projects\n\nOld project text [old-1]\n\n";
    let facts = vec![fact(ProfileSection::Projects, "New work.", "2026-06-01")];
    let tool_call: &ToolCallFn =
        &|_sys, _user, _tool, _max| Err(ProfileError::BadToolCall("truncated".to_string()));
    let mut warnings = Vec::new();
    let sections = run_reduce(
        ProfileScope::Work,
        Some(previous_md),
        &facts,
        tool_call,
        &mut warnings,
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0].contains("merge section projects:") && warnings[0].contains("kept raw facts"),
        "got: {}",
        warnings[0]
    );
    assert!(sections.projects.starts_with("Old project text [old-1]\n"));
    assert!(sections.projects.contains("New work."));
}

#[test]
fn failed_section_call_without_previous_keeps_raw_bullets_only() {
    let facts = vec![fact(
        ProfileSection::Identity,
        "Uses Windows.",
        "2026-06-01",
    )];
    let tool_call: &ToolCallFn =
        &|_sys, _user, _tool, _max| Ok(serde_json::json!({"wrong_key": true}));
    let mut warnings = Vec::new();
    let sections = run_reduce(
        ProfileScope::Work,
        None,
        &facts,
        tool_call,
        &mut warnings,
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("missing content string"));
    assert!(sections.identity.contains("Uses Windows."));
    assert!(!sections.identity.starts_with('\n'));
}

#[test]
fn chunk_lines_oversized_single_line_is_own_chunk() {
    let text = format!("short\n{}\nshort2", "z".repeat(500));
    let chunks = chunk_lines(&text, 100);
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0], "short");
    assert_eq!(chunks[2], "short2");
}

#[test]
fn no_facts_and_no_previous_yields_empty_sections_and_zero_calls() {
    let calls = AtomicUsize::new(0);
    let tool_call: &ToolCallFn = &|_sys, _user, _tool, _max| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(section_response(""))
    };
    let sections = run_reduce(
        ProfileScope::Work,
        None,
        &[],
        tool_call,
        &mut Vec::new(),
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    for section in ProfileSection::ALL {
        assert_eq!(sections.get(section), "");
    }
}

#[test]
fn serialize_section_facts_orders_newest_first() {
    let facts = vec![
        fact(ProfileSection::Timeline, "old", "2026-01-01"),
        fact(ProfileSection::Timeline, "new", "2026-06-01"),
    ];
    let serialized = serialize_section_facts(ProfileSection::Timeline, &facts);
    let new_pos = serialized.find("new").unwrap();
    let old_pos = serialized.find("old").unwrap();
    assert!(new_pos < old_pos);
}

#[test]
fn personal_scope_merges_personal_context_section() {
    let facts = vec![fact(
        ProfileSection::PersonalContext,
        "Enjoys hiking.",
        "2026-06-01",
    )];
    let tool_call: &ToolCallFn =
        &|_sys, _user, _tool, _max| Ok(section_response("Enjoys hiking. [s1]"));
    let sections = run_reduce(
        ProfileScope::Personal,
        None,
        &facts,
        tool_call,
        &mut Vec::new(),
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    assert_eq!(sections.personal_context, "Enjoys hiking. [s1]");
}

#[test]
fn work_scope_never_merges_personal_context() {
    // A personal_context fact under Work scope is ignored by the reduce (the
    // scope skeleton drives the loop), leaving the field empty.
    let facts = vec![
        fact(
            ProfileSection::PersonalContext,
            "Enjoys hiking.",
            "2026-06-01",
        ),
        fact(ProfileSection::Identity, "Dev.", "2026-06-01"),
    ];
    let tool_call: &ToolCallFn = &|_sys, _user, _tool, _max| Ok(section_response("merged"));
    let sections = run_reduce(
        ProfileScope::Work,
        None,
        &facts,
        tool_call,
        &mut Vec::new(),
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();
    assert_eq!(sections.personal_context, "");
    assert_eq!(sections.identity, "merged");
}

#[test]
fn identity_merge_input_carries_the_conflicting_attribute_warning() {
    // The generic "recency wins on contradictions" rule turned three ages
    // belonging to three different people into "46 (previously 45 and 43)".
    // Identity gets an explicit counter-instruction; other sections must not,
    // since for them recency genuinely is the right rule.
    let facts = vec![
        fact(ProfileSection::Identity, "Name is A.", "2026-06-02"),
        fact(ProfileSection::Conventions, "Prefers tabs.", "2026-06-01"),
    ];
    let calls = Mutex::new(Vec::<String>::new());
    let tool_call: &ToolCallFn = &|_sys, user, _tool, _max| {
        calls.lock().unwrap().push(user.to_string());
        Ok(section_response("merged [s1]"))
    };
    run_reduce(
        ProfileScope::Work,
        None,
        &facts,
        tool_call,
        &mut Vec::new(),
        &AtomicBool::new(false),
        &mut |_, _| {},
    )
    .unwrap();

    let calls = calls.lock().unwrap();
    let identity = calls
        .iter()
        .find(|c| c.starts_with("Section: Identity & Context"))
        .expect("identity section merged");
    assert!(identity.contains("DIFFERENT people"));
    assert!(identity.contains("omit it"));

    let conventions = calls
        .iter()
        .find(|c| c.starts_with("Section: Conventions & Preferences"))
        .expect("conventions section merged");
    assert!(!conventions.contains("DIFFERENT people"));
}

#[test]
fn section_merge_prompt_keeps_recency_away_from_identity_attributes() {
    // The recency clause must stay qualified: unqualified, it applies to names
    // and ages too, which is what manufactured the false biography.
    assert!(SECTION_MERGE_SYSTEM_PROMPT.contains("contradictions about tools, projects"));
}
