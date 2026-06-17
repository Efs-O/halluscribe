// HalluScribe - Forge VS Code extension session discovery.
// Reads JSONL session files written by the Forge extension to ~/.forge/sessions/.
// Each file is one conversation: <session-id>.jsonl

use super::shared::{collect_jsonl, cutoff, home_dir, mtime_secs};
use super::{ScanTarget, ScanTargetKind, ToolSource};
use std::path::Path;

pub(super) fn scan_forge(lookback_secs: u64, override_root: Option<&Path>) -> Vec<ScanTarget> {
    let root = if let Some(path) = override_root {
        path.to_path_buf()
    } else {
        let Some(r) = home_dir().map(|home| home.join(".forge").join("sessions")) else {
            return Vec::new();
        };
        r
    };
    scan_forge_from_root(&root, lookback_secs)
}

pub(super) fn scan_forge_from_root(root: &Path, lookback_secs: u64) -> Vec<ScanTarget> {
    if !root.is_dir() {
        return Vec::new();
    }
    let gate = cutoff(lookback_secs);
    collect_jsonl(root)
        .into_iter()
        .filter(|(mtime, _)| *mtime >= gate)
        .map(|(mtime, path)| ScanTarget {
            path,
            kind: ScanTargetKind::Coding(ToolSource::Forge),
            fill_pct: None,
            mtime_secs: mtime_secs(mtime),
        })
        .collect()
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

        let targets = scan_forge_from_root(&root, u64::MAX);
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
        let targets = scan_forge_from_root(&root, u64::MAX);
        assert!(targets.is_empty());
    }
}
