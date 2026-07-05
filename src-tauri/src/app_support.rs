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
    Ok(crate::workspace::resolve_active_dir(&default_root))
}

/// Build a SweepConfig from saved settings. Returns None if the backend config
/// is incomplete (e.g. empty llama-server bin path).
pub(crate) fn sweep_config(app: &tauri::AppHandle, force: bool) -> Option<scheduler::SweepConfig> {
    let dir = archive_dir(app).ok()?;
    let settings = settings::load_settings(&dir);
    settings.to_sweep_config(dir, force)
}

/// Return aggregate stats across all archived sessions.
pub(crate) fn collect_session_stats(app: &tauri::AppHandle) -> Result<SessionStats, String> {
    let dir = archive_dir(app)?;
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
    let settings = settings::load_settings(&dir);
    Ok(scanner::scan_sessions(&dir, &settings, u64::MAX, 0.0)
        .into_iter()
        .map(|target| match crate::readers::read_target(&target) {
            Ok(parsed_sessions) => parsed_sessions.len() as u32,
            Err(_) => 0,
        })
        .sum())
}

/// After a successful sweep, flip `first_run` to false so subsequent sweeps
/// use the normal lookback_hours window instead of scanning all history.
pub(crate) fn clear_first_run(app: &tauri::AppHandle) {
    if let Ok(dir) = archive_dir(app) {
        let mut settings = settings::load_settings(&dir);
        if settings.first_run {
            settings.first_run = false;
            let _ = settings::save_settings(&dir, &settings);
        }
    }
}

/// Record today's local date as the last successful sweep date, so the
/// catch-up scheduler runs the nightly sweep at most once per day (audit A-2).
pub(crate) fn record_sweep_date(app: &tauri::AppHandle) {
    if let Ok(dir) = archive_dir(app) {
        let mut settings = settings::load_settings(&dir);
        let today = Local::now().format("%Y-%m-%d").to_string();
        if settings.last_sweep_date != today {
            settings.last_sweep_date = today;
            let _ = settings::save_settings(&dir, &settings);
        }
    }
}

fn top_n(counts: HashMap<String, u32>, n: usize) -> Vec<String> {
    let mut pairs: Vec<(String, u32)> = counts.into_iter().collect();
    pairs.sort_by_key(|pair| std::cmp::Reverse(pair.1));
    pairs.into_iter().take(n).map(|(tag, _)| tag).collect()
}
