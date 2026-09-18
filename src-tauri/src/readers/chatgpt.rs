// HalluScribe - ChatGPT export reader for conversations.json imports.
use super::{
    build_session, chatgpt_content, file_stem_or_hash, read_json, ChatProvider, MessageRole,
    ParsedMessage, ParsedSession, ReaderError,
};
use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::Path;

pub fn read(path: &Path) -> Result<Vec<ParsedSession>, ReaderError> {
    let content = read_json(path)?;
    let conversations: Vec<Value> = serde_json::from_str(&content)?;
    let mut sessions = Vec::new();

    for conversation in conversations {
        let Some(id) = conversation
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        let Some(created_at) = conversation
            .get("create_time")
            .and_then(value_to_datetime)
            .or_else(|| conversation.get("update_time").and_then(value_to_datetime))
        else {
            continue;
        };

        let mapping = conversation
            .get("mapping")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let Some(node_ids) = resolve_active_thread(&conversation, &mapping) else {
            continue;
        };

        let mut messages = Vec::new();
        for node_id in node_ids {
            let Some(node) = mapping.get(&node_id) else {
                continue;
            };
            let Some(message) = node.get("message") else {
                continue;
            };
            let Some(role) = message_role(message) else {
                continue;
            };
            if chatgpt_content::should_skip_message(message, &role) {
                continue;
            }
            let Some(text) = chatgpt_content::extract_message_text(message) else {
                continue;
            };
            messages.push(ParsedMessage {
                role,
                text,
                timestamp: message.get("create_time").and_then(value_to_datetime),
                speaker: None,
            });
        }

        let title = conversation
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                messages.iter().find_map(|message| {
                    if matches!(message.role, MessageRole::User) {
                        let prefix = message.text.trim().chars().take(80).collect::<String>();
                        (!prefix.is_empty()).then_some(prefix)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| format!("ChatGPT {}", file_stem_or_hash(path)));

        let total_chars: usize = messages.iter().map(|message| message.text.len()).sum();
        let fill_pct = ((total_chars as f64 / 1_200_000.0) * 100.0).clamp(0.0, 100.0);

        // This conversation's own JSON object, not the whole export — the raw
        // preserved for a session must be that session alone.
        let raw_slice = serde_json::to_string_pretty(&conversation)
            .unwrap_or_else(|_| conversation.to_string());

        if let Some(session) = build_session(
            id.to_string(),
            title,
            created_at,
            conversation.get("update_time").and_then(value_to_datetime),
            path.to_path_buf(),
            ChatProvider::ChatGPT,
            fill_pct,
            true,
            messages,
        ) {
            sessions.push(session.with_raw_slice(raw_slice));
        }
    }

    Ok(sessions)
}

fn resolve_active_thread(
    conversation: &Value,
    mapping: &Map<String, Value>,
) -> Option<Vec<String>> {
    let start_id = conversation
        .get("current_node")
        .and_then(Value::as_str)
        .filter(|node_id| mapping.contains_key(*node_id))
        .map(ToOwned::to_owned)
        .or_else(|| {
            mapping.iter().find_map(|(node_id, node)| {
                node.get("parent")
                    .filter(|parent| parent.is_null())
                    .map(|_| node_id.clone())
            })
        })
        .or_else(|| mapping.keys().next().cloned())?;

    let mut thread = Vec::new();
    let mut seen = HashSet::new();
    let mut current_id = start_id;

    while seen.insert(current_id.clone()) {
        thread.push(current_id.clone());
        let Some(node) = mapping.get(&current_id) else {
            break;
        };
        let Some(parent_id) = node.get("parent").and_then(Value::as_str) else {
            break;
        };
        if !mapping.contains_key(parent_id) {
            break;
        }
        current_id = parent_id.to_string();
    }

    thread.reverse();
    Some(thread)
}

fn message_role(message: &Value) -> Option<MessageRole> {
    match message
        .get("author")
        .and_then(|author| author.get("role"))
        .and_then(Value::as_str)
    {
        Some("user") => Some(MessageRole::User),
        Some("assistant") => Some(MessageRole::Assistant),
        Some("system") => Some(MessageRole::System),
        _ => None,
    }
}

fn value_to_datetime(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(seconds) = value.as_i64() {
        return DateTime::from_timestamp(seconds, 0);
    }
    if let Some(seconds) = value.as_f64() {
        let whole = seconds.trunc() as i64;
        let nanos = ((seconds.fract().abs()) * 1_000_000_000.0) as u32;
        return DateTime::from_timestamp(whole, nanos);
    }
    if let Some(text) = value.as_str() {
        if let Ok(seconds) = text.parse::<i64>() {
            return DateTime::from_timestamp(seconds, 0);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::read;
    use crate::readers::{ChatProvider, MessageRole};
    use std::fs;

    fn write_fixture(name: &str, content: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join(name), content).expect("write fixture");
        dir
    }

    #[test]
    fn follows_current_node_branch_instead_of_first_child_path() {
        let dir = write_fixture(
            "conversations.json",
            r#"[
              {
                "id": "conv-1",
                "title": "",
                "create_time": 1710000000,
                "current_node": "a2",
                "mapping": {
                  "root": { "id": "root", "parent": null, "children": ["u1"] },
                  "u1": {
                    "id": "u1",
                    "parent": "root",
                    "children": ["a1", "a2"],
                    "message": {
                      "author": { "role": "user" },
                      "create_time": 1710000001,
                      "content": { "content_type": "text", "parts": ["Plan my trip"] }
                    }
                  },
                  "a1": {
                    "id": "a1",
                    "parent": "u1",
                    "children": [],
                    "message": {
                      "author": { "role": "assistant" },
                      "create_time": 1710000002,
                      "content": { "content_type": "text", "parts": ["Ignore this branch"] }
                    }
                  },
                  "a2": {
                    "id": "a2",
                    "parent": "u1",
                    "children": [],
                    "message": {
                      "author": { "role": "assistant" },
                      "create_time": 1710000003,
                      "content": { "content_type": "text", "parts": ["Try Athens and Naxos."] }
                    }
                  }
                }
              }
            ]"#,
        );

        let sessions = read(&dir.path().join("conversations.json")).expect("parse chatgpt export");
        assert_eq!(sessions.len(), 1);

        let session = &sessions[0];
        assert_eq!(session.id, "conv-1");
        assert_eq!(session.title, "Plan my trip");
        assert_eq!(session.provider, ChatProvider::ChatGPT);
        assert!(session.fill_estimated);
        assert_eq!(session.messages.len(), 2);
        assert!(matches!(session.messages[0].role, MessageRole::User));
        assert_eq!(session.messages[0].text, "Plan my trip");
        assert!(matches!(session.messages[1].role, MessageRole::Assistant));
        assert_eq!(session.messages[1].text, "Try Athens and Naxos.");
        assert_eq!(session.updated_at, None);
        // Golden transcript: with no speaker labels, the role labels are used
        // exactly as before the business-messaging work.
        assert_eq!(
            session.transcript(),
            "[User]\nPlan my trip\n\n[Assistant]\nTry Athens and Naxos."
        );
    }

    #[test]
    fn reads_code_and_multimodal_text_and_skips_hidden_context_nodes() {
        let dir = write_fixture(
            "conversations.json",
            r#"[
              {
                "id": "conv-2",
                "title": "Structured payload",
                "create_time": 1710000100,
                "update_time": 1710000200,
                "current_node": "a1",
                "mapping": {
                  "root": { "id": "root", "parent": null, "children": ["hidden"] },
                  "hidden": {
                    "id": "hidden",
                    "parent": "root",
                    "children": ["u1"],
                    "message": {
                      "author": { "role": "system" },
                      "content": {
                        "content_type": "user_editable_context",
                        "user_profile": "hidden"
                      },
                      "metadata": {
                        "is_user_system_message": true,
                        "is_visually_hidden_from_conversation": true
                      }
                    }
                  },
                  "u1": {
                    "id": "u1",
                    "parent": "hidden",
                    "children": ["a1"],
                    "message": {
                      "author": { "role": "user" },
                      "content": {
                        "content_type": "multimodal_text",
                        "parts": [
                          { "content_type": "image_asset_pointer", "asset_pointer": "sediment://file" },
                          "Summarize this PDF"
                        ]
                      },
                      "metadata": {
                        "attachments": [{ "name": "report.pdf" }]
                      }
                    }
                  },
                  "a1": {
                    "id": "a1",
                    "parent": "u1",
                    "children": [],
                    "message": {
                      "author": { "role": "assistant" },
                      "content": {
                        "content_type": "code",
                        "text": "search(\"example query\")"
                      },
                      "metadata": {
                        "citations": [{ "title": "Should not leak into transcript" }]
                      }
                    }
                  }
                }
              }
            ]"#,
        );

        let sessions = read(&dir.path().join("conversations.json")).expect("parse chatgpt export");
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].updated_at.as_ref().map(|date| date.timestamp()),
            Some(1710000200)
        );
        assert_eq!(
            sessions[0].messages[0].text,
            "Summarize this PDF\n[Attachments: report.pdf]"
        );
        assert_eq!(sessions[0].messages[1].text, "search(\"example query\")");
        assert_eq!(sessions[0].messages.len(), 2);
    }
}
