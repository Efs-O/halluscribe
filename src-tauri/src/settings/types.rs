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
    Json(serde_json::Error),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "settings I/O error: {error}"),
            Self::Json(error) => write!(f, "settings JSON error: {error}"),
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
        Self::Json(error)
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
    /// Last window size the user resized to, in physical pixels. `None` means
    /// "never resized" and leaves tauri.conf.json's dimensions in charge, so a
    /// fresh install still opens at the designed size.
    pub window_width: Option<u32>,
    pub window_height: Option<u32>,
    pub backend: BackendKind,
    pub llama_server_bin: String,
    pub gemma_model_path: String,
    pub embedding_model_path: String,
    pub gpu_layers: i32,
    /// `--device`: which llama.cpp devices may hold the model, comma-separated
    /// (`CUDA0,CUDA1`) as printed by `llama-server --list-devices`. Empty = use
    /// every visible device. Set this to pin work to one card on a mixed-GPU
    /// machine, where enumeration order otherwise decides.
    pub gpu_devices: String,
    /// `--split-mode`: `none`, `layer`, `row` or `tensor`. Empty = llama.cpp's
    /// default (`layer`).
    pub gpu_split_mode: String,
    /// `--tensor-split`: per-device fractions of the model in device order
    /// (`0.7,0.3`). Empty = split evenly.
    pub gpu_tensor_split: String,
    /// `--main-gpu`: device holding intermediate results and the KV cache.
    /// -1 = unset, leaving llama.cpp's default of device 0.
    pub gpu_main_index: i32,
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
    pub grok_import_path: String,
    pub ollama_chat_db_path: String,
    pub forge_sessions_path: String,
    pub scheduled_processing_enabled: bool,
    #[serde(deserialize_with = "deserialize_schedule_time")]
    pub schedule_time: String,
    /// Date (`YYYY-MM-DD`, local) of the last successful sweep. Empty until the
    /// first sweep completes. Drives the catch-up scheduler: a sweep is due once
    /// per day, any time at or after `schedule_time`, rather than in a single
    /// 60-second window (audit A-2).
    pub last_sweep_date: String,
    /// Date (`YYYY-MM-DD`, local) when the scheduled runner most recently made
    /// its one automatic attempt. This remains separate from `last_sweep_date`:
    /// a failed or busy automatic run is still an attempt, not a success.
    pub last_auto_sweep_attempt_date: String,
    pub idle_threshold_mins: u32,
    pub first_run: bool,
    pub ctx_size: u32,
    pub max_tokens: u32,
    pub briefing_window_hours: u32,
    /// Provider keys (see `ChatProvider::provider_key`) whose sessions the
    /// profile distiller may read. Default: coding/chat tools in, personal
    /// chat exports (ChatGPT, Claude.ai, Gemini) out — per the Persona
    /// Protocol plan's consent decision.
    pub profile_sources: Vec<String>,
    /// Absolute path to the piper TTS binary. Empty = auto-search
    /// `~/.halluscribe/tts/piper`.
    pub tts_piper_bin: String,
    /// Bare name (file stem, not full path) of the selected voice in
    /// `~/.halluscribe/tts/voices`. Empty = no voice selected.
    pub tts_voice: String,
}

fn default_profile_sources() -> Vec<String> {
    [
        "claude_code",
        "codex",
        "forge",
        "halluscribe_agent_chat",
        "ollama_chat",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

impl Default for HalluScribeSettings {
    fn default() -> Self {
        Self {
            always_on_top: false,
            window_width: None,
            window_height: None,
            backend: BackendKind::LlamaCpp,
            llama_server_bin: String::new(),
            gemma_model_path: String::new(),
            embedding_model_path: String::new(),
            gpu_layers: -1,
            // All four unset: a fresh install behaves exactly as llama.cpp
            // would on its own. Placement is opt-in, never guessed.
            gpu_devices: String::new(),
            gpu_split_mode: String::new(),
            gpu_tensor_split: String::new(),
            gpu_main_index: -1,
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
            grok_import_path: String::new(),
            ollama_chat_db_path: String::new(),
            forge_sessions_path: String::new(),
            // Off by default: sweeps are manual ("Run Now") unless the user opts
            // in. An unattended auto-sweep runs against whatever workspace is
            // active — including a freshly created guest, which the catch-up
            // scheduler treats as immediately overdue — so defaulting it on risks
            // ingesting the host's local coding logs into someone else's profile.
            scheduled_processing_enabled: false,
            schedule_time: "02:00".to_string(),
            last_sweep_date: String::new(),
            last_auto_sweep_attempt_date: String::new(),
            idle_threshold_mins: 30,
            first_run: true,
            ctx_size: 0,
            max_tokens: 0,
            briefing_window_hours: 2,
            profile_sources: default_profile_sources(),
            tts_piper_bin: String::new(),
            tts_voice: String::new(),
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
