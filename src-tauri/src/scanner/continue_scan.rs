// HalluScribe - Continue session discovery and fill_pct correlation.

use super::{ScanTarget, ScanTargetKind, ToolSource};
use crate::scanner::shared::home_dir;
use chrono::DateTime;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const CONTINUE_CORRELATION_MS: i64 = 120_000;

#[derive(Deserialize, Default)]
struct ContinueConfig {
    #[serde(default)]
    models: Vec<ContinueModelEntry>,
}

#[derive(Deserialize)]
struct ContinueModelEntry {
    model: Option<String>,
    name: Option<String>,
    #[serde(rename = "contextLength", alias = "context_length")]
    context_length: Option<u64>,
}

#[derive(Clone)]
struct ContinueModelConfig {
    canonical_model_id: String,
    context_window: u64,
}

#[derive(Clone)]
struct ContinueChatEvent {
    session_id: String,
    timestamp_ms: i64,
    model_id: String,
    model_title: Option<String>,
}

#[derive(Clone)]
struct ContinueTokenEvent {
    timestamp_ms: i64,
    model_id: String,
    prompt_tokens: u64,
}

fn continue_root() -> Option<PathBuf> {
    let home = home_dir()?;
    #[cfg(target_os = "windows")]
    let root = home.join("AppData").join("Roaming").join(".continue");
    #[cfg(not(target_os = "windows"))]
    let root = home.join(".continue");
    Some(root)
}

pub(super) fn scan_continue(
    lookback_secs: u64,
    min_fill_pct: f64,
    override_root: Option<&Path>,
) -> Vec<ScanTarget> {
    let root = if let Some(path) = override_root {
        path.to_path_buf()
    } else {
        let Some(r) = continue_root() else {
            return Vec::new();
        };
        r
    };
    scan_continue_from_root(&root, lookback_secs, min_fill_pct)
}

pub(super) fn scan_continue_from_root(
    root: &Path,
    lookback_secs: u64,
    min_fill_pct: f64,
) -> Vec<ScanTarget> {
    let event_dir = root.join("dev_data").join("0.2.0");
    let chat_path = event_dir.join("chatInteraction.jsonl");
    let tokens_path = event_dir.join("tokensGenerated.jsonl");
    let cutoff_ms = recent_cutoff_ms(lookback_secs);

    let model_map = continue_model_map(root);
    if model_map.is_empty() {
        return Vec::new();
    }

    let tokens = recent_token_events(&tokens_path, cutoff_ms)
        .into_iter()
        .map(|token| ContinueTokenEvent {
            model_id: canonical_model_id(&token.model_id, None, &model_map),
            ..token
        })
        .collect::<Vec<_>>();
    let mut by_session = HashMap::<String, (i64, ScanTarget)>::new();

    for chat in recent_chat_events(&chat_path, cutoff_ms) {
        let canonical_chat_model =
            canonical_model_id(&chat.model_id, chat.model_title.as_deref(), &model_map);
        let Some(matched) = best_token_match(&canonical_chat_model, chat.timestamp_ms, &tokens)
        else {
            continue;
        };
        let Some(fill_pct) =
            compute_fill_pct(&canonical_chat_model, matched.prompt_tokens, &model_map)
        else {
            continue;
        };
        if fill_pct < min_fill_pct {
            continue;
        }

        let session_path = root
            .join("sessions")
            .join(format!("{}.json", chat.session_id));
        if !session_path.exists() {
            continue;
        }

        let target = ScanTarget {
            path: session_path,
            kind: ScanTargetKind::Coding(ToolSource::Continue),
            fill_pct: Some(fill_pct),
            mtime_secs: chat.timestamp_ms / 1000,
        };

        let entry = by_session
            .entry(chat.session_id.clone())
            .or_insert_with(|| (chat.timestamp_ms, target.clone()));
        if chat.timestamp_ms >= entry.0 {
            *entry = (chat.timestamp_ms, target);
        }
    }

    let mut sessions = by_session
        .into_values()
        .map(|(_, target)| target)
        .collect::<Vec<_>>();
    sessions.sort_by_key(|target| std::cmp::Reverse(target.mtime_secs));
    sessions
}

