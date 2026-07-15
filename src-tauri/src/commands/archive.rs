// HalluScribe - archive and session Tauri command handlers.

use crate::app_support::{
    archive_dir, collect_raw_session_total, collect_session_stats, SessionStats,
};
use crate::{archive, retrieval, search};

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

/// Read the full markdown content of a session by its session_id.
#[tauri::command]
pub(crate) fn read_session(app: tauri::AppHandle, session_id: String) -> Result<String, String> {
    let dir = archive_dir(&app)?;
    search::read_session(&dir, &session_id)
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
