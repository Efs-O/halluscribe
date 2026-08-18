// HalluScribe - end-to-end tests for stopping a profile refresh (GROK_IMPORT
// PLAN Part II): a cancelled run must write nothing, leave the watermark
// alone, and keep `pending_facts.json` so the next run resumes from it.
// Split out of refresh_tests.rs to keep both files under the repo's LOC cap.

use super::pending::load_pending;
use super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("halluscribe_profile_cancel_{name}"));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn session_json(id: &str, ts: &str) -> serde_json::Value {
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
        "provider": "claude_code",
        "fill_estimated": false,
        "transcript_hash": ""
    })
}

/// `count` sessions, each with a body on disk. Callers that need several map
/// batches ask for more than `BATCH_SIZE` of them.
fn seed(dir: &std::path::Path, count: usize) {
    let mut sessions = Vec::new();
    for idx in 0..count {
        let id = format!("s{idx:03}");
        fs::write(dir.join(format!("{id}.md")), "body").unwrap();
        sessions.push(session_json(
            &id,
            &format!("2026-06-{:02}T00:00:00+00:00", 1 + idx % 28),
        ));
    }
    fs::write(
        dir.join("index.json"),
        serde_json::to_string_pretty(&serde_json::json!({ "sessions": sessions })).unwrap(),
    )
    .unwrap();
}

fn first_session_id(user: &str) -> String {
    user.lines()
        .find_map(|line| line.strip_prefix("Session id: "))
        .unwrap_or("s000")
        .trim()
        .to_string()
}

fn canned(user: &str, tool: &serde_json::Value) -> serde_json::Value {
    match tool["function"]["name"].as_str().unwrap_or("") {
        "save_profile_facts" => serde_json::json!({
            "facts": [{
                "section": "projects",
                "fact": "Works on proj.",
                "evidence": [first_session_id(user)],
                "date": "2026-06-01"
            }]
        }),
        _ => serde_json::json!({ "content": "merged prose" }),
    }
}

fn sources() -> Vec<String> {
    vec!["claude_code".to_string()]
}

/// Stopping before the first batch: nothing mapped, nothing written, and the
/// run is reported as cancelled rather than as a failure.
#[test]
fn cancel_before_the_first_batch_writes_nothing() {
    let dir = tmp_dir("before_first_batch");
    seed(&dir, 3);
    let calls = AtomicUsize::new(0);
    let tool_call: &ToolCallFn = &|_sys, user, tool, _max| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(canned(user, tool))
    };
    let outcome = run_refresh(
        &dir,
        ProfileScope::Work,
        &sources(),
        true,
        tool_call,
        &AtomicBool::new(true),
        |_, _, _| {},
    )
    .unwrap();

    assert!(outcome.cancelled);
    assert_eq!(outcome.failed_batches, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0, "no model call was made");
    assert!(read_profile_md(&dir, ProfileScope::Work).is_none());
    assert!(latest_digest(&dir, ProfileScope::Work).is_none());
    assert!(load_meta(&dir, ProfileScope::Work)
        .last_distilled_ts
        .is_empty());
}

