// HalluScribe - per-session raw slice tests across the multi-session readers.
//
// The defect these guard: the sweep preserved `source_path` for every session,
// so each conversation in an N-conversation export got an identical raw
// containing the whole export - N copies of the same file, and
// `read_raw_session` returning every other conversation alongside the wanted
// one. Each session must carry only its own slice.

use super::{chatgpt, claudeai, gemini, is_multi_session_provider, raw_slices_for_source};
use std::fs;
use std::path::Path;

fn write_fixture(name: &str, content: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join(name), content).expect("write fixture");
    dir
}

/// Both slices exist, each holds only its own marker, and they differ.
fn assert_disjoint_slices(slices: &[Option<String>]) {
    let alpha = slices[0].as_deref().expect("first raw slice");
    let beta = slices[1].as_deref().expect("second raw slice");

    assert!(
        alpha.contains("ALPHA-MARKER"),
        "first slice lost its content"
    );
    assert!(
        !alpha.contains("BETA-MARKER"),
        "first slice carried the other session - whole file was preserved"
    );
    assert!(
        beta.contains("BETA-MARKER"),
        "second slice lost its content"
    );
    assert!(
        !beta.contains("ALPHA-MARKER"),
        "second slice carried the other session - whole file was preserved"
    );
    assert_ne!(alpha, beta, "each session needs a distinct raw");
}

const CHATGPT_EXPORT: &str = r#"[
  {
    "id": "conv-a", "title": "First", "create_time": 1710000000, "current_node": "n1",
    "mapping": { "n1": { "id": "n1", "parent": null, "children": [],
      "message": { "author": { "role": "user" },
        "content": { "content_type": "text", "parts": ["ALPHA-MARKER"] } } } }
  },
  {
    "id": "conv-b", "title": "Second", "create_time": 1710009999, "current_node": "n2",
    "mapping": { "n2": { "id": "n2", "parent": null, "children": [],
      "message": { "author": { "role": "user" },
        "content": { "content_type": "text", "parts": ["BETA-MARKER"] } } } }
  }
]"#;

#[test]
fn chatgpt_conversations_carry_only_their_own_raw_slice() {
    let dir = write_fixture("conversations.json", CHATGPT_EXPORT);
    let sessions = chatgpt::read(&dir.path().join("conversations.json")).expect("parse export");

    assert_eq!(sessions.len(), 2);
    let slices: Vec<Option<String>> = sessions.iter().map(|s| s.raw_slice.clone()).collect();
    assert_disjoint_slices(&slices);
}

#[test]
fn claude_ai_conversations_carry_only_their_own_raw_slice() {
    let dir = write_fixture(
        "conversations.json",
        r#"[
          {
            "uuid": "claude-a", "name": "First",
            "created_at": "2026-03-21T17:21:16.231185Z",
            "chat_messages": [
              { "sender": "human", "text": "ALPHA-MARKER", "created_at": "2026-03-21T17:21:16.598355Z" }
            ]
          },
          {
            "uuid": "claude-b", "name": "Second",
            "created_at": "2026-03-22T09:00:00.000000Z",
            "chat_messages": [
              { "sender": "human", "text": "BETA-MARKER", "created_at": "2026-03-22T09:00:01.000000Z" }
            ]
          }
        ]"#,
    );
    let sessions = claudeai::read(&dir.path().join("conversations.json")).expect("parse export");

    assert_eq!(sessions.len(), 2);
    let slices: Vec<Option<String>> = sessions.iter().map(|s| s.raw_slice.clone()).collect();
    assert_disjoint_slices(&slices);
}

#[test]
fn gemini_activities_carry_only_their_own_raw_slice() {
    let dir = write_fixture(
        "My Activity.json",
        r#"[
          {
            "header": "Gemini Apps", "title": "Prompted ALPHA-MARKER",
            "time": "2026-04-20T18:24:31.187Z",
            "safeHtmlItem": [{ "html": "<p>first answer</p>" }]
          },
          {
            "header": "Gemini Apps", "title": "Prompted BETA-MARKER",
            "time": "2026-04-21T08:00:00.000Z",
            "safeHtmlItem": [{ "html": "<p>second answer</p>" }]
          }
        ]"#,
    );
    let sessions = gemini::read(&dir.path().join("My Activity.json")).expect("parse export");

    assert_eq!(sessions.len(), 2);
    let slices: Vec<Option<String>> = sessions.iter().map(|s| s.raw_slice.clone()).collect();
    assert_disjoint_slices(&slices);
}

#[test]
fn raw_slices_for_source_maps_every_conversation_id() {
    let dir = write_fixture("conversations.json", CHATGPT_EXPORT);
    let slices = raw_slices_for_source(&dir.path().join("conversations.json"), "chatgpt")
        .expect("chatgpt is a multi-session provider");

    assert_eq!(slices.len(), 2);
    assert!(slices["conv-a"].contains("ALPHA-MARKER"));
    assert!(!slices["conv-a"].contains("BETA-MARKER"));
    assert!(slices["conv-b"].contains("BETA-MARKER"));
}

/// Coding tools write one file per session, so the whole-file copy the sweep
/// falls back to is already correct - these must NOT report slices.
#[test]
fn one_file_per_session_providers_report_no_slices() {
    for provider in ["claude_code", "codex", "continue", "forge", ""] {
        assert!(
            raw_slices_for_source(Path::new("session.jsonl"), provider).is_none(),
            "{provider} maps 1:1 to a file and must keep the whole-file raw"
        );
    }
}

/// An unparseable export must yield an empty map, never `None`: `None` would
/// send the backfill back to copying the whole file as one session's raw.
#[test]
fn unparseable_multi_session_source_yields_no_slices_not_a_file_fallback() {
    let dir = write_fixture("conversations.json", "{ not valid json");
    let slices = raw_slices_for_source(&dir.path().join("conversations.json"), "chatgpt")
        .expect("still a multi-session provider");

    assert!(slices.is_empty());
}

#[test]
fn apple_messages_is_a_multi_session_provider() {
    // An Apple backup holds every conversation in one sms.db, so it is
    // multi-session like the other chat exports.
    assert!(is_multi_session_provider("apple_messages"));
}

#[test]
fn coding_providers_are_not_multi_session_providers() {
    assert!(!is_multi_session_provider("claude_code"));
    assert!(!is_multi_session_provider("codex"));
    assert!(!is_multi_session_provider("forge"));
}
