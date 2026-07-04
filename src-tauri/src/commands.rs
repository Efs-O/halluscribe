// HalluScribe - Tauri command handlers for archive, settings, briefing, and chat.
use crate::app_state::{BriefingCancel, ChatCancel, SweepCancel};
use crate::app_support::{
    archive_dir, clear_first_run, collect_raw_session_total, collect_session_stats,
    record_sweep_date, ChatMessage, SessionStats,
};
use crate::chat_prompt::{build_chat_system_prompt, ChatPromptContext, SearchModePrompt};
use crate::recorded_sessions::{SaveRecordedChatRequest, SaveRecordedChatResult};
use crate::{archive, briefing, profile, retrieval, scheduler, search, settings};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use tauri::Emitter;
use tauri::Manager;

const BUSY_MESSAGE: &str =
    "Another job (a sweep, briefing, or chat) is using the model. Try again once it finishes.";
const BRIEFING_MIN_WORDS_PER_SESSION: usize = 200;
const BRIEFING_MAX_WORDS_PER_SESSION: usize = 300;
const BRIEFING_WARN_SESSION_COUNT: usize = 15;
const BRIEFING_CONTEXT_RISK_SESSION_COUNT: usize = 30;

#[derive(Serialize)]
pub(crate) struct ApiKeyValidationResult {
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EmbeddingRebuildProgressPayload {
    pub current: usize,
    pub total: usize,
    pub indexed: u32,
    pub skipped: u32,
    pub failed: u32,
}

#[derive(Debug, Clone, Copy, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ChatSearchMode {
    Archive,
    Semantic,
}

/// Return all indexed sessions, newest first.
#[tauri::command]
pub(crate) fn get_recent_sessions(
    app: tauri::AppHandle,
) -> Result<Vec<archive::IndexEntry>, String> {
    let dir = archive_dir(&app)?;
    let mut sessions = archive::read_sessions(&dir);
    sessions.sort_by_key(|session| {
        std::cmp::Reverse(if !session.updated_at.is_empty() {
            session.updated_at.clone()
        } else if !session.session_timestamp.is_empty() {
            session.session_timestamp.clone()
        } else {
            session.date.clone()
        })
    });
    Ok(sessions)
}

/// Return aggregate stats across all archived sessions.
#[tauri::command]
pub(crate) fn get_stats(app: tauri::AppHandle) -> Result<SessionStats, String> {
    collect_session_stats(&app)
}

#[tauri::command]
pub(crate) async fn get_raw_session_total(app: tauri::AppHandle) -> Result<u32, String> {
    tauri::async_runtime::spawn_blocking(move || collect_raw_session_total(&app))
        .await
        .map_err(|error| error.to_string())?
}

/// Run a sweep immediately ("Run Now" button). Bypasses the time-window check.
#[tauri::command]
pub(crate) fn trigger_sweep(app: tauri::AppHandle) -> Result<(), String> {
    let dir = archive_dir(&app)?;
    let settings = settings::load_settings(&dir);
    settings.generation_limits()?;
    let config = settings
        .to_sweep_config(dir, true)
        .ok_or_else(|| "backend is not configured (check Settings)".to_string())?;
    let cancel = app.state::<SweepCancel>().0.clone();
    cancel.store(false, Ordering::Relaxed);
    std::thread::spawn(move || {
        let result = scheduler::run_sweep(&app, &config, cancel);
        if result.busy {
            let _ = app.emit(
                "sweep-done",
                "A sweep or other model job is already running. Try again once it finishes."
                    .to_string(),
            );
            return;
        }
        clear_first_run(&app);
        record_sweep_date(&app);
        let mut message = format!(
            "Sweep complete - processed: {}, skipped: {}, deferred: {}",
            result.processed, result.skipped, result.deferred,
        );
        if !result.errors.is_empty() {
            message.push_str(&format!(", errors: {}", result.errors.len()));
        }
        if result.flagged > 0 {
            message.push_str(&format!(", possible secrets flagged: {}", result.flagged));
        }
        let _ = app.emit("sweep-done", message);
    });
    Ok(())
}

/// Load current settings from disk.
#[tauri::command]
pub(crate) fn get_settings(app: tauri::AppHandle) -> Result<settings::HalluScribeSettings, String> {
    let dir = archive_dir(&app)?;
    Ok(settings::load_settings(&dir))
}

/// Search the session archive using optional query/date/tag/tool filters.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub(crate) fn search_sessions(
    app: tauri::AppHandle,
    query: Option<String>,
    date_from: Option<String>,
    date_to: Option<String>,
    tags: Option<Vec<String>>,
    project: Option<String>,
    tool: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<archive::IndexEntry>, String> {
    let dir = archive_dir(&app)?;
    let params = search::SearchParams {
        query,
        date_from,
        date_to,
        tags,
        project,
        tool,
        limit,
        offset: None,
    };
    Ok(search::search_sessions(&dir, &params))
}

