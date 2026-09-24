// HalluScribe - persisted agent chat recording artifacts for save-on-clear chat sessions.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

const RECORDED_PROJECT: &str = "HalluScribe";
const RECORDED_TOOL: &str = "HalluScribe Chat";
const RECORDED_PROVIDER: &str = "halluscribe_agent_chat";
const RECORDED_ORIGIN: &str = "recorded_chat";
const RECORDED_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedChatTurn {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub tool_activity: Option<String>,
    #[serde(default)]
    pub attachment_name: Option<String>,
    #[serde(default)]
    pub had_thinking_output: bool,
    #[serde(default)]
    pub state: Option<RecordedTurnState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedTurnState {
    pub search_mode: String,
    pub web_search_enabled: bool,
    pub thinking_enabled: bool,
    pub backend_name: String,
    pub model_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveRecordedChatRequest {
    #[serde(default)]
    pub existing_relative_path: Option<String>,
    pub chat_search_mode: String,
    #[serde(default)]
    pub source_session_ids: Vec<String>,
    pub web_search_enabled: bool,
    pub thinking_enabled: bool,
    pub had_thinking_output: bool,
    pub backend_name: String,
    pub model_name: String,
    pub boundary_reason: String,
    pub turns: Vec<RecordedChatTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveRecordedChatResult {
    pub session_id: String,
    pub relative_path: String,
    pub absolute_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecordedChatSession {
    version: u32,
    session_id: String,
    project: String,
    tool: String,
    provider: String,
    session_origin: String,
    created_at: String,
    updated_at: String,
    chat_search_mode: String,
    source_session_ids: Vec<String>,
    web_search_enabled: bool,
    thinking_enabled: bool,
    had_thinking_output: bool,
    backend_name: String,
    model_name: String,
    boundary_reason: String,
    turns: Vec<RecordedChatTurn>,
}

pub fn save_recorded_chat_session(
    archive_dir: &Path,
    request: SaveRecordedChatRequest,
) -> Result<SaveRecordedChatResult, String> {
    let trimmed_turns: Vec<RecordedChatTurn> = request
        .turns
        .clone()
        .into_iter()
        .filter(|turn| {
            !turn.content.trim().is_empty()
                || turn
                    .tool_activity
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                || turn
                    .attachment_name
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
        })
        .collect();
    if trimmed_turns.is_empty() {
        return Err("recorded chat is empty".to_string());
    }

    let now = Utc::now();
    let existing_relative_path = request
        .existing_relative_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from);
    let (rel, abs) = existing_relative_path
        .map(|relative| resolve_existing_recorded_path(archive_dir, &relative))
        .transpose()?
        .unwrap_or_else(|| build_new_recorded_path(archive_dir, &request, &trimmed_turns, now));
    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let previous = fs::read_to_string(&abs)
        .ok()
        .and_then(|text| serde_json::from_str::<RecordedChatSession>(&text).ok());
    let created_at = previous
        .as_ref()
        .map(|existing| existing.created_at.clone())
        .unwrap_or_else(|| now.to_rfc3339());
    let session_id = previous
        .as_ref()
        .map(|existing| existing.session_id.clone())
        .unwrap_or_else(|| build_session_id(&rel, now));

    let payload = RecordedChatSession {
        version: RECORDED_VERSION,
        session_id,
        project: RECORDED_PROJECT.to_string(),
        tool: RECORDED_TOOL.to_string(),
        provider: RECORDED_PROVIDER.to_string(),
        session_origin: RECORDED_ORIGIN.to_string(),
        created_at,
        updated_at: now.to_rfc3339(),
        chat_search_mode: request.chat_search_mode,
        source_session_ids: request.source_session_ids,
        web_search_enabled: request.web_search_enabled,
        thinking_enabled: request.thinking_enabled,
        had_thinking_output: request.had_thinking_output,
        backend_name: request.backend_name,
        model_name: request.model_name,
        boundary_reason: request.boundary_reason,
        turns: trimmed_turns,
    };

    let json = serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?;
    fs::write(&abs, json).map_err(|error| error.to_string())?;
    Ok(SaveRecordedChatResult {
        session_id: payload.session_id,
        relative_path: rel.to_string_lossy().replace('\\', "/"),
        absolute_path: abs.to_string_lossy().replace('\\', "/"),
    })
}

fn request_fingerprint(
    chat_search_mode: &str,
    boundary_reason: &str,
    backend_name: &str,
    model_name: &str,
    turns: &[RecordedChatTurn],
) -> String {
    let mut hasher = DefaultHasher::new();
    chat_search_mode.hash(&mut hasher);
    boundary_reason.hash(&mut hasher);
    backend_name.hash(&mut hasher);
    model_name.hash(&mut hasher);
    for turn in turns {
        turn.role.hash(&mut hasher);
        turn.content.hash(&mut hasher);
        turn.tool_activity.hash(&mut hasher);
        turn.attachment_name.hash(&mut hasher);
        turn.had_thinking_output.hash(&mut hasher);
        if let Some(state) = &turn.state {
            state.search_mode.hash(&mut hasher);
            state.web_search_enabled.hash(&mut hasher);
            state.thinking_enabled.hash(&mut hasher);
            state.backend_name.hash(&mut hasher);
            state.model_name.hash(&mut hasher);
        }
    }
    format!("{:08x}", hasher.finish() as u32)
}

fn resolve_existing_recorded_path(
    archive_dir: &Path,
    relative_path: &Path,
) -> Result<(PathBuf, PathBuf), String> {
    if relative_path.is_absolute() {
        return Err("recorded session path must be relative".to_string());
    }
    let rel = normalize_relative_path(relative_path);
    let Some(first) = rel.components().next() else {
        return Err("recorded session path must stay under recorded_sessions".to_string());
    };
    if first.as_os_str() != "recorded_sessions" {
        return Err("recorded session path must stay under recorded_sessions".to_string());
    }
    Ok((rel.clone(), archive_dir.join(rel)))
}

fn normalize_relative_path(path: &Path) -> PathBuf {
    path.components()
        .fold(PathBuf::new(), |mut acc, component| {
            if let std::path::Component::Normal(part) = component {
                acc.push(part);
            }
            acc
        })
}

fn build_new_recorded_path(
    archive_dir: &Path,
    request: &SaveRecordedChatRequest,
    turns: &[RecordedChatTurn],
    now: chrono::DateTime<Utc>,
) -> (PathBuf, PathBuf) {
    let date = now.format("%Y-%m-%d").to_string();
    let time = now.format("%H-%M-%S").to_string();
    let millis = now.timestamp_subsec_millis();
    let fingerprint = request_fingerprint(
        &request.chat_search_mode,
        &request.boundary_reason,
        &request.backend_name,
        &request.model_name,
        turns,
    );
    let rel = PathBuf::from("recorded_sessions")
        .join(RECORDED_PROJECT)
        .join("agent-chat")
        .join(&date)
        .join(format!("{time}-{millis:03}-{fingerprint}-agent-chat.json"));
    let abs = archive_dir.join(&rel);
    (rel, abs)
}

fn build_session_id(relative_path: &Path, now: chrono::DateTime<Utc>) -> String {
    let stem = relative_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("agent-chat");
    format!("agent-chat-{}-{stem}", now.format("%Y-%m-%d"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> SaveRecordedChatRequest {
        SaveRecordedChatRequest {
            existing_relative_path: None,
            chat_search_mode: "archive".to_string(),
            source_session_ids: vec!["abc".to_string()],
            web_search_enabled: true,
            thinking_enabled: true,
            had_thinking_output: true,
            backend_name: "llama.cpp".to_string(),
            model_name: "gemma-4.gguf".to_string(),
            boundary_reason: "clear".to_string(),
            turns: vec![
                RecordedChatTurn {
                    role: "user".to_string(),
                    content: "why did the parser fail?".to_string(),
                    tool_activity: None,
                    attachment_name: None,
                    had_thinking_output: false,
                    state: Some(RecordedTurnState {
                        search_mode: "archive".to_string(),
                        web_search_enabled: true,
                        thinking_enabled: true,
                        backend_name: "llama.cpp".to_string(),
                        model_name: "gemma-4.gguf".to_string(),
                    }),
                },
                RecordedChatTurn {
                    role: "assistant".to_string(),
                    content: "The boundary condition was wrong.".to_string(),
                    tool_activity: Some("searching archives: parser boundary".to_string()),
                    attachment_name: None,
                    had_thinking_output: true,
                    state: Some(RecordedTurnState {
                        search_mode: "archive".to_string(),
                        web_search_enabled: true,
                        thinking_enabled: true,
                        backend_name: "llama.cpp".to_string(),
                        model_name: "gemma-4.gguf".to_string(),
                    }),
                },
            ],
        }
    }

    #[test]
    fn saves_recorded_chat_under_recorded_sessions_tree() {
        let dir = tempfile::tempdir().unwrap();
        let result = save_recorded_chat_session(dir.path(), sample_request()).unwrap();
        let text = fs::read_to_string(&result.absolute_path).unwrap();
        assert!(result.absolute_path.contains("recorded_sessions"));
        assert!(result.absolute_path.contains("HalluScribe"));
        assert!(result.absolute_path.contains("agent-chat"));
        assert!(text.contains("\"tool\": \"HalluScribe Chat\""));
        assert!(text.contains("\"provider\": \"halluscribe_agent_chat\""));
        assert!(text.contains("\"boundary_reason\": \"clear\""));
        assert!(text.contains("\"search_mode\": \"archive\""));
    }

    #[test]
    fn rejects_empty_recorded_chat() {
        let dir = tempfile::tempdir().unwrap();
        let mut request = sample_request();
        request.turns = vec![RecordedChatTurn {
            role: "assistant".to_string(),
            content: "   ".to_string(),
            tool_activity: None,
            attachment_name: None,
            had_thinking_output: false,
            state: None,
        }];
        let error = save_recorded_chat_session(dir.path(), request).unwrap_err();
        assert!(error.contains("empty"));
    }

    #[test]
    fn updates_existing_recorded_chat_file_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let first = save_recorded_chat_session(dir.path(), sample_request()).unwrap();
        let mut request = sample_request();
        request.existing_relative_path = Some(first.relative_path.clone());
        request.boundary_reason = "assistant_done".to_string();
        request.turns.push(RecordedChatTurn {
            role: "assistant".to_string(),
            content: "Follow-up answer".to_string(),
            tool_activity: None,
            attachment_name: None,
            had_thinking_output: false,
            state: Some(RecordedTurnState {
                search_mode: "semantic".to_string(),
                web_search_enabled: false,
                thinking_enabled: false,
                backend_name: "ollama".to_string(),
                model_name: "gemma4:26b".to_string(),
            }),
        });

        let second = save_recorded_chat_session(dir.path(), request).unwrap();
        assert_eq!(first.relative_path, second.relative_path);
        assert_eq!(first.session_id, second.session_id);
        let text = fs::read_to_string(&second.absolute_path).unwrap();
        assert!(text.contains("\"boundary_reason\": \"assistant_done\""));
        assert!(text.contains("Follow-up answer"));
    }
}
