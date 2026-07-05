// HalluScribe - Piper voice discovery: locate the piper binary and scan
// installed voice models. Piper install + voices live host-global under
// `~/.halluscribe/tts/` (default_archive_dir), shared across workspaces -
// the piper install and voice packs are machine-level tools, like
// `llama_server_bin`, not per-workspace state.

use crate::app_support::default_archive_dir;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

/// One installed Piper voice model, as returned to the frontend voice picker.
#[derive(Debug, Clone, Serialize)]
pub struct VoiceInfo {
    pub name: String,
    pub sample_rate: u32,
}

/// Absolute path to the host-global TTS root (`<default_archive_dir>/tts`).
pub(super) fn tts_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(default_archive_dir(app)?.join("tts"))
}

/// Scan `<tts>/voices/*.onnx` for voices with a sibling `<name>.onnx.json`,
/// reading `audio.sample_rate` (default 22050) from the JSON. A malformed or
/// missing voice JSON skips only that voice - it never panics or drops the
/// whole list.
pub fn scan_voices(app: &tauri::AppHandle) -> Vec<VoiceInfo> {
    match tts_root(app) {
        Ok(root) => scan_voices_in(&root.join("voices")),
        Err(_) => Vec::new(),
    }
}

fn scan_voices_in(voices_dir: &Path) -> Vec<VoiceInfo> {
    let Ok(entries) = fs::read_dir(voices_dir) else {
        return Vec::new();
    };

    let mut found: Vec<VoiceInfo> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let model_path = entry.path();
            if model_path.extension().and_then(|ext| ext.to_str()) != Some("onnx") {
                return None;
            }
            let name = model_path.file_stem()?.to_str()?.to_string();
            let json_path = voices_dir.join(format!("{name}.onnx.json"));
            let sample_rate = read_sample_rate(&json_path)?;
            Some(VoiceInfo { name, sample_rate })
        })
        .collect();

    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// Read `audio.sample_rate` from a voice's `.onnx.json`. Returns `None` when
/// the file is missing or not valid JSON (caller skips that voice); defaults
/// to 22050 when the file parses but the field is absent.
fn read_sample_rate(json_path: &Path) -> Option<u32> {
    let contents = fs::read_to_string(json_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&contents).ok()?;
    Some(
        value
            .get("audio")
            .and_then(|audio| audio.get("sample_rate"))
            .and_then(serde_json::Value::as_u64)
            .map(|rate| rate as u32)
            .unwrap_or(22_050),
    )
}

/// Resolve the piper binary: `explicit` (from settings `tts_piper_bin`) wins
/// when it points at an existing file; otherwise recursively search
/// `<tts>/piper/` for the platform binary. Never assumes PATH (port of
/// Gemma4kids' `findPiperBinary`).
pub fn find_piper_bin(app: &tauri::AppHandle, explicit: &str) -> Option<PathBuf> {
    if !explicit.is_empty() {
        let candidate = PathBuf::from(explicit);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let root = tts_root(app).ok()?;
    recursive_find_binary(&root.join("piper"), piper_binary_name(), 5)
}

#[cfg(target_os = "windows")]
fn piper_binary_name() -> &'static str {
    "piper.exe"
}

#[cfg(not(target_os = "windows"))]
fn piper_binary_name() -> &'static str {
    "piper"
}

/// Depth-limited search for an exact filename match: checks the directory
/// itself first, then recurses into subdirectories (port of Gemma4kids'
/// `recursiveFindBinary`).
fn recursive_find_binary(start_dir: &Path, binary_name: &str, depth: u32) -> Option<PathBuf> {
    if !start_dir.is_dir() {
        return None;
    }
    let direct = start_dir.join(binary_name);
    if direct.is_file() {
        return Some(direct);
    }
    if depth == 0 {
        return None;
    }
    let entries = fs::read_dir(start_dir).ok()?;
    for entry in entries.filter_map(|e| e.ok()) {
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }
        if let Some(found) = recursive_find_binary(&entry.path(), binary_name, depth - 1) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_voice(dir: &Path, name: &str, json: &str) {
        fs::write(dir.join(format!("{name}.onnx")), b"fake-onnx").unwrap();
        fs::write(dir.join(format!("{name}.onnx.json")), json).unwrap();
    }

    #[test]
    fn scan_voices_in_reads_sample_rate_and_sorts_by_name() {
        let dir = tempdir().unwrap();
        write_voice(
            dir.path(),
            "en_US-amy-medium",
            r#"{"audio":{"sample_rate":24000}}"#,
        );
        write_voice(
            dir.path(),
            "el_GR-joy-medium",
            r#"{"audio":{"sample_rate":22050}}"#,
        );

        let voices = scan_voices_in(dir.path());
        assert_eq!(voices.len(), 2);
        assert_eq!(voices[0].name, "el_GR-joy-medium");
        assert_eq!(voices[0].sample_rate, 22050);
        assert_eq!(voices[1].name, "en_US-amy-medium");
        assert_eq!(voices[1].sample_rate, 24000);
    }

    #[test]
    fn scan_voices_in_defaults_sample_rate_when_field_absent() {
        let dir = tempdir().unwrap();
        write_voice(dir.path(), "no_rate-voice", r#"{"audio":{}}"#);

        let voices = scan_voices_in(dir.path());
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0].sample_rate, 22_050);
    }

    #[test]
    fn scan_voices_in_skips_voice_without_sibling_json() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("orphan.onnx"), b"fake-onnx").unwrap();

        assert!(scan_voices_in(dir.path()).is_empty());
    }

    #[test]
    fn scan_voices_in_skips_malformed_json_without_panicking() {
        let dir = tempdir().unwrap();
        write_voice(dir.path(), "broken", "{ not valid json");

        assert!(scan_voices_in(dir.path()).is_empty());
    }

    #[test]
    fn scan_voices_in_returns_empty_for_missing_dir() {
        let dir = tempdir().unwrap();
        assert!(scan_voices_in(&dir.path().join("does-not-exist")).is_empty());
    }

    #[test]
    fn recursive_find_binary_finds_nested_file() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        let target = nested.join("piper.exe");
        fs::write(&target, b"fake-binary").unwrap();

        assert_eq!(
            recursive_find_binary(dir.path(), "piper.exe", 5),
            Some(target)
        );
    }

    #[test]
    fn recursive_find_binary_returns_none_when_absent() {
        let dir = tempdir().unwrap();
        assert_eq!(recursive_find_binary(dir.path(), "piper.exe", 5), None);
    }
}
