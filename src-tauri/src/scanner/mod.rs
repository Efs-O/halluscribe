// HalluScribe - JSONL session file scanner.
// Discovers session files for Claude Code, Codex, and Forge,
// computes fill_pct from the last usage line, and filters by threshold.
// No inference dependency - pure filesystem + JSON/YAML parsing.

mod claude;
mod codex;
mod forge;
mod import_discovery_tests;
pub mod import_status;
#[cfg(test)]
mod import_status_tests;
pub mod secrets;
pub(crate) mod shared;
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
/// Gemini's Takeout nesting (`Takeout/My Activity/Gemini Apps/`) no longer needs
/// to be spelled out: the bare file name is the anchor and the bounded-depth
/// search in `resolve_import_paths` walks down to it. `settings::imports` still
/// seeds the Takeout sub-path, because that is where the user unpacks it.
const GEMINI_CANDIDATES: &[&str] = &["My Activity.json"];
const GROK_CANDIDATES: &[&str] = &["prod-grok-backend.json"];

/// How far below a configured import folder the search looks.
///
/// The rule is **the deepest provider nesting, plus one folder the user names
/// themselves**. Unzipping an export always produces a wrapper folder (named
/// after the zip, so usually a UUID or a date, or renamed to something the user
/// can recognise later), and dropping that folder in as-is is the normal
/// workflow rather than an edge case. Grok is the worst case and therefore sets
/// the bound: its own layout is already four levels
/// (`ttl`/`30d`/`export_data`/`<user_id>`/file), so a wrapper around it puts the
/// export at five. Gemini's Takeout nesting is three; ChatGPT's and
/// Claude.ai's are zero.
///
/// Raising this is cheap - the walk stops at the first depth that matches, and
/// the levels in between hold one directory each - but it is not free of
/// judgement: every extra level is another chance to pick up a copy the user
/// forgot about. Shallowest-wins is what keeps that safe, not this number.
const MAX_IMPORT_DEPTH: usize = 5;

/// Hard cap on directories opened during one search, so pointing an import
/// path at something enormous (a whole Desktop) still returns promptly.
const MAX_IMPORT_DIRS_VISITED: usize = 2_000;

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
    resolve_import_paths(&PathBuf::from(trimmed), candidates)
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
    for path in resolve_import_paths(&base, candidates) {
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

/// Resolve one or more import files for a configured base path.
///
/// A base that points directly at a file is used as-is. A base directory is
/// searched for the provider's candidate file names (e.g. `conversations.json`)
/// plus any split-export siblings (`conversations-000.json` …
/// `conversations-004.json`), which large ChatGPT/Claude.ai exports are chunked
/// into. Each matching file is read independently, so every chunk contributes
/// its conversations.
///
/// The search is bounded-depth and **shallowest-wins**: subdirectories are only
/// descended into while nothing has been found, and the first depth that yields
/// any match is the answer. That is what lets every provider's export be
/// dropped in exactly as downloaded — a wrapper folder, Gemini's Takeout
/// nesting, Grok's `ttl/30d/export_data/<user_id>/` — with one rule instead of
/// a per-provider special case. Shallowest-wins is also the safety property:
/// with both `conversations.json` and `old-backup/conversations.json` present,
/// only the top-level one is imported, so a stale copy can never write over the
/// same session ids.
///
/// This is the SINGLE authority for locating chat imports: the sweep reaches it
/// through `maybe_add_import`, the raw backfill through `chat_import_sources`.
/// See docs/internal/IMPORT_PATHS_PLAN.md § 2.
fn resolve_import_paths(base: &Path, candidates: &[&str]) -> Vec<PathBuf> {
    if base.is_file() {
        return vec![base.to_path_buf()];
    }
    // Anchors are compared case-insensitively: `My Activity.json` and
    // `my activity.json` are the same export, and only Linux (which CI runs)
    // would ever tell them apart.
    let anchors: Vec<String> = candidates
        .iter()
        .filter_map(|candidate| Path::new(candidate).file_name()?.to_str())
        .map(|name| name.to_lowercase())
        .collect();
    let mut level = vec![base.to_path_buf()];
    let mut visited = 0usize;
    for _ in 0..=MAX_IMPORT_DEPTH {
        let mut found = Vec::new();
        let mut next = Vec::new();
        for dir in &level {
            if visited >= MAX_IMPORT_DIRS_VISITED {
                break;
            }
            visited += 1;
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                    // Dot-directories are tool state, never an export.
                    if !name.starts_with('.') {
                        next.push(path);
                    }
                    continue;
                }
                if matches_candidate(name, &anchors) {
                    found.push(path);
                }
            }
        }
        if !found.is_empty() {
            found.sort();
            found.dedup();
            return found;
        }
        if next.is_empty() {
            break;
        }
        level = next;
    }
    Vec::new()
}

/// True when `name` is one of the candidate export files, or a split-export
/// sibling of one: `<stem>-<suffix>.<ext>` beside the expected `<stem>.<ext>`.
/// `anchors` must already be lowercase.
fn matches_candidate(name: &str, anchors: &[String]) -> bool {
    let lower = name.to_lowercase();
    anchors.iter().any(|anchor| {
        if lower == *anchor {
            return true;
        }
        let (stem, ext) = match anchor.rsplit_once('.') {
            Some((stem, ext)) => (stem, Some(ext)),
            None => (anchor.as_str(), None),
        };
        if !lower.starts_with(&format!("{stem}-")) {
            return false;
        }
        match ext {
            Some(ext) => lower.ends_with(&format!(".{ext}")),
            None => !lower.contains('.'),
        }
    })
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
            kind: ScanTargetKind::Import(ChatProvider::HalluScribeAgentChat),
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
