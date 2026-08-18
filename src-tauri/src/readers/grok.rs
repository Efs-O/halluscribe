// HalluScribe - Grok export reader for prod-grok-backend.json imports.
//
// Shape: {"conversations":[{"conversation":{...},"responses":[{"response":{...}}]}], ...}.
// Three things in this format bite, and each has a test below:
//   * `sender` is inconsistent in BOTH case and vocabulary - "human",
//     "assistant", "ASSISTANT", and even a bare model name ("grok-3"). Only
//     "human" is a user turn; everything else is the assistant.
//   * response timestamps are MongoDB extended JSON
//     ({"$date":{"$numberLong":"ms"}}), while the conversation's own
//     `create_time` is plain RFC3339.
//   * `leaf_response_id` looks like ChatGPT's `current_node` but is set on
//     almost no conversation, so messages are ordered by timestamp instead of
//     by walking the reply tree.
// See docs/internal/GROK_IMPORT_PLAN.md § 2.
use super::{
    build_session, file_stem_or_hash, parse_rfc3339, read_json, ChatProvider, MessageRole,
    ParsedMessage, ParsedSession, ReaderError,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::Path;

pub fn read(path: &Path) -> Result<Vec<ParsedSession>, ReaderError> {
    let content = read_json(path)?;
    let export: Value = serde_json::from_str(&content)?;
    let Some(conversations) = export.get("conversations").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let mut sessions = Vec::new();
    for entry in conversations {
        if let Some(session) = read_conversation(entry, path) {
            sessions.push(session);
        }
    }
    Ok(sessions)
}

/// One `{"conversation":{...},"responses":[...]}` entry, or `None` when it
/// carries no usable id or no non-empty message.
fn read_conversation(entry: &Value, path: &Path) -> Option<ParsedSession> {
    let conversation = entry.get("conversation")?;
    let id = conversation
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?;

    let messages = collect_messages(entry);
    if messages.is_empty() {
        return None;
    }

    // The conversation's own create_time is RFC3339; fall back to the first
    // message's timestamp when it is missing or unparseable.
    let created_at = conversation
        .get("create_time")
        .and_then(Value::as_str)
        .and_then(parse_rfc3339)
        .or_else(|| messages.first().and_then(|message| message.timestamp))?;
    let updated_at = conversation
        .get("modify_time")
        .and_then(Value::as_str)
        .and_then(parse_rfc3339);

    let title = conversation
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| first_user_prefix(&messages))
        .unwrap_or_else(|| format!("Grok {}", file_stem_or_hash(path)));

    let total_chars: usize = messages.iter().map(|message| message.text.len()).sum();
    let fill_pct = ((total_chars as f64 / 1_200_000.0) * 100.0).clamp(0.0, 100.0);

    // This conversation's own JSON object, not the whole export - the raw
    // preserved for a session must be that session alone.
    let raw_slice = serde_json::to_string_pretty(entry).unwrap_or_else(|_| entry.to_string());

    build_session(
        id.to_string(),
        title,
        created_at,
        updated_at,
        path.to_path_buf(),
        ChatProvider::Grok,
        fill_pct,
        true,
        messages,
    )
    .map(|session| session.with_raw_slice(raw_slice))
}

/// Every non-empty turn of one conversation, ordered oldest first.
///
/// Ordering is by timestamp rather than by walking `parent_response_id` from
/// `leaf_response_id`: that pointer is null on almost every conversation, so
/// the walk would be dead code, and branching is rare enough that a
/// chronological flatten reads correctly. Turns without a parseable timestamp
/// keep their export order, which is already chronological.
fn collect_messages(entry: &Value) -> Vec<ParsedMessage> {
    let Some(responses) = entry.get("responses").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut messages: Vec<(usize, ParsedMessage)> = Vec::new();
    for (index, wrapper) in responses.iter().enumerate() {
        let Some(response) = wrapper.get("response") else {
            continue;
        };
        // Image-generation turns carry an empty body; they would contribute
        // nothing to the transcript.
        let text = response
            .get("message")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty());
        let Some(text) = text else {
            continue;
        };
        messages.push((
            index,
            ParsedMessage {
                role: message_role(response),
                text: text.to_string(),
                timestamp: response.get("create_time").and_then(mongo_datetime),
            },
        ));
    }

    // Stable by (timestamp, original index) so undated turns never jump the
    // queue and equal timestamps keep export order.
    messages.sort_by(|(left_index, left), (right_index, right)| {
        left.timestamp
            .cmp(&right.timestamp)
            .then(left_index.cmp(right_index))
    });
    messages.into_iter().map(|(_, message)| message).collect()
}

