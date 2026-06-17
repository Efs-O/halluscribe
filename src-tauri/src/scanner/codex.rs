use super::shared::{collect_jsonl, cutoff, home_dir, mtime_secs};
use super::{ScanTarget, ScanTargetKind, ToolSource};
use serde_json::Value;
use std::fs;

pub(super) fn scan_codex(lookback_secs: u64, min_fill_pct: f64) -> Vec<ScanTarget> {
    let Some(dir) = home_dir().map(|home| home.join(".codex").join("sessions")) else {
        return Vec::new();
    };
    let gate = cutoff(lookback_secs);

    collect_jsonl(&dir)
        .into_iter()
        .filter(|(mtime, _)| *mtime >= gate)
        .filter_map(|(mtime, path)| {
            let content = fs::read_to_string(&path).ok()?;
            let fill_pct = codex_fill_pct(&content)?;
            (fill_pct >= min_fill_pct).then_some(ScanTarget {
                path,
                kind: ScanTargetKind::Coding(ToolSource::Codex),
                fill_pct: Some(fill_pct),
                mtime_secs: mtime_secs(mtime),
            })
        })
        .collect()
}

pub(super) fn codex_fill_pct(content: &str) -> Option<f64> {
    let mut last = None;
    for line in content.lines() {
        if let Some(pct) = parse_codex_token_count(line) {
            last = Some(pct);
        }
    }
    last
}

pub(super) fn parse_codex_token_count(line: &str) -> Option<f64> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }
    let payload = value.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let info = payload.get("info").filter(|info| !info.is_null())?;
    let input = info
        .get("last_token_usage")
        .and_then(|usage| usage.get("input_tokens"))
        .and_then(Value::as_f64)?;
    let ctx = info.get("model_context_window").and_then(Value::as_f64)?;
    (ctx > 0.0).then_some((input / ctx * 100.0).clamp(0.0, 100.0))
}