/// Stopping mid-mapping keeps the snapshot written after the last finished
/// batch, and the re-run resumes from it instead of re-mapping those sessions.
#[test]
fn cancel_during_mapping_keeps_the_snapshot_and_resumes() {
    let dir = tmp_dir("during_mapping");
    // Three batches worth, so one batch can finish before the stop lands.
    seed(&dir, BATCH_SIZE * 3);
    let cancel = AtomicBool::new(false);
    let map_calls = AtomicUsize::new(0);
    let tool_call: &ToolCallFn = &|_sys, user, tool, _max| {
        if tool["function"]["name"].as_str() == Some("save_profile_facts") {
            // Stop from inside the first batch: the flag is only read at the
            // top of the next iteration, so batch 1 still completes and is
            // saved to the snapshot.
            map_calls.fetch_add(1, Ordering::SeqCst);
            cancel.store(true, Ordering::SeqCst);
        }
        Ok(canned(user, tool))
    };
    let outcome = run_refresh(
        &dir,
        ProfileScope::Work,
        &sources(),
        true,
        tool_call,
        &cancel,
        |_, _, _| {},
    )
    .unwrap();

    assert!(outcome.cancelled);
    assert_eq!(map_calls.load(Ordering::SeqCst), 1, "only batch 1 ran");
    assert!(read_profile_md(&dir, ProfileScope::Work).is_none());
    assert!(load_meta(&dir, ProfileScope::Work)
        .last_distilled_ts
        .is_empty());

    let pending = load_pending(&dir, ProfileScope::Work).unwrap().unwrap();
    assert_eq!(pending.session_ids.len(), BATCH_SIZE);

    // Re-run without a stop: the snapshot sessions are not re-mapped.
    let resumed_calls = AtomicUsize::new(0);
    let resumed_call: &ToolCallFn = &|_sys, user, tool, _max| {
        if tool["function"]["name"].as_str() == Some("save_profile_facts") {
            resumed_calls.fetch_add(1, Ordering::SeqCst);
        }
        Ok(canned(user, tool))
    };
    let outcome = run_refresh(
        &dir,
        ProfileScope::Work,
        &sources(),
        true,
        resumed_call,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();

    assert!(!outcome.cancelled);
    assert_eq!(
        resumed_calls.load(Ordering::SeqCst),
        2,
        "batches 2 and 3 only"
    );
    assert_eq!(outcome.session_count, BATCH_SIZE * 3);
    assert!(read_profile_md(&dir, ProfileScope::Work).is_some());
    assert!(load_pending(&dir, ProfileScope::Work).unwrap().is_none());
}

/// Stopping between reduce calls must not write a half-merged profile: the
/// previous profile.md, its meta and the digest stay byte-identical.
#[test]
fn cancel_during_merging_leaves_the_previous_profile_untouched() {
    let dir = tmp_dir("during_merging");
    seed(&dir, 2);

    // A completed run first, to have something on disk that a bad cancel
    // could overwrite.
    let tool_call: &ToolCallFn = &|_sys, user, tool, _max| Ok(canned(user, tool));
    run_refresh(
        &dir,
        ProfileScope::Work,
        &sources(),
        true,
        tool_call,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    let before_md = read_profile_md(&dir, ProfileScope::Work).unwrap();
    let before_meta = load_meta(&dir, ProfileScope::Work);
    let before_digest = latest_digest(&dir, ProfileScope::Work).unwrap();
    assert!(!before_meta.last_distilled_ts.is_empty());

    // A second (full) run that maps fine, then stops as the reduce begins.
    let cancel = AtomicBool::new(false);
    let section_calls = AtomicUsize::new(0);
    let stopping_call: &ToolCallFn = &|_sys, user, tool, _max| {
        if tool["function"]["name"].as_str() == Some("save_profile_section") {
            section_calls.fetch_add(1, Ordering::SeqCst);
        }
        cancel.store(true, Ordering::SeqCst);
        Ok(canned(user, tool))
    };
    let outcome = run_refresh(
        &dir,
        ProfileScope::Work,
        &sources(),
        true,
        stopping_call,
        &cancel,
        |_, _, _| {},
    )
    .unwrap();

    assert!(outcome.cancelled);
    assert_eq!(
        section_calls.load(Ordering::SeqCst),
        0,
        "the reduce stopped at its first safe boundary"
    );
    assert_eq!(
        read_profile_md(&dir, ProfileScope::Work).unwrap(),
        before_md
    );
    assert_eq!(
        latest_digest(&dir, ProfileScope::Work).unwrap(),
        before_digest
    );
    let after_meta = load_meta(&dir, ProfileScope::Work);
    assert_eq!(after_meta.last_distilled_ts, before_meta.last_distilled_ts);
    assert_eq!(after_meta.generated_at, before_meta.generated_at);
    // The mapping work survives for the re-run.
    assert!(load_pending(&dir, ProfileScope::Work).unwrap().is_some());
}

/// Stopping one scope leaves the other scope profile completely alone.
#[test]
fn cancelling_work_does_not_touch_personal() {
    let dir = tmp_dir("scopes");
    seed(&dir, 2);
    let tool_call: &ToolCallFn = &|_sys, user, tool, _max| Ok(canned(user, tool));
    run_refresh(
        &dir,
        ProfileScope::Personal,
        &sources(),
        true,
        tool_call,
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    let personal_before = read_profile_md(&dir, ProfileScope::Personal).unwrap();

    let outcome = run_refresh(
        &dir,
        ProfileScope::Work,
        &sources(),
        true,
        tool_call,
        &AtomicBool::new(true),
        |_, _, _| {},
    )
    .unwrap();

    assert!(outcome.cancelled);
    assert!(read_profile_md(&dir, ProfileScope::Work).is_none());
    assert_eq!(
        read_profile_md(&dir, ProfileScope::Personal).unwrap(),
        personal_before
    );
}