/// Only `sender == "human"` (in any case) is a user turn.
///
/// Everything else is the assistant, INCLUDING values that are not roles at
/// all: the export carries "ASSISTANT" and bare model names such as "grok-3"
/// in this field. Matching "assistant" explicitly would silently drop those.
fn message_role(response: &Value) -> MessageRole {
    let sender = response
        .get("sender")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if sender == "human" {
        MessageRole::User
    } else {
        MessageRole::Assistant
    }
}

/// MongoDB extended JSON: `{"$date":{"$numberLong":"1786052035229"}}`, where
/// the value is milliseconds since the epoch carried as a STRING. Also accepts
/// the plain-number and RFC3339 spellings of `$date`, which the same format
/// permits, so a future export change does not silently drop every timestamp.
fn mongo_datetime(value: &Value) -> Option<DateTime<Utc>> {
    let date = value.get("$date")?;
    if let Some(millis) = date.get("$numberLong").and_then(Value::as_str) {
        return millis
            .parse::<i64>()
            .ok()
            .and_then(DateTime::from_timestamp_millis);
    }
    if let Some(millis) = date.as_i64() {
        return DateTime::from_timestamp_millis(millis);
    }
    date.as_str().and_then(parse_rfc3339)
}

fn first_user_prefix(messages: &[ParsedMessage]) -> Option<String> {
    messages.iter().find_map(|message| {
        if matches!(message.role, MessageRole::User) {
            let prefix = message.text.trim().chars().take(80).collect::<String>();
            (!prefix.is_empty()).then_some(prefix)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::read;
    use crate::readers::{ChatProvider, MessageRole};
    use std::fs;
    use std::path::PathBuf;

    fn write_fixture(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("prod-grok-backend.json");
        fs::write(&path, content).expect("write fixture");
        (dir, path)
    }

    /// `1786052035229` ms and `1786052040000` ms, in export order.
    const TWO_TURNS: &str = r#"{
      "conversations": [
        {
          "conversation": {
            "id": "b66c501e-77a3-431d-a286-bfe6ab0baefb",
            "title": "Old Windows XP Malware Infection",
            "create_time": "2026-08-06T21:33:23.599938Z",
            "modify_time": "2026-08-06T21:52:12.494Z",
            "leaf_response_id": null
          },
          "responses": [
            { "response": {
                "_id": "a", "message": "How does an old Windows box get owned so fast?",
                "sender": "human",
                "create_time": { "$date": { "$numberLong": "1786052035229" } } } },
            { "response": {
                "_id": "b", "message": "Because unpatched services are scanned constantly.",
                "sender": "ASSISTANT",
                "create_time": { "$date": { "$numberLong": "1786052040000" } } } }
          ]
        }
      ],
      "projects": [], "tasks": [], "media_posts": []
    }"#;

    #[test]
    fn reads_a_conversation_into_ordered_turns() {
        let (_dir, path) = write_fixture(TWO_TURNS);
        let sessions = read(&path).expect("parse grok export");
        assert_eq!(sessions.len(), 1);

        let session = &sessions[0];
        assert_eq!(session.id, "b66c501e-77a3-431d-a286-bfe6ab0baefb");
        assert_eq!(session.title, "Old Windows XP Malware Infection");
        assert_eq!(session.provider, ChatProvider::Grok);
        assert!(session.fill_estimated);
        assert_eq!(session.messages.len(), 2);
        assert!(matches!(session.messages[0].role, MessageRole::User));
        assert!(matches!(session.messages[1].role, MessageRole::Assistant));
        assert_eq!(
            session.created_at.to_rfc3339(),
            "2026-08-06T21:33:23.599938+00:00"
        );
        assert!(session.updated_at.is_some());
    }

    /// The trap from GROK_IMPORT_PLAN § 2.1: only "human" is a user turn, and
    /// the export really does carry "ASSISTANT" and bare model names here.
    #[test]
    fn every_sender_spelling_maps_to_a_role() {
        let (_dir, path) = write_fixture(
            r#"{"conversations":[{"conversation":{"id":"c1","title":"t",
                 "create_time":"2026-08-06T21:33:23.599938Z"},
                 "responses":[
                   {"response":{"message":"a","sender":"human",
                     "create_time":{"$date":{"$numberLong":"1"}}}},
                   {"response":{"message":"b","sender":"assistant",
                     "create_time":{"$date":{"$numberLong":"2"}}}},
                   {"response":{"message":"c","sender":"ASSISTANT",
                     "create_time":{"$date":{"$numberLong":"3"}}}},
                   {"response":{"message":"d","sender":"grok-3",
                     "create_time":{"$date":{"$numberLong":"4"}}}},
                   {"response":{"message":"e",
                     "create_time":{"$date":{"$numberLong":"5"}}}}
                 ]}]}"#,
        );
        let sessions = read(&path).expect("parse grok export");
        let roles: Vec<&MessageRole> = sessions[0].messages.iter().map(|m| &m.role).collect();
        assert_eq!(roles.len(), 5);
        assert!(matches!(roles[0], MessageRole::User));
        // "assistant", "ASSISTANT", "grok-3" and a missing sender are all
        // assistant turns - none of them may be dropped.
        for role in &roles[1..] {
            assert!(matches!(role, MessageRole::Assistant));
        }
    }

    #[test]
    fn mongo_millis_are_parsed_and_order_the_turns() {
        let (_dir, path) = write_fixture(
            r#"{"conversations":[{"conversation":{"id":"c1","title":"t",
                 "create_time":"2026-08-06T21:33:23.599938Z"},
                 "responses":[
                   {"response":{"message":"second","sender":"assistant",
                     "create_time":{"$date":{"$numberLong":"1786052040000"}}}},
                   {"response":{"message":"first","sender":"human",
                     "create_time":{"$date":{"$numberLong":"1786052035229"}}}}
                 ]}]}"#,
        );
        let sessions = read(&path).expect("parse grok export");
        let messages = &sessions[0].messages;
        assert_eq!(messages[0].text, "first");
        assert_eq!(messages[1].text, "second");
        assert_eq!(
            messages[0].timestamp.expect("timestamp").to_rfc3339(),
            "2026-08-06T21:33:55.229+00:00"
        );
    }

    #[test]
    fn empty_message_bodies_are_skipped_and_empty_conversations_dropped() {
        let (_dir, path) = write_fixture(
            r#"{"conversations":[
                 {"conversation":{"id":"c1","title":"kept",
                   "create_time":"2026-08-06T21:33:23.599938Z"},
                  "responses":[
                    {"response":{"message":"   ","sender":"human",
                      "create_time":{"$date":{"$numberLong":"1"}}}},
                    {"response":{"message":"real","sender":"assistant",
                      "create_time":{"$date":{"$numberLong":"2"}}}}]},
                 {"conversation":{"id":"c2","title":"dropped",
                   "create_time":"2026-08-06T21:33:23.599938Z"},
                  "responses":[
                    {"response":{"message":"","sender":"human",
                      "create_time":{"$date":{"$numberLong":"1"}}}}]}
               ]}"#,
        );
        let sessions = read(&path).expect("parse grok export");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "kept");
        assert_eq!(sessions[0].messages.len(), 1);
    }

    /// The preserved raw must be THIS conversation, never the whole export.
    #[test]
    fn raw_slice_holds_only_its_own_conversation() {
        let (_dir, path) = write_fixture(
            r#"{"conversations":[
                 {"conversation":{"id":"c1","title":"mine",
                   "create_time":"2026-08-06T21:33:23.599938Z"},
                  "responses":[{"response":{"message":"mine","sender":"human",
                    "create_time":{"$date":{"$numberLong":"1"}}}}]},
                 {"conversation":{"id":"c2","title":"sibling",
                   "create_time":"2026-08-06T21:33:23.599938Z"},
                  "responses":[{"response":{"message":"sibling","sender":"human",
                    "create_time":{"$date":{"$numberLong":"1"}}}}]}
               ]}"#,
        );
        let sessions = read(&path).expect("parse grok export");
        assert_eq!(sessions.len(), 2);
        let raw = sessions[0].raw_slice.as_deref().expect("raw slice");
        assert!(raw.contains("mine"));
        assert!(!raw.contains("sibling"));
    }

    #[test]
    fn falls_back_to_the_first_user_message_when_untitled() {
        let (_dir, path) = write_fixture(
            r#"{"conversations":[{"conversation":{"id":"c1","title":"  ",
                 "create_time":"2026-08-06T21:33:23.599938Z"},
                 "responses":[{"response":{"message":"Best islands in Greece?",
                   "sender":"human","create_time":{"$date":{"$numberLong":"1"}}}}]}]}"#,
        );
        let sessions = read(&path).expect("parse grok export");
        assert_eq!(sessions[0].title, "Best islands in Greece?");
    }

    #[test]
    fn an_export_without_conversations_is_not_an_error() {
        let (_dir, path) = write_fixture(r#"{"projects":[],"tasks":[]}"#);
        assert!(read(&path).expect("parse grok export").is_empty());
    }
}
