// HalluScribe - shared Tauri-side application support helpers.

use crate::{archive, scanner, scheduler, settings};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};
use tauri::Manager;

/// Aggregated archive statistics returned to the frontend.
#[derive(Debug, Serialize)]
pub(crate) struct SessionStats {
    pub total: u32,
    pub raw_total: Option<u32>,
    pub by_tool: HashMap<String, u32>,
    pub by_type: HashMap<String, u32>,
    pub top_error_tags: Vec<String>,
    pub top_topic_tags: Vec<String>,
}

/// One message in a chat conversation (mirrors frontend ChatMessage type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatImage {
    pub mime_type: String,
    pub data: String,
}

/// One message in a chat conversation (mirrors frontend ChatMessage type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ChatImage>>,
}

/// Absolute path to the DEFAULT archive root (`<home>/.halluscribe`). The
/// workspace registry and host-global markers always live here, regardless of
/// which workspace is active.
pub(crate) fn default_archive_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .home_dir()
        .map(|home| home.join(".halluscribe"))
        .map_err(|error| error.to_string())
}

/// Resolve the archive root: the active workspace root, or the default root when
/// no workspace is active (full back-compat). All archive reads/writes key off
/// this one chokepoint.
pub(crate) fn archive_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let default_root = default_archive_dir(app)?;
    crate::workspace::resolve_active_dir(&default_root)
}

/// Build a SweepConfig from saved settings. Automatic-clock admission is kept
/// separate in `scheduler::automatic` so configuration failures, backoff, and
/// exhaustion cannot all collapse into one ambiguous `None`.
pub(crate) fn sweep_config(
    app: &tauri::AppHandle,
    force: bool,
) -> Result<Option<scheduler::SweepConfig>, String> {
    let dir = archive_dir(app)?;
    let default_root = default_archive_dir(app)?;
    let import_only = crate::workspace::is_active_import_only(&default_root, &dir)?;
    let settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    let Some(mut config) = settings.to_sweep_config(dir, force) else {
        return Ok(None);
    };
    config.import_only = import_only;
    Ok(Some(config))
}

/// Return aggregate stats across all archived sessions.
pub(crate) fn collect_session_stats(app: &tauri::AppHandle) -> Result<SessionStats, String> {
    let dir = archive_dir(app)?;
    archive::ensure_index_readable(&dir).map_err(|error| error.to_string())?;
    let sessions = archive::read_sessions(&dir);

    let mut by_tool: HashMap<String, u32> = HashMap::new();
    let mut by_type: HashMap<String, u32> = HashMap::new();
    let mut error_counts: HashMap<String, u32> = HashMap::new();
    let mut topic_counts: HashMap<String, u32> = HashMap::new();

    for session in &sessions {
        *by_tool.entry(session.tool.clone()).or_insert(0) += 1;
        *by_type.entry(session.session_type.clone()).or_insert(0) += 1;
        for tag in &session.error_tags {
            *error_counts.entry(tag.clone()).or_insert(0) += 1;
        }
        for tag in &session.topic_tags {
            *topic_counts.entry(tag.clone()).or_insert(0) += 1;
        }
    }

    Ok(SessionStats {
        total: sessions.len() as u32,
        raw_total: None,
        by_tool,
        by_type,
        top_error_tags: top_n(error_counts, 10),
        top_topic_tags: top_n(topic_counts, 10),
    })
}

pub(crate) fn collect_raw_session_total(app: &tauri::AppHandle) -> Result<u32, String> {
    let dir = archive_dir(app)?;
    let default_root = default_archive_dir(app)?;
    let import_only = crate::workspace::is_active_import_only(&default_root, &dir)?;
    let settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    Ok(
        scanner::scan_sessions(&dir, &settings, u64::MAX, 0.0, import_only)
            .into_iter()
            .map(|target| {
                match crate::readers::read_target(
                    &target,
                    settings.business_default_country_code_opt(),
                ) {
                    Ok(parsed_sessions) => parsed_sessions.len() as u32,
                    Err(_) => 0,
                }
            })
            .sum(),
    )
}

/// Persist both sweep-success markers in one write. This avoids recording a
/// day as complete while leaving `first_run` behind after a partial failure.
pub(crate) fn mark_sweep_success(app: &tauri::AppHandle) -> Result<(), String> {
    let dir = archive_dir(app)?;
    let mut settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    let today = Local::now().format("%Y-%m-%d").to_string();
    let changed = settings.first_run || settings.last_sweep_date != today;
    if changed {
        settings.first_run = false;
        settings.last_sweep_date = today;
        settings::save_settings(&dir, &settings).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Read the pure scheduled-sweep admission state using the local wall clock.
pub(crate) fn automatic_sweep_admission(
    app: &tauri::AppHandle,
) -> Result<scheduler::AutomaticAdmission, String> {
    let dir = archive_dir(app)?;
    let settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    Ok(scheduler::admit(&settings, scheduler::now_fixed()))
}

/// Persist an automatic attempt before it competes for inference. A busy run
/// spends the same bounded budget as a failed run, so it cannot become a
/// tight retry loop.
pub(crate) fn mark_auto_sweep_attempt(app: &tauri::AppHandle) -> Result<(), String> {
    let dir = archive_dir(app)?;
    let mut settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    let admission = scheduler::admit(&settings, scheduler::now_fixed());
    scheduler::record_attempt(&mut settings, scheduler::now_fixed(), admission);
    settings::save_settings(&dir, &settings).map_err(|error| error.to_string())?;
    Ok(())
}

/// Persist the once-per-day terminal exhaustion notification state.
pub(crate) fn mark_auto_sweep_exhausted(app: &tauri::AppHandle) -> Result<(), String> {
    let dir = archive_dir(app)?;
    let mut settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    scheduler::record_exhaustion(&mut settings, scheduler::now_fixed());
    settings::save_settings(&dir, &settings).map_err(|error| error.to_string())
}

/// Automatic success owns its retry state; manual success intentionally does
/// not mutate it, beyond the shared `last_sweep_date` completion marker.
pub(crate) fn mark_auto_sweep_success(app: &tauri::AppHandle) -> Result<(), String> {
    let dir = archive_dir(app)?;
    let mut settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    scheduler::record_success(&mut settings, scheduler::now_fixed());
    settings::save_settings(&dir, &settings).map_err(|error| error.to_string())
}

fn top_n(counts: HashMap<String, u32>, n: usize) -> Vec<String> {
    let mut pairs: Vec<(String, u32)> = counts.into_iter().collect();
    pairs.sort_by_key(|pair| std::cmp::Reverse(pair.1));
    pairs.into_iter().take(n).map(|(tag, _)| tag).collect()
}
