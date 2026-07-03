// HalluScribe - unit tests for the profile distiller reduce step (merge.rs).

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fact(section: ProfileSection, text: &str, date: &str) -> ProfileFact {
    ProfileFact {
        section,
        fact: text.to_string(),
        evidence: vec!["s1".to_string()],
        date: date.to_string(),
    }
}

fn canned_sections_response() -> Value {
    serde_json::json!({
        "identity": "Rust/Tauri developer.",
        "projects": "HalluScribe [s1]",
        "conventions": "",
        "recurring_problems": "",
        "communication_style": "",
        "timeline": ""
    })
}

fn canned_personal_sections_response() -> Value {
    serde_json::json!({
        "identity": "Rust/Tauri developer.",
        "personal_context": "Enjoys hiking.",
        "projects": "HalluScribe [s1]",
        "conventions": "",
        "recurring_problems": "",
        "communication_style": "",
        "timeline": ""
    })
}

#[test]
fn run_reduce_assembles_sections_from_fake_tool_call() {
    let facts = vec![fact(
        ProfileSection::Projects,
        "Building HalluScribe.",
        "2026-06-01",
    )];
    let tool_call: &ToolCallFn = &|_sys, _user, _tool, _max| Ok(canned_sections_response());
    let sections =
        run_reduce(ProfileScope::Work, None, &facts, tool_call, &mut Vec::new()).unwrap();
    assert_eq!(sections.identity, "Rust/Tauri developer.");
    assert_eq!(sections.projects, "HalluScribe [s1]");
}

#[test]
fn run_reduce_passes_previous_profile_into_user_content() {
    let facts = vec![fact(
        ProfileSection::Identity,
        "Uses Windows.",
        "2026-06-01",
    )];
    let seen_previous = std::sync::Mutex::new(String::new());
    let tool_call: &ToolCallFn = &|_sys, user, _tool, _max| {
        *seen_previous.lock().unwrap() = user.to_string();
        Ok(canned_sections_response())
    };
    run_reduce(
        ProfileScope::Work,
        Some("# User Profile — old"),
        &facts,
        tool_call,
        &mut Vec::new(),
    )
    .unwrap();
    assert!(seen_previous
        .lock()
        .unwrap()
        .contains("# User Profile — old"));
}

#[test]
fn oversized_facts_trigger_chunked_consolidation_before_final_merge() {
    // 100 realistic-length facts (~70K chars total) in a single section:
    // past the 60K threshold, so consolidation must run — and in several
    // bounded chunks, not one unbounded call.
    let facts: Vec<ProfileFact> = (0..100)
        .map(|i| {
            fact(
                ProfileSection::RecurringProblems,
                &format!("problem {i}: {}", "x".repeat(680)),
                "2026-06-01",
            )
        })
        .collect();
    let consolidate_calls = AtomicUsize::new(0);
    let max_input_len = AtomicUsize::new(0);
    let tool_call: &ToolCallFn = &|_sys, user, tool, _max| {
        let name = tool["function"]["name"].as_str().unwrap_or("");
        if name == "save_section_consolidated" {
            consolidate_calls.fetch_add(1, Ordering::SeqCst);
            max_input_len.fetch_max(user.len(), Ordering::SeqCst);
            Ok(serde_json::json!({"consolidated": "condensed problem list [s1]"}))
        } else {
            Ok(canned_sections_response())
        }
    };
    let sections =
        run_reduce(ProfileScope::Work, None, &facts, tool_call, &mut Vec::new()).unwrap();
    assert!(
        consolidate_calls.load(Ordering::SeqCst) >= 10,
        "expected many bounded chunks, got {}",
        consolidate_calls.load(Ordering::SeqCst)
    );
    // No consolidate call may see more than one chunk of bullets (plus
    // the short prompt preamble).
    assert!(max_input_len.load(Ordering::SeqCst) < CONSOLIDATE_CHUNK_CHARS + 200);
    assert_eq!(sections.identity, "Rust/Tauri developer.");
}

