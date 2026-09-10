// HalluScribe - manifest of raws captured by the startup capture pass
// (`archive::capture`). Distinct from `index.json`: this tracks every coding
// source file the capture pass has ever preserved a raw copy of, keyed by
// session id, so a re-run can skip files whose size+mtime haven't changed
// without hashing anything. Sessions that later get summarised by a sweep
// stay in this manifest too - it is never pruned, only appended/updated.

use super::ArchiveError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Archive-relative path to the manifest file.
const CAPTURED_MANIFEST_REL_PATH: &str = "raw/captured.json";

/// One captured session's skip-fast-path fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapturedRecord {
    pub source_path: String,
    pub size: u64,
    pub mtime_secs: i64,
    pub captured_at: String,
}

pub type CapturedManifest = HashMap<String, CapturedRecord>;

/// Load the manifest, or an empty map if it doesn't exist yet / fails to parse
/// (a corrupt manifest degrades to "capture everything again", never a hard
/// error - the skip fast-path is a pure optimization).
pub fn load_captured(archive_dir: &Path) -> CapturedManifest {
    let path = archive_dir.join(CAPTURED_MANIFEST_REL_PATH);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// Persist the manifest atomically so a crash or a concurrent sweep never
/// observes a half-written `captured.json`.
pub fn save_captured(archive_dir: &Path, manifest: &CapturedManifest) -> Result<(), ArchiveError> {
    let path = archive_dir.join(CAPTURED_MANIFEST_REL_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::atomic_file::write_atomic(&path, serde_json::to_string_pretty(manifest)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "halluscribe_captured_manifest_test_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_manifest_loads_as_empty() {
        let dir = tmp_dir("missing");
        assert!(load_captured(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_load_round_trips_and_leaves_no_tmp_file() {
        let dir = tmp_dir("round_trip");
        let mut manifest = CapturedManifest::new();
        manifest.insert(
            "abc-123".to_string(),
            CapturedRecord {
                source_path: "C:/fake/session.jsonl".to_string(),
                size: 42,
                mtime_secs: 1_700_000_000,
                captured_at: "2026-07-15T00:00:00Z".to_string(),
            },
        );
        save_captured(&dir, &manifest).unwrap();

        let loaded = load_captured(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["abc-123"].size, 42);
        assert!(!dir.join("raw/.captured.json.tmp").exists());
        assert!(dir.join("raw/captured.json").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_manifest_degrades_to_empty_instead_of_erroring() {
        let dir = tmp_dir("corrupt");
        std::fs::create_dir_all(dir.join("raw")).unwrap();
        std::fs::write(dir.join(CAPTURED_MANIFEST_REL_PATH), b"not json").unwrap();
        assert!(load_captured(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
