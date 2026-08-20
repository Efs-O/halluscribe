// HalluScribe - "is there actually an export in there?" for the chat-import
// paths (GROK_IMPORT_PLAN § 12.4.4).
//
// The failure this exists for: an import path can be set, point at a real
// folder, and still yield nothing - the export was never unpacked, was unpacked
// somewhere else, or sits deeper than the discovery walk goes. Before this, the
// only symptom was a sweep that imported zero sessions and said nothing about
// why. That cost a whole sweep cycle to notice and was invisible until someone
// went looking.
//
// The status is derived from `chat_import_sources` and nothing else, so what
// Settings reports is by construction what the sweep will find - a second
// "does this look right" implementation could disagree with the real one, which
// is the failure mode a diagnostic must never have.

use super::chat_import_sources;
use crate::settings::HalluScribeSettings;
use serde::Serialize;
use std::path::Path;

/// The four user-owned chat providers, as `(provider_key, label)`. Ollama and
/// the coding tools are excluded on purpose: their files are located by their
/// own resolvers rather than by unpacking an export into a folder.
pub const IMPORT_STATUS_PROVIDERS: [(&str, &str); 4] = [
    ("chatgpt", "ChatGPT"),
    ("claude_ai", "Claude.ai"),
    ("gemini", "Gemini"),
    ("grok", "Grok"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportPathState {
    /// No path configured. Not a problem - that provider is simply unused.
    Unset,
    /// A path is set but there is no such folder: renamed, deleted, or on a
    /// drive that is not mounted right now.
    MissingFolder,
    /// The folder is there and was searched, and holds no export this app
    /// recognises. THIS is the case § 12.4.4 exists for.
    NoExportFound,
    /// One or more export files found. `file_count` > 1 is normal for a split
    /// ChatGPT export.
    Found,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportPathStatus {
    pub provider: String,
    pub label: String,
    pub path: String,
    pub state: ImportPathState,
    pub file_count: usize,
    pub total_bytes: u64,
    /// The found files, newest-looking first is not promised - this is for
    /// showing the user WHICH file answered, since "found" on the wrong copy
    /// is its own class of confusion.
    pub files: Vec<String>,
}

/// Status of one provider's import path.
pub fn import_status(settings: &HalluScribeSettings, provider_key: &str) -> ImportPathStatus {
    let label = IMPORT_STATUS_PROVIDERS
        .iter()
        .find(|(key, _)| *key == provider_key)
        .map(|(_, label)| *label)
        .unwrap_or(provider_key);
    let configured = match provider_key {
        "chatgpt" => &settings.chatgpt_import_path,
        "claude_ai" => &settings.claudeai_import_path,
        "gemini" => &settings.gemini_import_path,
        "grok" => &settings.grok_import_path,
        _ => "",
    };
    let path = configured.trim().to_string();

    let mut status = ImportPathStatus {
        provider: provider_key.to_string(),
        label: label.to_string(),
        path: path.clone(),
        state: ImportPathState::Unset,
        file_count: 0,
        total_bytes: 0,
        files: Vec::new(),
    };
    if path.is_empty() {
        return status;
    }
    if !Path::new(&path).exists() {
        status.state = ImportPathState::MissingFolder;
        return status;
    }

    let found = chat_import_sources(settings, provider_key);
    status.total_bytes = found
        .iter()
        .filter_map(|file| std::fs::metadata(file).ok())
        .map(|meta| meta.len())
        .sum();
    status.file_count = found.len();
    status.files = found
        .iter()
        .map(|file| file.display().to_string())
        .collect();
    status.state = if found.is_empty() {
        ImportPathState::NoExportFound
    } else {
        ImportPathState::Found
    };
    status
}

/// Status of every chat-import path, in the order Settings shows the fields.
pub fn all_import_statuses(settings: &HalluScribeSettings) -> Vec<ImportPathStatus> {
    IMPORT_STATUS_PROVIDERS
        .iter()
        .map(|(key, _)| import_status(settings, key))
        .collect()
}
