// HalluScribe - speaker rendering tests for the reader model.
use super::{build_session, ChatProvider, MessageRole, ParsedMessage, ParsedSession};
use std::path::PathBuf;

fn session_with(speaker: Option<&str>) -> ParsedSession {
    let created_at = chrono::Utc::now();
    build_session(
        "test-session".to_string(),
        "Test".to_string(),
        created_at,
        None,
        PathBuf::from("/tmp/test.jsonl"),
        ChatProvider::AppleMessages,
        0.0,
        true,
        vec![ParsedMessage {
            role: MessageRole::User,
            text: "Hello there".to_string(),
            timestamp: None,
            speaker: speaker.map(ToString::to_string),
        }],
    )
    .expect("non-empty session")
}

#[test]
fn transcript_renders_speaker_label_in_place_of_role() {
    let session = session_with(Some("Me"));
    assert_eq!(session.transcript(), "[Me]\nHello there");

    let session = session_with(Some("Client"));
    assert_eq!(session.transcript(), "[Client]\nHello there");
}

#[test]
fn transcript_renders_role_label_when_speaker_is_none() {
    let session = session_with(None);
    assert_eq!(session.transcript(), "[User]\nHello there");
}
