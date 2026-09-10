// HalluScribe - settings and credential-validation Tauri command handlers.

use crate::app_support::archive_dir;
use crate::{briefing, scanner, settings};
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct ApiKeyValidationResult {
    pub status: String,
    pub message: String,
}

/// Load current settings from disk. Blank import paths are derived from the
/// active archive root (and their folders created) before the UI ever sees
/// them, so no workspace can drift back to an invented layout.
#[tauri::command]
pub(crate) fn get_settings(app: tauri::AppHandle) -> Result<settings::HalluScribeSettings, String> {
    let dir = archive_dir(&app)?;
    settings::load_with_import_paths(&dir).map_err(|error| error.to_string())
}

/// Report, per chat-import provider, whether an export is actually reachable
/// at the configured path (GROK_IMPORT_PLAN § 12.4.4).
///
/// Answers from `scanner::chat_import_sources` - the same authority the sweep
/// uses - so Settings can never claim an export the sweep would miss, or miss
/// one it would find. Runs the bounded discovery walk for up to four providers,
/// so it is async and off the main thread like the other walking commands.
#[tauri::command]
pub(crate) async fn chat_import_status(
    app: tauri::AppHandle,
) -> Result<Vec<scanner::import_status::ImportPathStatus>, String> {
    let dir = archive_dir(&app)?;
    let current = settings::load_with_import_paths(&dir).map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        scanner::import_status::all_import_statuses(&current)
    })
    .await
    .map_err(|error| format!("import status check failed: {error}"))
}

/// Persist updated settings to disk.
#[tauri::command]
pub(crate) fn save_settings(
    app: tauri::AppHandle,
    mut new_settings: settings::HalluScribeSettings,
) -> Result<(), String> {
    let dir = archive_dir(&app)?;
    // Catch a model path pasted into the binary field (or vice versa) here,
    // while we can still name the offending field. Past this point the mistake
    // only shows up as a raw OS spawn error at the next inference.
    settings::validate_paths(&new_settings)?;
    // Scheduler state is backend-owned. Preserve it so a Settings save cannot
    // reset a successful-run marker or automatic retry budget.
    let saved = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    new_settings.last_sweep_date = saved.last_sweep_date;
    new_settings.last_auto_sweep_attempt_at = saved.last_auto_sweep_attempt_at;
    new_settings.auto_sweep_retry_count = saved.auto_sweep_retry_count;
    new_settings.auto_sweep_exhausted_date = saved.auto_sweep_exhausted_date;
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
