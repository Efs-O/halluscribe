// HalluScribe - unit tests for the pending map-facts persistence (pending.rs).

use super::super::types::{ProfileFact, ProfileSection};
use super::*;
use std::fs;
use std::path::PathBuf;

fn tmp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("halluscribe_profile_pending_{name}"));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn sample_pending() -> PendingFacts {
    PendingFacts {
        session_ids: vec!["s1".to_string(), "s2".to_string()],
        facts: vec![ProfileFact {
            section: ProfileSection::Conventions,
            fact: "Prefers Rust.".to_string(),
            evidence: vec!["s1".to_string()],
            date: "2026-06-01".to_string(),
        }],
        last_ts: "2026-06-10T00:00:00+00:00".to_string(),
        created_at: "2026-07-04T00:00:00Z".to_string(),
    }
}

#[test]
fn save_then_load_round_trips() {
    let dir = tmp_dir("round_trip");
    save_pending(&dir, ProfileScope::Work, &sample_pending()).unwrap();
    let loaded = load_pending(&dir, ProfileScope::Work).unwrap().unwrap();
    assert_eq!(loaded.session_ids, vec!["s1", "s2"]);
    assert_eq!(loaded.facts.len(), 1);
    assert_eq!(loaded.facts[0].section, ProfileSection::Conventions);
    assert_eq!(loaded.last_ts, "2026-06-10T00:00:00+00:00");
}

#[test]
fn load_returns_none_when_absent() {
    let dir = tmp_dir("absent");
    assert!(load_pending(&dir, ProfileScope::Work).unwrap().is_none());
}

#[test]
fn corrupt_file_returns_warning_not_panic() {
    let dir = tmp_dir("corrupt");
    let scope_path = scope_dir(&dir, ProfileScope::Work);
    fs::create_dir_all(&scope_path).unwrap();
    fs::write(scope_path.join("pending_facts.json"), "{ not json").unwrap();
    let warning = load_pending(&dir, ProfileScope::Work).unwrap_err();
    assert!(warning.contains("corrupt"), "got: {warning}");
}

#[test]
fn clear_pending_removes_file_and_tolerates_missing() {
    let dir = tmp_dir("clear");
    save_pending(&dir, ProfileScope::Work, &sample_pending()).unwrap();
    assert!(load_pending(&dir, ProfileScope::Work).unwrap().is_some());
    clear_pending(&dir, ProfileScope::Work);
    assert!(load_pending(&dir, ProfileScope::Work).unwrap().is_none());
    // Second clear on a missing file is a no-op.
    clear_pending(&dir, ProfileScope::Work);
}

#[test]
fn has_pending_facts_requires_at_least_one_fact() {
    let dir = tmp_dir("has_facts");
    assert!(!has_pending_facts(&dir, ProfileScope::Work));

    let empty = PendingFacts {
        facts: Vec::new(),
        ..sample_pending()
    };
    save_pending(&dir, ProfileScope::Work, &empty).unwrap();
    assert!(!has_pending_facts(&dir, ProfileScope::Work));

    save_pending(&dir, ProfileScope::Work, &sample_pending()).unwrap();
    assert!(has_pending_facts(&dir, ProfileScope::Work));
}

#[test]
fn has_pending_facts_is_false_for_corrupt_file() {
    let dir = tmp_dir("has_facts_corrupt");
    let scope_path = scope_dir(&dir, ProfileScope::Work);
    fs::create_dir_all(&scope_path).unwrap();
    fs::write(scope_path.join("pending_facts.json"), "garbage").unwrap();
    assert!(!has_pending_facts(&dir, ProfileScope::Work));
}

#[test]
fn scopes_have_independent_pending_files() {
    let dir = tmp_dir("scoped");
    save_pending(&dir, ProfileScope::Personal, &sample_pending()).unwrap();
    assert!(load_pending(&dir, ProfileScope::Work).unwrap().is_none());
    assert!(load_pending(&dir, ProfileScope::Personal)
        .unwrap()
        .is_some());
}
