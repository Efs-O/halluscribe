use super::shared::{collect_jsonl, cutoff, home_dir, mtime_secs};
use super::{ScanTarget, ScanTargetKind, ToolSource};
use crate::core::context_window_for_model;
use serde_json::Value;
use std::fs;

/// Claude Code autocompact buffer reservation (33 000 tokens).
const CLAUDE_RESERVE: f64 = 33_000.0;

pub(super) fn scan_claude(lookback_secs: u64, min_fill_pct: f64) -> Vec<ScanTarget> {
    let Some(dir) = home_dir().map(|home| home.join(".claude").join("projects")) else {
        return Vec::new();
    };
    let gate = cutoff(lookback_secs);

    collect_jsonl(&dir)
        .into_iter()
        .filter(|(mtime, _)| *mtime >= gate)
        .filter_map(|(mtime, path)| {
            let content = fs::read_to_string(&path).ok()?;
            let fill_pct = claude_fill_pct(&content)?;
            (fill_pct >= min_fill_pct).then_some(ScanTarget {
                path,
                kind: ScanTargetKind::Coding(ToolSource::ClaudeCode),
                fill_pct: Some(fill_pct),
                mtime_secs: mtime_secs(mtime),
            })
        })
        .collect()
}

pub(super) fn claude_fill_pct(content: &str) -> Option<f64> {
    content
        .lines()
        .filter_map(parse_claude_usage_line)
        .next_back()
        .map(|(_, pct)| pct)
}

pub(super) fn parse_claude_usage_line(line: &str) -> Option<(String, f64)> {
    let value: Value = serde_json::from_str(line).ok()?;
    let message = value.get("message")?;
    let model = message
        .get("model")?
        .as_str()
        .filter(|name| !name.is_empty())?;
    let usage = message.get("usage")?;
    let get = |key: &str| usage.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let total = get("input_tokens")
        + get("cache_read_input_tokens")
        + get("cache_creation_input_tokens")
        + get("output_tokens")
        + CLAUDE_RESERVE;
    let ctx = context_window_for_model(model) as f64;
    Some((model.to_string(), (total / ctx * 100.0).clamp(0.0, 100.0)))
}
