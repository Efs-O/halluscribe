// HalluScribe - settings and credential-validation Tauri command handlers.

use crate::app_support::archive_dir;
use crate::{briefing, settings};
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct ApiKeyValidationResult {
    pub status: String,
    pub message: String,
}

/// Load current settings from disk.
#[tauri::command]
pub(crate) fn get_settings(app: tauri::AppHandle) -> Result<settings::HalluScribeSettings, String> {
    let dir = archive_dir(&app)?;
    Ok(settings::load_settings(&dir))
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
