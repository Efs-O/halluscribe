// HalluScribe - end-to-end tests for the profile refresh orchestration
// (`run_refresh`) against a temp archive with a fake inference closure.

use super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "halluscribe_profile_refresh_{}_{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn write_index(dir: &std::path::Path, sessions: &[serde_json::Value]) {
    let index = serde_json::json!({ "sessions": sessions });
    fs::write(
        dir.join("index.json"),
        serde_json::to_string_pretty(&index).unwrap(),
    )
    .unwrap();
}

fn session_json(id: &str, provider: &str, ts: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "project": "proj",
        "date": &ts[..10],
        "title": format!("Session {id}"),
        "tool": "Claude Code",
        "fill_pct": 90.0,
        "session_timestamp": ts,
        "updated_at": "",
        "session_type": "building",
        "error_tags": [],
        "topic_tags": [],
        "archive_path": format!("{id}.md"),
        "source_jsonl": "",
        "source_size_bytes": 0,
        "provider": provider,
        "fill_estimated": false,
        "transcript_hash": ""
    })
}

fn canned_response(tool: &serde_json::Value) -> serde_json::Value {
    match tool["function"]["name"].as_str().unwrap_or("") {
        "save_profile_facts" => serde_json::json!({
            "facts": [{
                "section": "projects",
                "fact": "Works on proj.",
                "evidence": ["s1"],
                "date": "2026-06-01"
            }]
        }),
        // Per-section merge (`save_profile_section`): one string back.
        _ => serde_json::json!({ "content": "proj [s1]" }),
    }
}

#[test]
fn full_refresh_writes_profile_meta_and_digest_and_advances_watermark() {
    let dir = tmp_dir("full");
    fs::write(dir.join("s1.md"), "body one").unwrap();
    fs::write(dir.join("s2.md"), "body two").unwrap();
    write_index(
        &dir,
        &[
            session_json("s1", "claude_code", "2026-06-10T00:00:00+00:00"),
            session_json("s2", "codex", "2026-06-01T00:00:00+00:00"),
            session_json("x1", "chatgpt", "2026-06-20T00:00:00+00:00"),
        ],
    );
    let sources = vec!["claude_code".to_string(), "codex".to_string()];
    let tool_call: &ToolCallFn = &|_sys, _user, tool, _max| Ok(canned_response(tool));
    let mut stages = Vec::new();
    let outcome = run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        true,
        tool_call,
        &AtomicBool::new(false),
        |cur, total, stage| {
            stages.push((cur, total, stage.as_str()));
        },
    )
    .unwrap();

    assert_eq!(outcome.session_count, 2); // chatgpt excluded by consent
    assert_eq!(outcome.facts_count, 1);
    assert!(outcome.errors.is_empty());

    let profile = read_profile_md(&dir, ProfileScope::Work).unwrap();
    assert!(profile.contains("from 2 sessions"));
    assert!(profile.contains("proj [s1]"));

    let meta = load_meta(&dir, ProfileScope::Work);
    // Watermark advances to the newest consented session, not the excluded one.
    assert_eq!(meta.last_distilled_ts, "2026-06-10T00:00:00+00:00");
    assert_eq!(meta.sources, sources);

    assert!(latest_digest(&dir, ProfileScope::Work)
        .unwrap()
        .contains("2 sessions distilled"));
    assert!(stages.iter().any(|s| s.2 == "mapping"));
    assert!(stages.iter().any(|s| s.2 == "merging"));
    assert!(stages.iter().any(|s| s.2 == "writing"));
}

