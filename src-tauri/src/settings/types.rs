// HalluScribe - persisted settings types and defaults.
use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize};
use std::{fmt, io};

/// Which inference backend the user has chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    #[default]
    LlamaCpp,
    Ollama,
}

#[derive(Debug)]
pub enum SettingsError {
    Io(io::Error),
    Serialize(serde_json::Error),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "settings I/O error: {error}"),
            Self::Serialize(error) => write!(f, "settings serialize error: {error}"),
        }
    }
}

impl From<io::Error> for SettingsError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for SettingsError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialize(error)
    }
}

/// All user-configurable HalluScribe options.
/// Stored as JSON in `<archive_dir>/settings.json`.
/// All fields have serde defaults - a partial or absent file is always valid.
/// `ctx_size` and `max_tokens` default to zero and must be set before generation can run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HalluScribeSettings {
    pub always_on_top: bool,
    pub backend: BackendKind,
    pub llama_server_bin: String,
    pub gemma_model_path: String,
    pub embedding_model_path: String,
    pub gpu_layers: i32,
    pub llama_server_port: u16,
    pub ollama_host: String,
    pub ollama_port: u16,
    pub ollama_model: String,
    pub ollama_api_key: String,
    pub tavily_api_key: String,
    pub summary_min_fill_pct: f64,
    pub lookback_hours: u64,
    pub chatgpt_import_path: String,
    pub claudeai_import_path: String,
    pub gemini_import_path: String,
    pub ollama_chat_db_path: String,
    pub continue_data_path: String,
    pub forge_sessions_path: String,
    pub scheduled_processing_enabled: bool,
    #[serde(deserialize_with = "deserialize_schedule_time")]
    pub schedule_time: String,
    /// Date (`YYYY-MM-DD`, local) of the last successful sweep. Empty until the
    /// first sweep completes. Drives the catch-up scheduler: a sweep is due once
    /// per day, any time at or after `schedule_time`, rather than in a single
    /// 60-second window (audit A-2).
    pub last_sweep_date: String,
    pub idle_threshold_mins: u32,
    pub first_run: bool,
    pub ctx_size: u32,
    pub max_tokens: u32,
    pub briefing_window_hours: u32,
}

impl Default for HalluScribeSettings {
    fn default() -> Self {
        Self {
            always_on_top: false,
            backend: BackendKind::LlamaCpp,
            llama_server_bin: String::new(),
            gemma_model_path: String::new(),
            embedding_model_path: String::new(),
            gpu_layers: -1,
            llama_server_port: 8080,
            ollama_host: "localhost".to_string(),
            ollama_port: 11434,
            ollama_model: "gemma4:26b".to_string(),
            ollama_api_key: String::new(),
            tavily_api_key: String::new(),
            summary_min_fill_pct: 50.0,
            lookback_hours: 24,
            chatgpt_import_path: String::new(),
            claudeai_import_path: String::new(),
            gemini_import_path: String::new(),
            ollama_chat_db_path: String::new(),
            continue_data_path: String::new(),
            forge_sessions_path: String::new(),
            scheduled_processing_enabled: true,
            schedule_time: "02:00".to_string(),
            last_sweep_date: String::new(),
            idle_threshold_mins: 30,
            first_run: true,
            ctx_size: 0,
            max_tokens: 0,
            briefing_window_hours: 2,
        }
    }
}

pub(crate) fn parse_schedule_time(value: &str) -> Option<(u32, u32)> {
    let trimmed = value.trim();
    let (hour, minute) = trimmed.split_once(':')?;
    let hour = hour.parse::<u32>().ok()?;
    let minute = minute.parse::<u32>().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

fn normalize_schedule_time(value: &str) -> Option<String> {
    let (hour, minute) = parse_schedule_time(value)?;
    Some(format!("{hour:02}:{minute:02}"))
}

fn deserialize_schedule_time<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(text) => normalize_schedule_time(&text)
            .ok_or_else(|| D::Error::custom("schedule_time must be HH:MM in 24-hour format")),
        serde_json::Value::Number(number) => number
            .as_u64()
            .filter(|hour| *hour <= 23)
            .map(|hour| format!("{hour:02}:00"))
            .ok_or_else(|| D::Error::custom("legacy schedule_hour must be 0-23")),
        other => Err(D::Error::custom(format!(
            "schedule_time must be a string or legacy hour number, got {other}"
        ))),
    }
}
