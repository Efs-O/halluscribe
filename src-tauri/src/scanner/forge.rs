// HalluScribe - Forge VS Code extension session discovery.
// Reads JSONL session files written by the Forge extension to ~/.forge/sessions/.
// Each file is one conversation: <session-id>.jsonl

use super::shared::{collect_jsonl, cutoff, home_dir, mtime_secs};
use super::{ScanTarget, ScanTargetKind, ToolSource};
use serde_json::Value;
use std::fs;
use std::path::Path;

pub(super) fn scan_forge(
    lookback_secs: u64,
    min_fill_pct: f64,
    override_root: Option<&Path>,
) -> Vec<ScanTarget> {
    let root = if let Some(path) = override_root {
        path.to_path_buf()
    } else {
        let Some(r) = home_dir().map(|home| home.join(".forge").join("sessions")) else {
            return Vec::new();
        };
        r
    };
    scan_forge_from_root(&root, lookback_secs, min_fill_pct)
}

pub(super) fn scan_forge_from_root(
    root: &Path,
    lookback_secs: u64,
    min_fill_pct: f64,
) -> Vec<ScanTarget> {
    if !root.is_dir() {
        return Vec::new();
    }
    let gate = cutoff(lookback_secs);
    collect_jsonl(root)
        .into_iter()
        .filter(|(mtime, _)| *mtime >= gate)
        .filter_map(|(mtime, path)| {
            let content = fs::read_to_string(&path).ok()?;
            let fill_pct = forge_fill_pct(&content);
            // A Forge session without a compaction row has no real fill
            // measurement, so it is kept (unknown fill bypasses the gate)
            // rather than silently discarded.
            (fill_pct.map(|pct| pct >= min_fill_pct).unwrap_or(true)).then_some(ScanTarget {
                path,
                kind: ScanTargetKind::Coding(ToolSource::Forge),
                fill_pct,
                mtime_secs: mtime_secs(mtime),
            })
        })
        .collect()
}