/// Read the full markdown content of a session by its session_id.
#[tauri::command]
pub(crate) fn read_session(app: tauri::AppHandle, session_id: String) -> Result<String, String> {
    let dir = archive_dir(&app)?;
    search::read_session(&dir, &session_id)
}

/// Full-text session search for the SESSION SUMMARY tab.
#[tauri::command]
pub(crate) fn search_sessions_fulltext(
    app: tauri::AppHandle,
    query: String,
) -> Result<Vec<archive::IndexEntry>, String> {
    let dir = archive_dir(&app)?;
    Ok(search::search_fulltext(&dir, &query))
}

/// Semantic search over archived session summaries.
#[tauri::command]
pub(crate) fn search_sessions_semantic(
    app: tauri::AppHandle,
    query: String,
    limit: Option<usize>,
    allowed_session_ids: Option<Vec<String>>,
) -> Result<Vec<retrieval::SemanticSearchResult>, String> {
    let dir = archive_dir(&app)?;
    let settings = settings::load_settings(&dir);
    let allowed_ids = allowed_session_ids
        .filter(|ids| !ids.is_empty())
        .map(|ids| ids.into_iter().collect::<HashSet<_>>());
    let _inference_guard = crate::infer_lock::try_acquire().ok_or(BUSY_MESSAGE)?;
    retrieval::semantic_search(
        &dir,
        &settings,
        &query,
        limit.unwrap_or(5).min(10),
        allowed_ids.as_ref(),
    )
}

/// Persist updated settings to disk.
#[tauri::command]
pub(crate) fn save_settings(
    app: tauri::AppHandle,
    mut new_settings: settings::HalluScribeSettings,
) -> Result<(), String> {
    let dir = archive_dir(&app)?;
    // `last_sweep_date` is managed by the scheduler, not the UI. Preserve the
    // on-disk value so saving settings never resets the catch-up state (audit
    // A-2) - otherwise a save would let the nightly sweep re-run the same day.
    new_settings.last_sweep_date = settings::load_settings(&dir).last_sweep_date;
    settings::save_settings(&dir, &new_settings).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn validate_ollama_api_key(api_key: String) -> Result<ApiKeyValidationResult, String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Ok(ApiKeyValidationResult {
            status: "empty".to_string(),
            message: "API key cleared.".to_string(),
        });
    }

    match briefing::tools::validate_ollama_api_key(trimmed) {
        Ok(()) => Ok(ApiKeyValidationResult {
            status: "valid".to_string(),
            message: "API key saved and validated.".to_string(),
        }),
        Err(error) => {
            let lower = error.to_ascii_lowercase();
            let (status, message) = if lower.contains("401")
                || lower.contains("403")
                || lower.contains("unauthorized")
                || lower.contains("forbidden")
                || lower.contains("invalid")
            {
                (
                    "invalid",
                    "API key saved, but validation failed. Check the key and try again.",
                )
            } else {
                (
                    "unreachable",
                    "API key saved, but validation could not complete right now.",
                )
            };
            Ok(ApiKeyValidationResult {
                status: status.to_string(),
                message: format!("{message} ({error})"),
            })
        }
    }
}

/// Trigger the auto briefing stream. Emits "briefing-token" and "briefing-done" events.
#[tauri::command]
pub(crate) fn run_briefing(
    app: tauri::AppHandle,
    date_from: Option<String>,
    date_to: Option<String>,
    fill_min: Option<f64>,
    fill_max: Option<f64>,
    keyword: Option<String>,
    session_ids: Option<Vec<String>>,
) -> Result<(), String> {
    let dir = archive_dir(&app)?;
    let settings = settings::load_settings(&dir);
    let backend = settings
        .to_inference_backend()
        .ok_or_else(|| "backend not configured (check Settings)".to_string())?;
    let (ctx_size, max_tokens) = settings.generation_limits()?;

    let filters = briefing::BriefingFilters {
        date_from: date_from
            .as_deref()
            .and_then(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()),
        date_to: date_to
            .as_deref()
            .and_then(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()),
        fill_min,
        fill_max,
        keyword,
    };

    let scope = match session_ids {
        Some(ids) if !ids.is_empty() => briefing::BriefingScope::SelectedSessionIds(ids),
        _ if filters.date_from.is_some()
            || filters.date_to.is_some()
            || filters.fill_min.is_some()
            || filters.fill_max.is_some()
            || filters
                .keyword
                .as_deref()
                .is_some_and(|text| !text.is_empty()) =>
        {
            briefing::BriefingScope::FilterBased(filters)
        }
        _ => briefing::BriefingScope::ArchiveWide,
    };

    let (content, header, session_count) = briefing::collect_briefing_sessions(&dir, &scope, 6)?;
    let min_word_limit = session_count * BRIEFING_MIN_WORDS_PER_SESSION;
    let max_word_limit = session_count * BRIEFING_MAX_WORDS_PER_SESSION;
    let warning = if session_count > BRIEFING_CONTEXT_RISK_SESSION_COUNT {
        Some(format!(
            "Selected {session_count} sessions. Context-size risk is high for larger scoped briefings and some detail may be dropped."
        ))
    } else if session_count > BRIEFING_WARN_SESSION_COUNT {
        Some(format!(
            "Selected {session_count} sessions. Briefing detail per session will be compressed to stay readable."
        ))
    } else {
        None
    };

    let cancel = app.state::<BriefingCancel>().0.clone();
    cancel.store(false, Ordering::Relaxed);
    let _ = app.emit("briefing-warning", warning);

    let app_clone = app.clone();
    std::thread::spawn(move || {
        briefing::run_briefing_stream(
            &app_clone,
            &backend,
            ctx_size,
            max_tokens,
            session_count,
            &content,
            &header,
            min_word_limit,
            max_word_limit,
            cancel,
        );
    });
    Ok(())
}