fn continue_model_map(root: &Path) -> HashMap<String, ContinueModelConfig> {
    let Ok(content) = fs::read_to_string(root.join("config.yaml")) else {
        return HashMap::new();
    };
    let config: ContinueConfig = serde_yaml::from_str(&content).unwrap_or_default();
    let mut map = HashMap::new();
    for model in config.models {
        let Some(context_window) = model.context_length.filter(|&ctx| ctx > 0) else {
            continue;
        };
        let normalized_model = model
            .model
            .as_deref()
            .map(normalize_model_id)
            .filter(|name| !name.is_empty());
        let normalized_name = model
            .name
            .as_deref()
            .map(normalize_model_id)
            .filter(|name| !name.is_empty());
        let canonical_model_id = normalized_model
            .clone()
            .or_else(|| normalized_name.clone())
            .unwrap_or_default();
        if canonical_model_id.is_empty() {
            continue;
        }
        let config = ContinueModelConfig {
            canonical_model_id: canonical_model_id.clone(),
            context_window,
        };
        if let Some(key) = normalized_model {
            map.insert(key, config.clone());
        }
        if let Some(key) = normalized_name {
            map.entry(key).or_insert_with(|| config.clone());
        }
    }
    map
}

fn recent_cutoff_ms(lookback_secs: u64) -> i64 {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(lookback_secs))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    match cutoff.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => duration.as_millis().min(i64::MAX as u128) as i64,
        Err(_) => 0,
    }
}

fn recent_chat_events(path: &Path, cutoff_ms: i64) -> Vec<ContinueChatEvent> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(parse_continue_chat_line)
        .filter(|event| event.timestamp_ms >= cutoff_ms)
        .collect()
}

fn recent_token_events(path: &Path, cutoff_ms: i64) -> Vec<ContinueTokenEvent> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(parse_continue_token_event_line)
        .filter(|event| event.timestamp_ms >= cutoff_ms)
        .collect()
}

fn parse_continue_chat_line(line: &str) -> Option<ContinueChatEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    let session_id = value.get("sessionId")?.as_str()?.trim().to_string();
    let model_id = normalize_model_id(value.get("modelName")?.as_str()?);
    let timestamp_ms = parse_timestamp_ms(value.get("timestamp")?.as_str()?)?;
    let model_title = value
        .get("modelTitle")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToString::to_string);
    if session_id.is_empty() || model_id.is_empty() {
        return None;
    }
    Some(ContinueChatEvent {
        session_id,
        timestamp_ms,
        model_id,
        model_title,
    })
}

fn parse_continue_token_event_line(line: &str) -> Option<ContinueTokenEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    let model_id = normalize_model_id(value.get("model")?.as_str()?);
    let prompt_tokens = value.get("promptTokens")?.as_u64()?;
    let timestamp_ms = parse_timestamp_ms(value.get("timestamp")?.as_str()?)?;
    if model_id.is_empty() || prompt_tokens == 0 {
        return None;
    }
    Some(ContinueTokenEvent {
        timestamp_ms,
        model_id,
        prompt_tokens,
    })
}

#[cfg(test)]
pub(super) fn parse_continue_token_line(line: &str) -> Option<(String, u64)> {
    let event = parse_continue_token_event_line(line)?;
    Some((event.model_id, event.prompt_tokens))
}

fn best_token_match(
    model_id: &str,
    chat_timestamp_ms: i64,
    tokens: &[ContinueTokenEvent],
) -> Option<ContinueTokenEvent> {
    tokens
        .iter()
        .filter(|token| {
            token.model_id == model_id
                && (token.timestamp_ms - chat_timestamp_ms).abs() <= CONTINUE_CORRELATION_MS
        })
        .max_by_key(|token| token.timestamp_ms)
        .cloned()
}

fn compute_fill_pct(
    model_id: &str,
    prompt_tokens: u64,
    model_map: &HashMap<String, ContinueModelConfig>,
) -> Option<f64> {
    let context_window = model_map.get(model_id)?.context_window as f64;
    Some((prompt_tokens as f64 / context_window * 100.0).clamp(0.0, 100.0))
}

fn canonical_model_id(
    model_id: &str,
    model_title: Option<&str>,
    model_map: &HashMap<String, ContinueModelConfig>,
) -> String {
    let normalized_id = normalize_model_id(model_id);
    let normalized_title = model_title.map(normalize_model_id);
    model_map
        .get(&normalized_id)
        .or_else(|| {
            normalized_title
                .as_ref()
                .and_then(|title| model_map.get(title))
        })
        .map(|config| config.canonical_model_id.clone())
        .unwrap_or(normalized_id)
}

fn normalize_model_id(value: &str) -> String {
    value.trim().to_lowercase()
}

fn parse_timestamp_ms(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}