/// Best-observed context fill for a Forge session, from its `compaction` rows.
///
/// Forge's `usage.input_tokens` is a cumulative running sum that the logger
/// flushes once per turn, while a turn may contain several model rounds — so a
/// persisted usage delta can span multiple request prompts and overstates peak
/// context. Only a `compaction` row records an exact occupancy: `used_tokens`
/// over `max_tokens`, where `max_tokens` is Forge's own resolved per-slot
/// window. The largest valid ratio across the session is the best-observed
/// fill. `None` means no compaction row was present, so the caller must keep
/// the session and let the reader fall back to an estimated fill.
pub(super) fn forge_fill_pct(content: &str) -> Option<f64> {
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|value| value.get("type").and_then(Value::as_str) == Some("compaction"))
        .filter_map(|value| {
            let used = value.get("used_tokens").and_then(Value::as_f64)?;
            let max = value.get("max_tokens").and_then(Value::as_f64)?;
            (used >= 0.0 && max > 0.0).then_some((used / max * 100.0).clamp(0.0, 100.0))
        })
        .max_by(f64::total_cmp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn forge_scan_discovers_session_jsonl_files() {
        let dir = tempdir().unwrap();
        let root = dir.path().join(".forge").join("sessions");
        fs::create_dir_all(&root).unwrap();
        let session = root.join("abc-123.jsonl");
        fs::write(
            &session,
            "{\"type\":\"session_start\",\"session_id\":\"abc-123\",\"title\":\"Chat\",\"model\":\"gemma4\",\"timestamp_ms\":1}\n",
        )
        .unwrap();

        let targets = scan_forge_from_root(&root, u64::MAX, 0.0);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, session);
        assert!(matches!(
            targets[0].kind,
            ScanTargetKind::Coding(ToolSource::Forge)
        ));
    }

    #[test]
    fn forge_scan_returns_empty_when_dir_missing() {
        let dir = tempdir().unwrap();
        let root = dir.path().join(".forge").join("sessions");
        let targets = scan_forge_from_root(&root, u64::MAX, 0.0);
        assert!(targets.is_empty());
    }

    #[test]
    fn forge_fill_pct_none_without_compaction() {
        let content = "{\"type\":\"session_start\",\"session_id\":\"abc\"}\n";
        assert!(forge_fill_pct(content).is_none());
    }

    #[test]
    fn forge_fill_pct_valid_compaction() {
        let content = "{\"type\":\"compaction\",\"used_tokens\":40000,\"max_tokens\":60000}\n";
        let pct = forge_fill_pct(content).unwrap();
        assert!((pct - 66.666_666).abs() < 0.001);
    }

    #[test]
    fn forge_fill_pct_keeps_largest_ratio() {
        let content = format!(
            "{}\n{}",
            "{\"type\":\"compaction\",\"used_tokens\":10000,\"max_tokens\":100000}",
            "{\"type\":\"compaction\",\"used_tokens\":90000,\"max_tokens\":100000}"
        );
        let pct = forge_fill_pct(&content).unwrap();
        assert!((pct - 90.0).abs() < 0.001);
    }

    #[test]
    fn forge_fill_pct_ignores_zero_max_and_negative_used() {
        let content = format!(
            "{}\n{}\n{}\n",
            "{\"type\":\"compaction\",\"used_tokens\":5000,\"max_tokens\":0}",
            "{\"type\":\"compaction\",\"used_tokens\":-100,\"max_tokens\":100000}",
            "{\"type\":\"compaction\",\"used_tokens\":30000,\"max_tokens\":100000}"
        );
        let pct = forge_fill_pct(&content).unwrap();
        assert!((pct - 30.0).abs() < 0.001);
    }

    #[test]
    fn forge_fill_pct_clamps_used_over_max_to_100() {
        // A compaction where used exceeds max is a real observation that
        // overflows the window; it clamps to 100 rather than exceeding it.
        let content = "{\"type\":\"compaction\",\"used_tokens\":120000,\"max_tokens\":100000}\n";
        let pct = forge_fill_pct(content).unwrap();
        assert!((pct - 100.0).abs() < 0.001);
    }

    #[test]
    fn forge_fill_pct_malformed_only_returns_none() {
        // A file with only malformed rows has no usable measurement.
        let content = "not-json\n{\"broken\":true}\n{\"type\":\"compaction\"}\n";
        assert!(forge_fill_pct(content).is_none());
    }

    #[test]
    fn forge_fill_pct_invalid_only_compaction_returns_none() {
        // Compaction rows that are all invalid (zero max) yield no measurement.
        let content = "{\"type\":\"compaction\",\"used_tokens\":5000,\"max_tokens\":0}\n";
        assert!(forge_fill_pct(content).is_none());
    }

    #[test]
    fn forge_fill_pct_large_usage_delta_without_compaction_is_none() {
        // A huge cumulative usage delta is NOT fill: without a compaction row
        // the session has no real measurement and must stay unknown.
        let content = format!(
            "{}\n{}\n",
            "{\"type\":\"usage\",\"input_tokens\":1000,\"output_tokens\":50}",
            "{\"type\":\"usage\",\"input_tokens\":500000,\"output_tokens\":2000}"
        );
        assert!(forge_fill_pct(&content).is_none());
    }

    #[test]
    fn forge_scan_gate_keeps_unknown_drops_low_fill() {
        let dir = tempdir().unwrap();
        let root = dir.path().join(".forge").join("sessions");
        fs::create_dir_all(&root).unwrap();
        // No compaction row -> unknown fill -> kept even at a high threshold.
        fs::write(root.join("unknown.jsonl"), "{\"type\":\"session_start\"}\n").unwrap();
        // Known low fill -> dropped at a 50% threshold.
        fs::write(
            root.join("low.jsonl"),
            "{\"type\":\"compaction\",\"used_tokens\":10000,\"max_tokens\":100000}\n",
        )
        .unwrap();
        // Known high fill -> kept.
        fs::write(
            root.join("high.jsonl"),
            "{\"type\":\"compaction\",\"used_tokens\":90000,\"max_tokens\":100000}\n",
        )
        .unwrap();

        let targets = scan_forge_from_root(&root, u64::MAX, 50.0);
        let names: Vec<String> = targets
            .iter()
            .map(|t| t.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"unknown.jsonl".to_string()));
        assert!(names.contains(&"high.jsonl".to_string()));
        assert!(!names.contains(&"low.jsonl".to_string()));
    }

    #[test]
    fn forge_scan_gate_keeps_malformed_only_file() {
        // A file with only malformed rows has no fill measurement, so it is
        // kept (unknown fill bypasses the gate) rather than discarded.
        let dir = tempdir().unwrap();
        let root = dir.path().join(".forge").join("sessions");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("garbage.jsonl"), "not-json\n{\"broken\":true}\n").unwrap();

        let targets = scan_forge_from_root(&root, u64::MAX, 50.0);
        assert_eq!(targets.len(), 1);
        assert!(targets[0].fill_pct.is_none());
    }
}