#[test]
fn incremental_refresh_only_maps_sessions_newer_than_watermark() {
    let dir = tmp_dir("incremental");
    fs::write(dir.join("s1.md"), "body one").unwrap();
    write_index(
        &dir,
        &[session_json(
            "s1",
            "claude_code",
            "2026-06-10T00:00:00+00:00",
        )],
    );
    let sources = vec!["claude_code".to_string()];
    let map_calls = AtomicUsize::new(0);
    let tool_call: &ToolCallFn = &|_sys, _user, tool, _max| {
        if tool["function"]["name"].as_str() == Some("save_profile_facts") {
            map_calls.fetch_add(1, Ordering::SeqCst);
        }
        Ok(canned_response(tool))
    };

    // First (full) refresh distills s1 and sets the watermark.
    run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        true,
        tool_call,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(map_calls.load(Ordering::SeqCst), 1);

    // Second (incremental) refresh finds nothing newer: no map calls, and the
    // watermark stays where it was.
    let outcome = run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        false,
        tool_call,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(map_calls.load(Ordering::SeqCst), 1);
    assert_eq!(outcome.session_count, 0);
    assert_eq!(
        load_meta(&dir, ProfileScope::Work).last_distilled_ts,
        "2026-06-10T00:00:00+00:00"
    );

    // A newer session arrives; incremental refresh maps only it and advances.
    fs::write(dir.join("s2.md"), "body two").unwrap();
    write_index(
        &dir,
        &[
            session_json("s1", "claude_code", "2026-06-10T00:00:00+00:00"),
            session_json("s2", "claude_code", "2026-06-15T00:00:00+00:00"),
        ],
    );
    let outcome = run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        false,
        tool_call,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(map_calls.load(Ordering::SeqCst), 2);
    assert_eq!(outcome.session_count, 1);
    assert_eq!(
        load_meta(&dir, ProfileScope::Work).last_distilled_ts,
        "2026-06-15T00:00:00+00:00"
    );
}

#[test]
fn failed_map_batch_is_collected_as_error_not_fatal() {
    let dir = tmp_dir("map_error");
    fs::write(dir.join("s1.md"), "body one").unwrap();
    write_index(
        &dir,
        &[session_json(
            "s1",
            "claude_code",
            "2026-06-10T00:00:00+00:00",
        )],
    );
    let sources = vec!["claude_code".to_string()];
    let tool_call: &ToolCallFn = &|_sys, _user, tool, _max| {
        if tool["function"]["name"].as_str() == Some("save_profile_facts") {
            Ok(serde_json::json!({"garbage": true}))
        } else {
            Ok(canned_response(tool))
        }
    };
    let outcome = run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        true,
        tool_call,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(outcome.errors.len(), 1);
    assert!(outcome.errors[0].contains("missing facts array"));
    // The reduce/write stages still ran: a profile exists.
    assert!(read_profile_md(&dir, ProfileScope::Work).is_some());
    // But the watermark must NOT advance past the failed batch, so the next
    // incremental run retries those sessions instead of skipping them forever.
    assert_eq!(load_meta(&dir, ProfileScope::Work).last_distilled_ts, "");
}

#[test]
fn failed_section_merge_falls_back_and_still_writes_profile() {
    let dir = tmp_dir("merge_error");
    fs::write(dir.join("s1.md"), "body one").unwrap();
    write_index(
        &dir,
        &[session_json(
            "s1",
            "claude_code",
            "2026-06-10T00:00:00+00:00",
        )],
    );
    let sources = vec!["claude_code".to_string()];
    let tool_call: &ToolCallFn = &|_sys, _user, tool, _max| {
        if tool["function"]["name"].as_str() == Some("save_profile_section") {
            Err(ProfileError::BadToolCall("truncated mid-JSON".to_string()))
        } else {
            Ok(canned_response(tool))
        }
    };
    let outcome = run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        true,
        tool_call,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    // The reduce no longer fails outright: the broken section keeps its raw
    // bullet facts (warned), the profile IS written, and the map work is
    // visible in the outcome. Every session WAS distilled, so the fallback
    // warning must not hold the watermark back (that would re-map the whole
    // selection on every future run just to retry one merge call).
    assert_eq!(outcome.session_count, 1);
    assert_eq!(outcome.facts_count, 1);
    assert_eq!(outcome.errors.len(), 1);
    assert!(
        outcome.errors[0].contains("merge section projects:")
            && outcome.errors[0].contains("kept raw facts"),
        "got: {}",
        outcome.errors[0]
    );
    let profile = read_profile_md(&dir, ProfileScope::Work).unwrap();
    assert!(profile.contains("Works on proj."));
    assert_eq!(
        load_meta(&dir, ProfileScope::Work).last_distilled_ts,
        "2026-06-10T00:00:00+00:00"
    );
}

