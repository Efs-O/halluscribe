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

/// Expected file names inside a configured import folder, per provider. One
/// list, used by both the sweep and the raw backfill, so the two can never
/// disagree about which files an import folder holds.
const CHATGPT_CANDIDATES: &[&str] = &["conversations.json"];
const CLAUDEAI_CANDIDATES: &[&str] = &["conversations.json"];
const GEMINI_CANDIDATES: &[&str] = &[
    "Takeout/My Activity/Gemini Apps/My Activity.json",
    "My Activity.json",
];
const GROK_CANDIDATES: &[&str] = &["prod-grok-backend.json"];

/// Grok's export unpacks as `ttl/30d/export_data/<user_id>/prod-grok-backend.json`.
/// `<user_id>` is a per-account UUID, so it cannot be a literal candidate the way
/// Gemini's Takeout path can - the directory is enumerated instead (see
/// `grok_export_roots`), which lets the folder be dropped in exactly as downloaded.
const GROK_EXPORT_DATA_SUBPATH: [&str; 3] = ["ttl", "30d", "export_data"];

pub fn scan_chat_imports(settings: &HalluScribeSettings, lookback_secs: u64) -> Vec<ScanTarget> {
    let mut targets = Vec::new();
    maybe_add_import(
        &mut targets,
        &settings.chatgpt_import_path,
        lookback_secs,
        ChatProvider::ChatGPT,
        CHATGPT_CANDIDATES,
    );
    maybe_add_import(
        &mut targets,
        &settings.claudeai_import_path,
        lookback_secs,
        ChatProvider::ClaudeAI,
        CLAUDEAI_CANDIDATES,
    );
    maybe_add_import(
        &mut targets,
        &settings.gemini_import_path,
        lookback_secs,
        ChatProvider::Gemini,
        GEMINI_CANDIDATES,
    );
    maybe_add_import(
        &mut targets,
        &settings.grok_import_path,
        lookback_secs,
        ChatProvider::Grok,
        GROK_CANDIDATES,
    );
    targets
}

/// Every source file currently reachable for a user-owned chat-import provider,
/// resolved from settings alone.
///
/// This is the SINGLE authority for locating chat imports. The sweep reaches it
/// through `scan_chat_imports`/`scan_ollama_chat`; the raw backfill calls it
/// directly instead of trusting the absolute path recorded when the session was
/// first archived, which goes stale the moment the user moves the folder or
/// changes the setting. Providers whose files belong to a coding tool are not
/// handled here - those are located by their recorded path and nothing else.
/// See docs/internal/IMPORT_PATHS_PLAN.md § 2.
pub fn chat_import_sources(settings: &HalluScribeSettings, provider_key: &str) -> Vec<PathBuf> {
    let (configured, candidates) = match provider_key {
        "chatgpt" => (&settings.chatgpt_import_path, CHATGPT_CANDIDATES),
        "claude_ai" => (&settings.claudeai_import_path, CLAUDEAI_CANDIDATES),
        "gemini" => (&settings.gemini_import_path, GEMINI_CANDIDATES),
        "grok" => (&settings.grok_import_path, GROK_CANDIDATES),
        // The Ollama chat DB is a single machine-local file with its own
        // resolver (optional override, else the OS default location).
        "ollama_chat" => return resolve_ollama_db_path(settings).into_iter().collect(),
        _ => return Vec::new(),
    };
    let trimmed = configured.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    resolve_provider_paths(&PathBuf::from(trimmed), provider_key, candidates)
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
    for path in resolve_provider_paths(&base, provider.provider_key(), candidates) {
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        // Imported chat exports are user-selected archive files rather than live session logs.
        // ZIP extraction and download flows can preserve stale mtimes, so gating on filesystem
        // modified time causes valid imports to disappear entirely.
        out.push(ScanTarget {
            path,
            kind: ScanTargetKind::Import(provider.clone()),
            fill_pct: None,
            mtime_secs: shared::mtime_secs(modified),
        });
    }
}

/// Resolve a provider's import files, applying any layout that provider alone
/// has on top of the shared candidate search.
///
/// Both authorities go through this - the sweep via `maybe_add_import` and the
/// raw backfill via `chat_import_sources` - so a provider-specific layout can
/// never be honoured by one and missed by the other. See
/// docs/internal/IMPORT_PATHS_PLAN.md § 2.
fn resolve_provider_paths(base: &Path, provider_key: &str, candidates: &[&str]) -> Vec<PathBuf> {
    let mut found = resolve_import_paths(base, candidates);
    if provider_key == "grok" {
        for root in grok_export_roots(base) {
            found.extend(resolve_import_paths(&root, candidates));
        }
        found.sort();
        found.dedup();
    }
    found
}

/// Every `ttl/30d/export_data/<user_id>` directory under `base`, so a Grok
/// export can be dropped in as downloaded. Returns nothing when that nesting is
/// absent, which is the case when the user points straight at the export folder.
fn grok_export_roots(base: &Path) -> Vec<PathBuf> {
    let export_data = GROK_EXPORT_DATA_SUBPATH
        .iter()
        .fold(base.to_path_buf(), |path, part| path.join(part));
    let Ok(entries) = fs::read_dir(&export_data) else {
        return Vec::new();
    };
    let mut roots: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    roots.sort();
    roots
}

/// Resolve one or more import files for a configured base path.
///
/// A base that points directly at a file is used as-is. A base directory is
/// searched for each expected candidate (e.g. `conversations.json`) plus any
/// split-export siblings (e.g. `conversations-000.json` … `conversations-004.json`),
/// which large ChatGPT/Claude.ai exports are chunked into. Each matching file is
/// read independently, so every chunk contributes its conversations.
fn resolve_import_paths(base: &Path, candidates: &[&str]) -> Vec<PathBuf> {
    if base.is_file() {
        return vec![base.to_path_buf()];
    }
    let mut found = Vec::new();
    for candidate in candidates {
        let direct = base.join(candidate);
        if direct.exists() {
            found.push(direct);
        }
        found.extend(split_export_siblings(base, candidate));
    }
    found.sort();
    found.dedup();
    found
}

/// Find split-export chunks for `candidate` inside `base`, i.e. files named
/// `<stem>-<suffix>.<ext>` next to the expected `<stem>.<ext>` (nested candidate
/// paths are resolved relative to `base`).
fn split_export_siblings(base: &Path, candidate: &str) -> Vec<PathBuf> {
    let candidate_path = Path::new(candidate);
    let dir = match candidate_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => base.join(parent),
        _ => base.to_path_buf(),
    };
    let Some(file_name) = candidate_path.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((stem, ext)) => (stem, Some(ext)),
        None => (file_name, None),
    };
    let prefix = format!("{stem}-");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        let ext_ok = match ext {
            Some(ext) => name.ends_with(&format!(".{ext}")),
            None => !name.contains('.'),
        };
        if ext_ok {
            out.push(entry.path());
        }
    }
    out
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
