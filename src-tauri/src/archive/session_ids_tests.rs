// HalluScribe - tests for session id proposal and stable collision resolution.

use super::session_ids::lookup_for_tests;
use super::{session_id, CapturedManifest, CapturedRecord, SessionLookup, Tombstone};
use std::collections::BTreeMap;
use std::path::Path;

const STORED_HASH: &str = "0123456789abcdef";

fn captured(records: &[(&str, &str)]) -> CapturedManifest {
    records
        .iter()
        .map(|(id, source)| {
            let record = CapturedRecord {
                source_path: source.to_string(),
                size: 1,
                mtime_secs: 0,
                captured_at: "2026-09-01T00:00:00Z".to_string(),
            };
            (id.to_string(), record)
        })
        .collect()
}

fn lookup(records: &[(&str, &str)]) -> SessionLookup {
    lookup_for_tests(Vec::new(), captured(records), BTreeMap::new())
}

#[test]
fn collision_resolver_keeps_legacy_owner_and_suffixes_the_new_source() {
    let lookup = lookup(&[("session", "C:/one/session.jsonl")]);
    let first = Path::new("C:/one/session.jsonl");
    let second = Path::new("C:/two/session.jsonl");

    assert_eq!(lookup.resolve_session_id(first, "session"), "session");
    let second_id = lookup.resolve_session_id(second, "session");
    assert!(second_id.starts_with("session-"));
    assert_eq!(second_id.len(), "session-".len() + 16);
    assert_eq!(lookup.resolve_session_id(second, "session"), second_id);
}

#[test]
fn the_suffix_hash_is_fixed_fnv1a_not_the_toolchain_hasher() {
    // FNV-1a 64 of the single byte "a" is a published test vector.
    let lookup = lookup(&[("s", "/elsewhere/s.jsonl")]);
    assert_eq!(
        lookup.resolve_session_id(Path::new("a"), "s"),
        "s-af63dc4c8601ec8c"
    );
}

#[test]
fn a_stored_hashed_id_is_kept_even_when_the_hash_would_differ() {
    // `STORED_HASH` stands in for a suffix an older build computed with a
    // different algorithm: the stored id wins over a recomputed one.
    let stored = format!("session-{STORED_HASH}");
    let lookup = lookup(&[
        ("session", "/one/session.jsonl"),
        (stored.as_str(), "/two/session.jsonl"),
    ]);
    assert_eq!(
        lookup.resolve_session_id(Path::new("/two/session.jsonl"), "session"),
        stored
    );
    // A third source is not handed another source's id.
    let third = lookup.resolve_session_id(Path::new("/three/session.jsonl"), "session");
    assert_ne!(third, stored);
    assert!(third.starts_with("session-"));
}

#[test]
fn a_stored_hashed_id_survives_its_rival_leaving_the_archive() {
    let stored = format!("session-{STORED_HASH}");
    let lookup = lookup(&[(stored.as_str(), "/two/session.jsonl")]);
    assert_eq!(
        lookup.resolve_session_id(Path::new("/two/session.jsonl"), "session"),
        stored
    );
}

#[test]
fn a_deleted_hashed_id_still_resolves_to_its_tombstone() {
    let stored = format!("session-{STORED_HASH}");
    let mut deleted = BTreeMap::new();
    deleted.insert(
        stored.clone(),
        Tombstone {
            deleted_at: "2026-09-01T00:00:00Z".to_string(),
            source_path: "/two/session.jsonl".to_string(),
        },
    );
    let lookup = lookup_for_tests(
        Vec::new(),
        captured(&[("session", "/one/session.jsonl")]),
        deleted,
    );
    let source = Path::new("/two/session.jsonl");
    let id = lookup.resolve_session_id(source, "session");
    assert_eq!(id, stored);
    assert!(lookup.is_deleted(&id, source));
}

#[test]
fn a_stemless_source_keeps_its_stored_unknown_id() {
    let source = Path::new("/tmp/..");
    let proposed = session_id(source);
    assert!(proposed.starts_with("unknown-"));
    let stored = format!("unknown-{STORED_HASH}");
    let lookup = lookup(&[(stored.as_str(), "/tmp/..")]);
    assert_eq!(lookup.resolve_session_id(source, &proposed), stored);
}

#[test]
fn an_unrelated_id_with_the_same_prefix_is_not_reused() {
    // `session-notes` is not a hashed variant of `session`.
    let lookup = lookup(&[
        ("session", "/one/session.jsonl"),
        ("session-notes", "/two/session.jsonl"),
    ]);
    let id = lookup.resolve_session_id(Path::new("/two/session.jsonl"), "session");
    assert_ne!(id, "session-notes");
    assert!(id.starts_with("session-"));
}
