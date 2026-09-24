// HalluScribe - reader for chat sessions recorded by HalluScribe's own chat panel.

use super::{
    build_session, file_stem_or_hash, parse_rfc3339, read_json, ChatProvider, MessageRole,
    ParsedMessage, ParsedSession, ReaderError,
};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct RecordedChatTurn {
    role: String,
    content: String,
    #[serde(default)]
    tool_activity: Option<String>,
    #[serde(default)]
    attachment_name: Option<String>,
    #[serde(default)]
    _had_thinking_output: bool,
}

#[derive(Debug, Deserialize)]
struct RecordedChatSession {
    #[allow(dead_code)]
    version: u32,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    turns: Vec<RecordedChatTurn>,
}

pub fn read(path: &Path) -> Result<Vec<ParsedSession>, ReaderError> {
    let raw = read_json(path)?;
    let payload: RecordedChatSession = serde_json::from_str(&raw)?;
    let created_at = parse_rfc3339(&payload.created_at).unwrap_or_else(chrono::Utc::now);
    let updated_at = parse_rfc3339(&payload.updated_at);
    let messages = payload
        .turns
        .into_iter()
        .filter_map(|turn| {
            let role = match turn.role.as_str() {
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                _ => return None,
            };
            let mut parts = Vec::new();
            let content = turn.content.trim();
            if !content.is_empty() {
                parts.push(content.to_string());
            }
            if let Some(tool_activity) = turn.tool_activity.as_deref().map(str::trim) {
                if !tool_activity.is_empty() {
                    parts.push(format!("Tool activity: {tool_activity}"));
                }
            }
            if let Some(attachment_name) = turn.attachment_name.as_deref().map(str::trim) {
                if !attachment_name.is_empty() {
                    parts.push(format!("Attachment: {attachment_name}"));
                }
            }
            let text = parts.join("\n");
            (!text.trim().is_empty()).then_some(ParsedMessage {
                role,
                text,
                timestamp: Some(created_at),
                speaker: None,
            })
        })
        .collect::<Vec<_>>();

    let id = if payload.session_id.trim().is_empty() {
        file_stem_or_hash(path)
    } else {
        payload.session_id
    };
    let title = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Agent Chat")
        .replace('-', " ");

    Ok(build_session(
        id,
        title,
        created_at,
        updated_at,
        path.to_path_buf(),
        ChatProvider::HalluScribeAgentChat,
        0.0,
        false,
        messages,
    )
    .into_iter()
    .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reads_recorded_agent_chat() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gemma-chat.json");
        fs::write(
            &path,
            r#"{
  "version": 1,
  "session_id": "gemma4-abc",
  "created_at": "2026-04-23T10:00:00Z",
  "updated_at": "2026-04-23T10:05:00Z",
  "turns": [
    {
      "role": "user",
      "content": "what broke?",
      "had_thinking_output": false
    },
    {
      "role": "assistant",
      "content": "the parser boundary check was wrong",
      "tool_activity": "searching archives: parser boundary",
      "had_thinking_output": true
    }
  ]
}"#,
        )
        .unwrap();

        let sessions = read(&path).unwrap();
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.id, "gemma4-abc");
        assert_eq!(session.provider, ChatProvider::HalluScribeAgentChat);
        assert_eq!(session.source_path, path);
        assert!(session
            .transcript()
            .contains("Tool activity: searching archives"));
        // Golden: the user turn renders with the role label exactly as before
        // the business-messaging work (no speaker label is set).
        assert!(session.transcript().contains("[User]\nwhat broke?"));
    }
}
