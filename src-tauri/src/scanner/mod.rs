// HalluScribe - JSONL session file scanner.
// Discovers session files for Claude Code, Codex, Continue, and Forge,
// computes fill_pct from the last usage line, and filters by threshold.
// No inference dependency - pure filesystem + JSON/YAML parsing.

mod claude;
mod codex;
mod continue_scan;
mod forge;
pub mod secrets;
mod shared;
mod tests;
mod types;

pub use types::{ScanTarget, ScanTargetKind, ToolSource};

use crate::readers::ChatProvider;
use crate::settings::HalluScribeSettings;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
use std::env;

/// Scan all known JSONL locations and return sessions passing both gates.
///
/// `lookback_secs` - only sessions whose file was modified within this window.
/// `min_fill_pct`  - skip sessions whose last recorded fill_pct is below this.
pub fn scan_sessions(
    archive_dir: &Path,
    settings: &HalluScribeSettings,
    lookback_secs: u64,
    min_fill_pct: f64,
    import_only: bool,
) -> Vec<ScanTarget> {
    let mut sessions = Vec::new();
    // A guest/import-only workspace skips every host-local coding-tool scan
    // (those live under the HOST's home dir, not the guest's) and ingests only
    // this workspace's configured chat imports + its own recorded in-app chats.
    if !import_only {
        sessions.extend(claude::scan_claude(lookback_secs, min_fill_pct));
        sessions.extend(codex::scan_codex(lookback_secs, min_fill_pct));
        let continue_override = {
            let p = settings.continue_data_path.trim();
            (!p.is_empty()).then(|| std::path::Path::new(p))
        };
        sessions.extend(continue_scan::scan_continue(
            lookback_secs,
            min_fill_pct,
            continue_override,
        ));
        let forge_override = {
            let p = settings.forge_sessions_path.trim();
            (!p.is_empty()).then(|| std::path::Path::new(p))
        };
        sessions.extend(forge::scan_forge(lookback_secs, forge_override));
        sessions.extend(scan_ollama_chat(settings));
    }
    sessions.extend(scan_chat_imports(settings, lookback_secs));
    sessions.extend(scan_recorded_chat_sessions(archive_dir, lookback_secs));
    sessions
}

pub fn scan_chat_imports(settings: &HalluScribeSettings, lookback_secs: u64) -> Vec<ScanTarget> {
    let mut targets = Vec::new();
    maybe_add_import(
        &mut targets,
        &settings.chatgpt_import_path,
        lookback_secs,
        ChatProvider::ChatGPT,
        &["conversations.json"],
    );
    maybe_add_import(
        &mut targets,
        &settings.claudeai_import_path,
        lookback_secs,
        ChatProvider::ClaudeAI,
        &["conversations.json"],
    );
    maybe_add_import(
        &mut targets,
        &settings.gemini_import_path,
        lookback_secs,
        ChatProvider::Gemini,
        &[
            "Takeout/My Activity/Gemini Apps/My Activity.json",
            "My Activity.json",
        ],
    );
    targets
}

fn maybe_add_import(
    out: &mut Vec<ScanTarget>,
    configured_path: &str,
    _lookback_secs: u64,
    provider: ChatProvider,
    candidates: &[&str],
) {
    let normalized_path = configured_path.trim();
    if normalized_path.is_empty() {
        return;
    }
    let base = PathBuf::from(normalized_path);
    let Some(path) = resolve_import_path(&base, candidates).filter(|path| path.exists()) else {
        return;
    };
    let Ok(meta) = fs::metadata(&path) else {
        return;
    };
    let Ok(modified) = meta.modified() else {
        return;
    };
    // Imported chat exports are user-selected archive files rather than live session logs.
    // ZIP extraction and download flows can preserve stale mtimes, so gating on filesystem
    // modified time causes valid imports to disappear entirely.
    out.push(ScanTarget {
        path,
        kind: ScanTargetKind::Import(provider),
        fill_pct: None,
        mtime_secs: shared::mtime_secs(modified),
    });
}

fn resolve_import_path(base: &Path, candidates: &[&str]) -> Option<PathBuf> {
    if base.is_file() {
        return Some(base.to_path_buf());
    }
    candidates
        .iter()
        .map(|candidate| base.join(candidate))
        .find(|candidate| candidate.exists())
}

fn scan_recorded_chat_sessions(archive_dir: &Path, _lookback_secs: u64) -> Vec<ScanTarget> {
    let root = archive_dir.join("recorded_sessions");
    let mut targets = Vec::new();
    collect_recorded_chat_files(&root, &mut targets);
    targets
}

fn collect_recorded_chat_files(dir: &Path, out: &mut Vec<ScanTarget>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            collect_recorded_chat_files(&path, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        out.push(ScanTarget {
            path,
            kind: ScanTargetKind::Import(ChatProvider::HalluScribeGemmaChat),
            fill_pct: Some(0.0),
            mtime_secs: shared::mtime_secs(modified),
        });
    }
}

fn scan_ollama_chat(settings: &HalluScribeSettings) -> Vec<ScanTarget> {
    let Some(path) = resolve_ollama_db_path(settings) else {
        return Vec::new();
    };
    let Ok(meta) = fs::metadata(&path) else {
        return Vec::new();
    };
    let Ok(modified) = meta.modified() else {
        return Vec::new();
    };
    vec![ScanTarget {
        path,
        kind: ScanTargetKind::Import(ChatProvider::OllamaChat),
        fill_pct: None,
        mtime_secs: shared::mtime_secs(modified),
    }]
}

fn resolve_ollama_db_path(settings: &HalluScribeSettings) -> Option<PathBuf> {
    let configured = settings.ollama_chat_db_path.trim();
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        return path.exists().then_some(path);
    }
    #[cfg(target_os = "windows")]
    {
        env::var("LOCALAPPDATA")
            .ok()
            .map(|base| PathBuf::from(base).join("Ollama").join("db.sqlite"))
            .filter(|p| p.exists())
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}
