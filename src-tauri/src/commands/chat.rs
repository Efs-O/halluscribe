// HalluScribe - recorded and interactive chat Tauri command handlers.

use super::BUSY_MESSAGE;
use crate::app_state::ChatCancel;
use crate::app_support::{archive_dir, ChatMessage};
use crate::chat_prompt::{build_chat_system_prompt, ChatPromptContext, SearchModePrompt};
use crate::recorded_sessions::{SaveRecordedChatRequest, SaveRecordedChatResult};
use crate::{archive, briefing, profile, retrieval, settings};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use tauri::{Emitter, Manager};

#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ChatSearchMode {
    Archive,
    Semantic,
}

/// Signal the active chat turn to stop after the current blocking step.
#[tauri::command]
pub(crate) fn cancel_chat(app: tauri::AppHandle) {
    app.state::<ChatCancel>().0.store(true, Ordering::Relaxed);
}

/// Persist one HalluScribe-owned recorded agent chat session for later sweep ingestion.
#[tauri::command]
pub(crate) fn save_recorded_chat_session(
    app: tauri::AppHandle,
    request: SaveRecordedChatRequest,
) -> Result<SaveRecordedChatResult, String> {
    let dir = archive_dir(&app)?;
    crate::recorded_sessions::save_recorded_chat_session(&dir, request)
}

/// Handle one chat turn with a tool-call loop.
#[tauri::command]
pub(crate) fn send_chat_message(
    app: tauri::AppHandle,
    messages: Vec<ChatMessage>,
    allowed_session_ids: Option<Vec<String>>,
    web_search_enabled: bool,
    thinking_enabled: bool,
    search_mode: Option<ChatSearchMode>,
    profile_scope: Option<String>,
) -> Result<(), String> {
    let dir = archive_dir(&app)?;
    archive::ensure_index_readable(&dir).map_err(|error| error.to_string())?;
    let profile_scope = match profile_scope.as_deref() {
        None => profile::ProfileScope::Work,
        Some(key) => profile::ProfileScope::from_key(key)
            .ok_or_else(|| format!("unknown profile scope: {key}"))?,
    };
    let settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    let has_images = messages.iter().any(|message| {
        message
            .images
            .as_ref()
            .is_some_and(|images| !images.is_empty())
    });
    let backend = settings
        .to_inference_backend()
        .ok_or_else(|| "backend not configured (check Settings)".to_string())?;
    let cancel = app.state::<ChatCancel>().0.clone();
    cancel.store(false, Ordering::Relaxed);
    let (ctx_size, max_tokens) = settings.generation_limits()?;
    let search_mode = search_mode.unwrap_or(ChatSearchMode::Archive);

    let json_messages: Vec<serde_json::Value> = messages
        .into_iter()
        .map(|message| serde_json::to_value(&message).unwrap_or_default())
        .collect();

    let chat_scope = match allowed_session_ids {
        Some(ids) if !ids.is_empty() => {
            briefing::tools::ChatScope::AllowedSessionIds(ids.into_iter().collect::<HashSet<_>>())
        }
        _ => briefing::tools::ChatScope::ArchiveWide,
    };
    let semantic_scope_ids = if matches!(search_mode, ChatSearchMode::Semantic) {
        if !retrieval::embedding_runtime_ready(&settings) {
            return Err(
                "Semantic search is not configured. Set the llama-server binary path and EmbeddingGemma GGUF model path in Settings first.".to_string(),
            );
        }
        let latest_user_query = json_messages
            .iter()
            .rev()
            .find_map(|message| {
                (message["role"].as_str() == Some("user"))
                    .then(|| message["content"].as_str().unwrap_or("").trim())
            })
            .unwrap_or("");
        let _ = app.emit(
            "chat-tool-call",
            briefing::ToolCallPayload {
                tool: "search_sessions_semantic".to_string(),
                args: serde_json::json!({ "query": latest_user_query }),
            },
        );
        let allowed_ids = match &chat_scope {
            briefing::tools::ChatScope::ArchiveWide => None,
            briefing::tools::ChatScope::AllowedSessionIds(ids) => Some(ids),
        };
        // Embedding the query loads the embedding model; serialise it against
        // any other inference job. Released before the chat turn re-acquires.
        let scope_ids = {
            let _inference_guard = crate::infer_lock::try_acquire().ok_or(BUSY_MESSAGE)?;
            retrieval::semantic_scope_ids(&dir, &settings, latest_user_query, 12, allowed_ids)?
        };
        Some(scope_ids)
    } else {
        None
    };
    let effective_scope = match semantic_scope_ids {
        Some(ids) => briefing::tools::ChatScope::AllowedSessionIds(ids.into_iter().collect()),
        None => chat_scope,
    };
    let ollama_api_key = settings.ollama_api_key.trim().to_string();
    let tavily_api_key = settings.tavily_api_key.trim().to_string();
    let web_search_available =
        web_search_enabled && (!ollama_api_key.is_empty() || !tavily_api_key.is_empty());
    // Chat uses the Work profile unless the UI toggle selects Personal
    // (Phase 2c sharing rule: Work is the default sharing scope).
    let user_profile = profile::read_profile_md(&dir, profile_scope);
    let mut final_messages = vec![serde_json::json!({
        "role": "system",
        "content": build_chat_system_prompt(&ChatPromptContext {
            web_search_available,
            has_images,
            search_mode: match search_mode {
                ChatSearchMode::Archive => SearchModePrompt::Archive,
                ChatSearchMode::Semantic => SearchModePrompt::Semantic,
            },
            scope_size: match &effective_scope {
                briefing::tools::ChatScope::AllowedSessionIds(ids) => Some(ids.len()),
                briefing::tools::ChatScope::ArchiveWide => None,
            },
            profile: user_profile,
            profile_scope,
        })
    })];
    final_messages.extend(json_messages);
    let runtime = briefing::ChatRuntimeOptions {
        chat_scope: effective_scope,
        web_search_enabled: web_search_available,
        ollama_api_key: if ollama_api_key.is_empty() {
            None
        } else {
            Some(ollama_api_key)
        },
        tavily_api_key: if tavily_api_key.is_empty() {
            None
        } else {
            Some(tavily_api_key)
        },
        reasoning_enabled: thinking_enabled,
    };

    let app_clone = app.clone();
    std::thread::spawn(move || {
        briefing::run_chat_turn(
            &app_clone,
            &backend,
            ctx_size,
            max_tokens,
            final_messages,
            &dir,
            runtime,
            cancel,
        );
    });
    Ok(())
}
