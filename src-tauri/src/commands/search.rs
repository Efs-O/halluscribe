// HalluScribe - archive search and embedding Tauri command handlers.

use super::BUSY_MESSAGE;
use crate::app_support::archive_dir;
use crate::{archive, retrieval, search, settings};
use serde::Serialize;
use std::collections::HashSet;
use tauri::Emitter;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EmbeddingRebuildProgressPayload {
    pub current: usize,
    pub total: usize,
    pub indexed: u32,
    pub skipped: u32,
    pub failed: u32,
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

/// Full-text session search for the SESSION SUMMARY tab.
#[tauri::command]
pub(crate) fn search_sessions_fulltext(
    app: tauri::AppHandle,
    query: String,
) -> Result<Vec<archive::IndexEntry>, String> {
    let dir = archive_dir(&app)?;
    Ok(search::search_fulltext(&dir, &query))
}

/// Brute-force scan of preserved raw transcripts (SESSION SUMMARY raw scope).
#[tauri::command]
pub(crate) fn search_raw_transcripts(
    app: tauri::AppHandle,
    query: String,
) -> Result<search::RawSearchResult, String> {
    let dir = archive_dir(&app)?;
    search::search_raw(&dir, &query, None).map_err(|error| error.to_string())
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
