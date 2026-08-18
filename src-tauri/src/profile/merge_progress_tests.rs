// HalluScribe - unit tests for the reduce step's two-pass path on a large
// archive: chunked consolidation, its failure fallback, and the progress
// accounting that must cover every model call the two passes go on to make.
// Split out of merge_tests.rs to keep both files under the repo's 350-LOC cap.

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

/// Enough facts in one section (~70K chars) to cross
/// `CONSOLIDATION_THRESHOLD_CHARS`, so the reduce takes the two-pass path and
/// makes many calls rather than one.
fn oversized_facts() -> Vec<ProfileFact> {
    (0..100)
        .map(|i| {
            fact(
                ProfileSection::RecurringProblems,
                &format!("problem {i}: {}", "x".repeat(680)),
                "2026-06-01",
            )
        })
        .collect()
}

#[test]
fn progress_reports_one_step_per_model_call_numbered_to_a_stable_total() {
    let facts = oversized_facts();
    let calls = AtomicUsize::new(0);
    let tool_call: &ToolCallFn = &|_sys, _user, tool, _max| {
        calls.fetch_add(1, Ordering::SeqCst);
        if tool["function"]["name"].as_str() == Some("save_section_consolidated") {
            Ok(serde_json::json!({"consolidated": "condensed [s1]"}))
        } else {
            Ok(serde_json::json!({ "content": "merged [s1]" }))
        }
    };
    let seen = Mutex::new(Vec::<(usize, usize)>::new());
    run_reduce(
        ProfileScope::Work,
        None,
        &facts,
        tool_call,
        &mut Vec::new(),
        &AtomicBool::new(false),
        &mut |current, total| seen.lock().unwrap().push((current, total)),
    )
    .unwrap();

    let seen = seen.lock().unwrap();
    let calls = calls.load(Ordering::SeqCst);
    // The consolidation path is what makes this stage long; if it did not run
    // the test would prove nothing about multi-call progress.
    assert!(
        calls > 10,
        "expected the consolidation path, got {calls} calls"
    );
    // Every call is announced, numbered 1..=total, and the total never moves —
    // a total that grew mid-stage would make the bar walk backwards.
    let steps: Vec<usize> = seen.iter().map(|(current, _)| *current).collect();
    assert_eq!(steps, (1..=calls).collect::<Vec<usize>>());
    assert!(seen.iter().all(|(_, total)| *total == calls));
}

#[test]
fn sections_without_facts_cost_no_steps() {
    // Only one section has facts and the archive is small enough to skip
    // consolidation, so the whole reduce is a single call — the progress total
    // must not count the sections that were kept verbatim without a call.
    let facts = vec![fact(
        ProfileSection::Projects,
        "Ships HalluScribe.",
        "2026-06-01",
    )];
    let tool_call: &ToolCallFn =
        &|_sys, _user, _tool, _max| Ok(serde_json::json!({ "content": "merged [s1]" }));
    let seen = Mutex::new(Vec::<(usize, usize)>::new());
    run_reduce(
        ProfileScope::Work,
        None,
        &facts,
        tool_call,
        &mut Vec::new(),
        &AtomicBool::new(false),
        &mut |current, total| seen.lock().unwrap().push((current, total)),
    )
    .unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![(1, 1)]);
}

#[test]
fn oversized_facts_trigger_chunked_consolidation_before_section_merge() {
    // Past the 60K threshold, so consolidation must run — and in several
    // bounded chunks, not one unbounded call.
    let facts = oversized_facts();
    let consolidate_calls = AtomicUsize::new(0);
    let max_input_len = AtomicUsize::new(0);
    let tool_call: &ToolCallFn = &|_sys, user, tool, _max| {
        let name = tool["function"]["name"].as_str().unwrap_or("");
        if name == "save_section_consolidated" {
            consolidate_calls.fetch_add(1, Ordering::SeqCst);
            max_input_len.fetch_max(user.len(), Ordering::SeqCst);
            Ok(serde_json::json!({"consolidated": "condensed problem list [s1]"}))
        } else {
            Ok(serde_json::json!({ "content": "merged problems [s1]" }))
        }
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
    assert!(
        consolidate_calls.load(Ordering::SeqCst) >= 10,
        "expected many bounded chunks, got {}",
        consolidate_calls.load(Ordering::SeqCst)
    );
    // No consolidate call may see more than one chunk of bullets (plus
    // the short prompt preamble).
    assert!(max_input_len.load(Ordering::SeqCst) < CONSOLIDATE_CHUNK_CHARS + 200);
    assert_eq!(sections.recurring_problems, "merged problems [s1]");
}

#[test]
fn failed_consolidate_chunk_keeps_raw_facts_and_warns() {
    let facts = vec![fact(
        ProfileSection::RecurringProblems,
        &"x".repeat(70_000),
        "2026-06-01",
    )];
    let merge_input = Mutex::new(String::new());
    let tool_call: &ToolCallFn = &|_sys, user, tool, _max| {
        let name = tool["function"]["name"].as_str().unwrap_or("");
        if name == "save_section_consolidated" {
            Err(ProfileError::BadToolCall("truncated".to_string()))
        } else {
            *merge_input.lock().unwrap() = user.to_string();
            Ok(serde_json::json!({ "content": "merged problems" }))
        }
    };
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
    // Reduce still completes; every failed chunk is reported and its raw
    // bullets flow into the section-merge input.
    assert_eq!(sections.recurring_problems, "merged problems");
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