/// Delete one or more archived sessions by ID.
#[tauri::command]
pub(crate) fn delete_sessions(
    app: tauri::AppHandle,
    ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let dir = archive_dir(&app)?;
    let deleted = archive::delete_sessions(&dir, &ids).map_err(|error| error.to_string())?;
    let _ = retrieval::remove_embeddings(&dir, &deleted);
    Ok(deleted)
}

/// Preview a redaction: count occurrences and show excerpts, without writing anything.
#[tauri::command]
pub(crate) fn preview_redaction(
    app: tauri::AppHandle,
    session_id: String,
    find: String,
) -> Result<archive::RedactionPreview, String> {
    let dir = archive_dir(&app)?;
    archive::preview_redaction(&dir, &session_id, &find).map_err(|error| error.to_string())
}

/// Apply a redaction to a session's archived body, backing up the original and
/// persisting the rule so it survives future re-sweeps of this session.
#[tauri::command]
pub(crate) fn apply_redaction(
    app: tauri::AppHandle,
    session_id: String,
    find: String,
    replace: String,
) -> Result<archive::RedactionOutcome, String> {
    let dir = archive_dir(&app)?;
    archive::apply_redaction(&dir, &session_id, &find, &replace).map_err(|error| error.to_string())
}

/// Rebuild semantic embeddings for all archived sessions.
#[tauri::command]
pub(crate) async fn rebuild_session_embeddings(
    app: tauri::AppHandle,
) -> Result<retrieval::EmbeddingRebuildResult, String> {
    let dir = archive_dir(&app)?;
    let settings = settings::load_settings(&dir);
    if !retrieval::embedding_runtime_ready(&settings) {
        return Err(
            "Semantic search is not configured. Set the llama-server binary path and EmbeddingGemma GGUF model path in Settings first.".to_string(),
        );
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _inference_guard = crate::infer_lock::try_acquire().ok_or(BUSY_MESSAGE)?;
        retrieval::rebuild_embeddings_with_progress(&dir, &settings, |current, total, result| {
            let _ = app.emit(
                "embedding-rebuild-progress",
                EmbeddingRebuildProgressPayload {
                    current,
                    total,
                    indexed: result.indexed,
                    skipped: result.skipped,
                    failed: result.failed,
                },
            );
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Signal the active briefing stream to stop. Safe to call when no stream is running.
#[tauri::command]
pub(crate) fn cancel_briefing(app: tauri::AppHandle) {
    app.state::<BriefingCancel>()
        .0
        .store(true, Ordering::Relaxed);
}

/// Signal the active sweep to stop after the current session finishes.
#[tauri::command]
pub(crate) fn cancel_sweep(app: tauri::AppHandle) {
    app.state::<SweepCancel>().0.store(true, Ordering::Relaxed);
}

/// Signal the active chat turn to stop after the current blocking step.
#[tauri::command]
pub(crate) fn cancel_chat(app: tauri::AppHandle) {
    app.state::<ChatCancel>().0.store(true, Ordering::Relaxed);
}

/// Persist one HalluScribe-owned recorded Gemma chat session for later sweep ingestion.
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
) -> Result<(), String> {
    let dir = archive_dir(&app)?;
    let settings = settings::load_settings(&dir);
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
    // Chat shares the Work profile by default (Phase 2c sharing rule):
    // Personal is for the user's own companion-agent use, not the coding chat.
    let user_profile = profile::read_profile_md(&dir, profile::ProfileScope::Work);
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
