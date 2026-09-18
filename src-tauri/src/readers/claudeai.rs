// HalluScribe - Claude.ai export reader for conversations.json imports.
use super::{
    build_session, parse_rfc3339, read_json, ChatProvider, MessageRole, ParsedMessage,
    ParsedSession, ReaderError,
};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct ClaudeConversation {
    uuid: Option<String>,
    name: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    chat_messages: Option<Vec<ClaudeMessage>>,
}

#[derive(Deserialize)]
struct ClaudeMessage {
    sender: Option<String>,
    text: Option<String>,
    created_at: Option<String>,
    #[serde(default)]
    content: Vec<ClaudeContentBlock>,
    #[serde(default)]
    attachments: Vec<ClaudeAttachment>,
    #[serde(default)]
    files: Vec<ClaudeFile>,
}

#[derive(Deserialize)]
struct ClaudeContentBlock {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
    name: Option<String>,
    message: Option<String>,
    #[serde(default)]
    content: Vec<ClaudeContentItem>,
    #[serde(default)]
    display_content: Option<ClaudeDisplayContent>,
}

#[derive(Deserialize)]
struct ClaudeContentItem {
    text: Option<String>,
}

#[derive(Deserialize)]
struct ClaudeDisplayContent {
    text: Option<String>,
}

#[derive(Deserialize)]
struct ClaudeAttachment {
    file_name: Option<String>,
}

#[derive(Deserialize)]
struct ClaudeFile {
    file_name: Option<String>,
}

pub fn read(path: &Path) -> Result<Vec<ParsedSession>, ReaderError> {
    let content = read_json(path)?;
    // Parsed as untyped values first so each conversation's own JSON object can
    // be captured as its raw slice before the typed view consumes it; a
    // malformed entry still fails the whole read, exactly as before.
    let raw_conversations: Vec<serde_json::Value> = serde_json::from_str(&content)?;
    let mut sessions = Vec::new();

    for raw_conversation in raw_conversations {
        let raw_slice = serde_json::to_string_pretty(&raw_conversation)
            .unwrap_or_else(|_| raw_conversation.to_string());
        let conversation: ClaudeConversation = serde_json::from_value(raw_conversation)?;

        let Some(id) = conversation.uuid.filter(|id| !id.is_empty()) else {
            continue;
        };
        let Some(created_at_raw) = conversation.created_at.as_deref() else {
            continue;
        };
        let Some(created_at) = parse_rfc3339(created_at_raw) else {
            continue;
        };

        let updated_at = conversation.updated_at.as_deref().and_then(parse_rfc3339);
        let mut messages = Vec::new();
        for message in conversation.chat_messages.unwrap_or_default() {
            let role = match message.sender.as_deref() {
                Some("human") => MessageRole::User,
                Some("assistant") => MessageRole::Assistant,
                Some("system") => MessageRole::System,
                _ => continue,
            };
            let text = normalize_message_text(&message);
            if text.is_empty() {
                continue;
            }
            messages.push(ParsedMessage {
                role,
                text,
                timestamp: message.created_at.as_deref().and_then(parse_rfc3339),
                speaker: None,
            });
        }

        let total_chars: usize = messages.iter().map(|message| message.text.len()).sum();
        let fill_pct = ((total_chars as f64 / 600_000.0) * 100.0).clamp(0.0, 100.0);
        let title = conversation
            .name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Claude conversation".to_string());

        if let Some(session) = build_session(
            id,
            title,
            created_at,
            updated_at,
            path.to_path_buf(),
            ChatProvider::ClaudeAI,
            fill_pct,
            true,
            messages,
        ) {
            sessions.push(session.with_raw_slice(raw_slice));
        }
    }

    Ok(sessions)
}

fn normalize_message_text(message: &ClaudeMessage) -> String {
    let primary = message.text.as_deref().map(str::trim).unwrap_or_default();
    let derived = extract_content_text(&message.content);
    let mut parts = Vec::new();

    if !primary.is_empty() {
        parts.push(primary.to_string());
    }
    if !derived.is_empty() && normalized_for_compare(primary) != normalized_for_compare(&derived) {
        parts.push(derived);
    }
    let attachment_note = attachment_note(&message.attachments, &message.files);
    if !attachment_note.is_empty() {
        parts.push(attachment_note);
    }

    parts.join("\n\n").trim().to_string()
}

