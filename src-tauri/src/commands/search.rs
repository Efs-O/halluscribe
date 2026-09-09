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
    archive::ensure_index_readable(&dir).map_err(|error| error.to_string())?;
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
///
/// Async and off the main thread on purpose: a sync command runs on the thread
/// pumping the window's event loop, so a multi-second scan froze the whole UI
/// ("Not Responding") rather than merely delaying the results. The scan itself
/// is fast once the search body cache is warm, but the first call after a
/// sweep still reads the corpus, and that must not block painting.
#[tauri::command]
pub(crate) async fn search_sessions_fulltext(
    app: tauri::AppHandle,
    query: String,
) -> Result<Vec<archive::IndexEntry>, String> {
    let dir = archive_dir(&app)?;
    archive::ensure_index_readable(&dir).map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || search::search_fulltext(&dir, &query))
        .await
        .map_err(|error| format!("search failed: {error}"))
}

/// Brute-force scan of preserved raw transcripts (SESSION SUMMARY raw scope).
#[tauri::command]
pub(crate) fn search_raw_transcripts(
    app: tauri::AppHandle,
    query: String,
) -> Result<search::RawSearchResult, String> {
    let dir = archive_dir(&app)?;
    archive::ensure_index_readable(&dir).map_err(|error| error.to_string())?;
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
    archive::ensure_index_readable(&dir).map_err(|error| error.to_string())?;
    let settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    let allowed_ids = allowed_session_ids
        .filter(|ids| !ids.is_empty())
        .map(|ids| ids.into_iter().collect::<HashSet<_>>());
    // Embedding loads a second model. Reclaim an idle warm chat server first so
    // semantic search cannot overcommit VRAM on constrained GPUs.
    let _inference_guard = crate::infer_lock::acquire_for_batch().ok_or(BUSY_MESSAGE)?;
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
    archive::ensure_index_readable(&dir).map_err(|error| error.to_string())?;
    let settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    if !retrieval::embedding_runtime_ready(&settings) {
        return Err(
            "Semantic search is not configured. Set the llama-server binary path and EmbeddingGemma GGUF model path in Settings first.".to_string(),
        );
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _inference_guard = crate::infer_lock::acquire_for_batch().ok_or(BUSY_MESSAGE)?;
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