#[test]
fn failed_consolidate_chunk_keeps_raw_facts_and_warns() {
    let facts = vec![fact(
        ProfileSection::RecurringProblems,
        &"x".repeat(70_000),
        "2026-06-01",
    )];
    let merge_input = std::sync::Mutex::new(String::new());
    let tool_call: &ToolCallFn = &|_sys, user, tool, _max| {
        let name = tool["function"]["name"].as_str().unwrap_or("");
        if name == "save_section_consolidated" {
            Err(ProfileError::BadToolCall("truncated".to_string()))
        } else {
            *merge_input.lock().unwrap() = user.to_string();
            Ok(canned_sections_response())
        }
    };
    let mut warnings = Vec::new();
    let sections = run_reduce(ProfileScope::Work, None, &facts, tool_call, &mut warnings).unwrap();
    // Reduce still completes; every failed chunk is reported and its raw
    // bullets flow into the final merge input.
    assert_eq!(sections.identity, "Rust/Tauri developer.");
    assert!(!warnings.is_empty());
    assert!(
        warnings[0].contains("kept raw facts"),
        "got: {}",
        warnings[0]
    );
    assert!(merge_input.lock().unwrap().contains("xxxx"));
}

#[test]
fn chunk_lines_splits_on_line_boundaries() {
    let text = (0..10)
        .map(|i| format!("- fact {i} {}", "y".repeat(50)))
        .collect::<Vec<_>>()
        .join("\n");
    let chunks = chunk_lines(&text, 150);
    assert!(chunks.len() > 1);
    for chunk in &chunks {
        assert!(chunk.len() <= 150);
        assert!(chunk.starts_with("- fact"));
    }
    assert_eq!(chunks.join("\n"), text);
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
fn small_facts_skip_consolidation() {
    let facts = vec![fact(ProfileSection::Identity, "Short fact.", "2026-06-01")];
    let call_count = AtomicUsize::new(0);
    let tool_call: &ToolCallFn = &|_sys, _user, _tool, _max| {
        call_count.fetch_add(1, Ordering::SeqCst);
        Ok(canned_sections_response())
    };
    run_reduce(ProfileScope::Work, None, &facts, tool_call, &mut Vec::new()).unwrap();
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
}

#[test]
fn parse_sections_errors_on_missing_field() {
    let value = serde_json::json!({"identity": "x"});
    let error = parse_sections(ProfileScope::Work, &value).unwrap_err();
    assert!(matches!(error, ProfileError::BadToolCall(_)));
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
fn work_scope_tool_schema_excludes_personal_context() {
    let tool = save_user_profile_tool(ProfileScope::Work);
    let properties = tool["function"]["parameters"]["properties"]
        .as_object()
        .unwrap();
    assert_eq!(properties.len(), 6);
    assert!(!properties.contains_key("personal_context"));
}

#[test]
fn personal_scope_tool_schema_includes_personal_context() {
    let tool = save_user_profile_tool(ProfileScope::Personal);
    let properties = tool["function"]["parameters"]["properties"]
        .as_object()
        .unwrap();
    assert_eq!(properties.len(), 7);
    assert!(properties.contains_key("personal_context"));
}

#[test]
fn personal_scope_run_reduce_reads_personal_context_field() {
    let facts = vec![fact(
        ProfileSection::PersonalContext,
        "Enjoys hiking.",
        "2026-06-01",
    )];
    let tool_call: &ToolCallFn =
        &|_sys, _user, _tool, _max| Ok(canned_personal_sections_response());
    let sections = run_reduce(
        ProfileScope::Personal,
        None,
        &facts,
        tool_call,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(sections.personal_context, "Enjoys hiking.");
}

#[test]
fn work_scope_run_reduce_errors_when_personal_context_missing_is_fine() {
    // Work scope's tool schema never requests personal_context, and
    // parse_sections must not fail looking for a field it never asked for.
    let facts = vec![fact(ProfileSection::Identity, "x", "2026-06-01")];
    let tool_call: &ToolCallFn = &|_sys, _user, _tool, _max| Ok(canned_sections_response());
    let sections =
        run_reduce(ProfileScope::Work, None, &facts, tool_call, &mut Vec::new()).unwrap();
    assert_eq!(sections.personal_context, "");
}