fn extract_content_text(blocks: &[ClaudeContentBlock]) -> String {
    let mut parts = Vec::new();
    for block in blocks {
        match block.kind.as_deref() {
            Some("text") => {
                if let Some(text) = block
                    .text
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    parts.push(text.to_string());
                }
            }
            Some("tool_use") => {
                let name = block.name.as_deref().unwrap_or("tool");
                let detail = block
                    .message
                    .as_deref()
                    .or_else(|| {
                        block
                            .display_content
                            .as_ref()
                            .and_then(|content| content.text.as_deref())
                    })
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .unwrap_or("");
                if detail.is_empty() {
                    parts.push(format!("[Tool use: {name}]"));
                } else {
                    parts.push(format!("[Tool use: {name}] {detail}"));
                }
            }
            Some("tool_result") => {
                let detail = block
                    .content
                    .iter()
                    .filter_map(|item| item.text.as_deref().map(str::trim))
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                if !detail.is_empty() {
                    parts.push(format!("[Tool result]\n{detail}"));
                }
            }
            _ => {}
        }
    }
    parts.join("\n\n").trim().to_string()
}

fn attachment_note(attachments: &[ClaudeAttachment], files: &[ClaudeFile]) -> String {
    let names = attachments
        .iter()
        .filter_map(|attachment| attachment.file_name.as_deref())
        .chain(files.iter().filter_map(|file| file.file_name.as_deref()))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if names.is_empty() {
        String::new()
    } else {
        format!("[Attachments: {}]", names.join(", "))
    }
}

fn normalized_for_compare(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::read;
    use crate::readers::{ChatProvider, MessageRole};
    use std::fs;

    fn write_fixture(content: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("conversations.json"), content).expect("write fixture");
        dir
    }

    #[test]
    fn reads_text_messages_and_skips_empty_or_unknown_senders() {
        let dir = write_fixture(
            r#"[
              {
                "uuid": "claude-1",
                "name": "Exported Claude Chat",
                "created_at": "2026-03-21T17:21:16.231185Z",
                "chat_messages": [
                  { "sender": "human", "text": "What does this error mean?", "created_at": "2026-03-21T17:21:16.598355Z" },
                  { "sender": "assistant", "text": "It means the media type is missing.", "created_at": "2026-03-21T17:21:37.556314Z" },
                  { "sender": "assistant", "text": "   ", "created_at": "2026-03-21T17:22:11.642513Z" },
                  { "sender": "tool", "text": "ignored" }
                ]
              }
            ]"#,
        );

        let sessions = read(&dir.path().join("conversations.json")).expect("parse claude export");
        assert_eq!(sessions.len(), 1);

        let session = &sessions[0];
        assert_eq!(session.id, "claude-1");
        assert_eq!(session.title, "Exported Claude Chat");
        assert_eq!(session.provider, ChatProvider::ClaudeAI);
        assert!(session.fill_estimated);
        assert_eq!(
            session.updated_at.as_ref().map(|date| date.to_rfc3339()),
            None
        );
        assert_eq!(session.messages.len(), 2);
        assert!(matches!(session.messages[0].role, MessageRole::User));
        assert!(matches!(session.messages[1].role, MessageRole::Assistant));
        // Golden transcript: empty and tool-sender turns are skipped, and the
        // role labels are used exactly as before the business-messaging work.
        assert_eq!(
            session.transcript(),
            "[User]\nWhat does this error mean?\n\n[Assistant]\nIt means the media type is missing."
        );
    }

    #[test]
    fn falls_back_to_content_blocks_and_preserves_updated_at() {
        let dir = write_fixture(
            r#"[
              {
                "uuid": "claude-2",
                "name": "Structured Claude Chat",
                "created_at": "2026-03-21T17:21:16.231185Z",
                "updated_at": "2026-03-21T17:22:53.719540Z",
                "chat_messages": [
                  {
                    "sender": "human",
                    "text": "",
                    "content": [{ "type": "text", "text": "Review these notes" }],
                    "files": [{ "file_name": "notes.txt" }]
                  },
                  {
                    "sender": "assistant",
                    "text": "",
                    "content": [
                      { "type": "tool_use", "name": "search", "message": "Search notes" },
                      { "type": "tool_result", "content": [{ "text": "Found two matches" }] }
                    ]
                  }
                ]
              }
            ]"#,
        );

        let sessions = read(&dir.path().join("conversations.json")).expect("parse claude export");
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0]
                .updated_at
                .as_ref()
                .map(|date| date.to_rfc3339()),
            Some("2026-03-21T17:22:53.719540+00:00".to_string())
        );
        assert_eq!(
            sessions[0].messages[0].text,
            "Review these notes\n\n[Attachments: notes.txt]"
        );
        assert!(sessions[0].messages[1]
            .text
            .contains("[Tool use: search] Search notes"));
        assert!(sessions[0].messages[1]
            .text
            .contains("[Tool result]\nFound two matches"));
    }
}