#[test]
fn empty_selection_returns_early_without_touching_profile() {
    let dir = tmp_dir("empty_selection");
    fs::write(dir.join("s1.md"), "body one").unwrap();
    write_index(
        &dir,
        &[session_json(
            "s1",
            "claude_code",
            "2026-06-10T00:00:00+00:00",
        )],
    );
    let sources = vec!["claude_code".to_string()];
    let tool_call: &ToolCallFn = &|_sys, _user, tool, _max| Ok(canned_response(tool));
    run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        true,
        tool_call,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    let profile_before = read_profile_md(&dir, ProfileScope::Work).unwrap();
    let meta_before = load_meta(&dir, ProfileScope::Work);

    // Incremental run with nothing new: no inference, no writes.
    let calls = AtomicUsize::new(0);
    let counting: &ToolCallFn = &|_sys, _user, tool, _max| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(canned_response(tool))
    };
    let outcome = run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        false,
        counting,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(outcome.session_count, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        read_profile_md(&dir, ProfileScope::Work).unwrap(),
        profile_before
    );
    assert_eq!(
        load_meta(&dir, ProfileScope::Work).generated_at,
        meta_before.generated_at
    );
}

/// Capture every per-section merge input a run produces.
fn recording_tool_call(
    seen: &std::sync::Mutex<Vec<String>>,
) -> impl Fn(&str, &str, &serde_json::Value, u32) -> Result<serde_json::Value, ProfileError>
       + Send
       + Sync
       + '_ {
    move |_sys, user, tool, _max| {
        if tool["function"]["name"].as_str() == Some("save_profile_section") {
            seen.lock().unwrap().push(user.to_string());
        }
        Ok(canned_response(tool))
    }
}

#[test]
fn full_refresh_does_not_feed_the_previous_profile_back_into_the_merge() {
    // Regression guard for the 2026-08-08 fix: `full` used to clear only the
    // watermark, while the merge still received the old profile.md as
    // "Previous content". Since run_reduce keeps a section verbatim when that
    // section gets no new facts, a misattributed claim survived a full rebuild
    // completely untouched — the rebuild could never repair it.
    let dir = tmp_dir("full_ignores_previous");
    fs::write(dir.join("s1.md"), "body one").unwrap();
    write_index(
        &dir,
        &[session_json(
            "s1",
            "claude_code",
            "2026-06-10T00:00:00+00:00",
        )],
    );
    let sources = vec!["claude_code".to_string()];
    let seen = std::sync::Mutex::new(Vec::<String>::new());
    let recorder = recording_tool_call(&seen);
    let tool_call: &ToolCallFn = &recorder;

    // First full run establishes a profile on disk.
    run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        true,
        tool_call,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    assert!(read_profile_md(&dir, ProfileScope::Work)
        .unwrap()
        .contains("proj [s1]"));
    seen.lock().unwrap().clear();

    // A second FULL run must re-derive from the sessions alone.
    run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        true,
        tool_call,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    let inputs = seen.lock().unwrap();
    assert!(!inputs.is_empty(), "expected at least one merge call");
    for input in inputs.iter() {
        assert!(
            input.contains("Previous content:\n(none)"),
            "full rebuild leaked the previous profile into the merge: {input}"
        );
    }
}

#[test]
fn incremental_refresh_still_merges_against_the_previous_profile() {
    // The counterpart guard: only `full` discards the previous prose. An
    // incremental run must keep building on it, or every refresh would drop
    // everything distilled before its watermark.
    let dir = tmp_dir("incremental_keeps_previous");
    fs::write(dir.join("s1.md"), "body one").unwrap();
    fs::write(dir.join("s2.md"), "body two").unwrap();
    write_index(
        &dir,
        &[session_json(
            "s1",
            "claude_code",
            "2026-06-10T00:00:00+00:00",
        )],
    );
    let sources = vec!["claude_code".to_string()];
    let seen = std::sync::Mutex::new(Vec::<String>::new());
    let recorder = recording_tool_call(&seen);
    let tool_call: &ToolCallFn = &recorder;

    run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        true,
        tool_call,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    seen.lock().unwrap().clear();

    // A newer session gives the incremental run something to map.
    write_index(
        &dir,
        &[
            session_json("s1", "claude_code", "2026-06-10T00:00:00+00:00"),
            session_json("s2", "claude_code", "2026-06-20T00:00:00+00:00"),
        ],
    );
    run_refresh(
        &dir,
        ProfileScope::Work,
        &sources,
        false,
        tool_call,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();

    let inputs = seen.lock().unwrap();
    let projects = inputs
        .iter()
        .find(|c| c.starts_with("Section: Active Projects"))
        .expect("projects section merged");
    assert!(
        projects.contains("proj [s1]"),
        "incremental run lost the previous profile: {projects}"
    );
}
