// HalluScribe - parsed chat import readers and shared session assembly.
pub mod apple_messages;
pub mod apple_messages_db;
pub mod apple_messages_raw;
pub mod apple_messages_window;
mod business;
mod chatgpt;
mod chatgpt_content;
mod claudeai;
mod gemini;
mod grok;
mod halluscribe_chat;
mod ollama_chat;
mod project_label;
#[cfg(test)]
mod raw_slice_tests;
mod raw_slices;
pub mod viber;
pub mod viber_db;
pub mod viber_raw;
pub mod whatsapp;
pub mod whatsapp_db;
pub mod whatsapp_raw;

pub use raw_slices::{is_multi_session_provider, raw_slices_for_source};

use crate::preprocessor::PreprocessError;
use crate::tokens::TokenCount;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatProvider {
    ClaudeCode,
    Codex,
    Forge,
    ChatGPT,
    ClaudeAI,
    Gemini,
    Grok,
    HalluScribeAgentChat,
    OllamaChat,
    /// iPhone "Messages" (iMessage/SMS) imported from an Apple backup. The
    /// reader lands in Phase 5; this variant exists now (Phase 2) so the model
    /// and its wiring are complete before the reader arrives.
    AppleMessages,
    /// WhatsApp (personal) imported from an Apple backup. Phase 7a.
    WhatsApp,
    /// WhatsApp Business imported from an Apple backup. Phase 7a. Shares the
    /// reader with `WhatsApp`; the variant selects the backup domain.
    WhatsAppBusiness,
    /// Viber imported from an Apple backup. Phase 7b. One app (no separate
    /// Business domain); the reader reads `Contacts.data`.
    Viber,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone)]
pub struct ParsedMessage {
    pub role: MessageRole,
    pub text: String,
    pub timestamp: Option<DateTime<Utc>>,
    /// Optional human-readable speaker label (e.g. "Me", "Client"). When set,
    /// the transcript renders `[speaker]` in place of the role label; when
    /// `None`, the role label is used exactly as before. Set by the business
    /// messaging readers (Phase 5+); every existing reader leaves it `None`.
    pub speaker: Option<String>,
}

impl ParsedMessage {
    /// The label rendered in the transcript: the speaker (business messaging)
    /// when present, otherwise the role label.
    pub fn label(&self) -> String {
        self.speaker
            .clone()
            .unwrap_or_else(|| self.role.label().to_string())
    }
}

#[derive(Debug, Clone)]
pub struct ParsedSession {
    pub id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub messages: Vec<ParsedMessage>,
    pub source_path: PathBuf,
    pub provider: ChatProvider,
    pub fill_pct: f64,
    pub fill_estimated: bool,
    /// Tokens the model generated across the whole session, and whether that
    /// came from the server or from a character estimate. Orthogonal to
    /// `fill_pct`, which is peak context occupancy rather than work produced.
    pub tokens: TokenCount,
    pub transcript_hash: String,
    /// Verbatim source for THIS session alone, set only by readers whose
    /// `source_path` holds many sessions (chat exports, the Ollama DB). The
    /// sweep preserves this instead of the whole file, so a 500-conversation
    /// export yields 500 distinct raws rather than 500 copies of the export.
    /// `None` means `source_path` already maps 1:1 to this session (every
    /// coding tool writes one file per session), where copying the file is
    /// correct.
    pub raw_slice: Option<String>,
    /// Overrides the project label the runner would otherwise derive from
    /// `provider.project_label(..)`. When `Some`, the runner archives the
    /// session under this project instead. Set by the business messaging
    /// readers (Phase 5+); every existing reader leaves it `None`.
    pub project_override: Option<String>,
    /// Coding preprocessors retain these whole-turn fragments before rendering
    /// the transcript. Imports use their already-structured messages instead.
    preprocessed_units: Option<Vec<String>>,
}

#[derive(Debug)]
pub enum ReaderError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Preprocess(PreprocessError),
    Database(String),
}

impl std::fmt::Display for ReaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Json(error) => write!(f, "JSON error: {error}"),
            Self::Preprocess(error) => write!(f, "preprocess error: {error}"),
            Self::Database(msg) => write!(f, "database error: {msg}"),
        }
    }
}

impl From<std::io::Error> for ReaderError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ReaderError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<PreprocessError> for ReaderError {
    fn from(error: PreprocessError) -> Self {
        Self::Preprocess(error)
    }
}

impl ChatProvider {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::Forge => "Forge",
            Self::ChatGPT => "ChatGPT",
            Self::ClaudeAI => "Claude.ai",
            Self::Gemini => "Gemini",
            Self::Grok => "Grok",
            Self::HalluScribeAgentChat => "HalluScribe Chat",
            Self::OllamaChat => "Ollama Chat",
            Self::AppleMessages => "Messages",
            Self::WhatsApp => "WhatsApp",
            Self::WhatsAppBusiness => "WhatsApp Business",
            Self::Viber => "Viber",
        }
    }

    /// The project a session belongs to. Every coding tool stores sessions
    /// under a different layout, so each resolver recovers this differently and
    /// falls back to the tool's display name rather than emitting a label it
    /// cannot stand behind — see `readers::project_label`. Two of them read the
    /// session's header line, which is why this takes a path and not just the
    /// provider; it is called once per session at write time.
    pub fn project_label(&self, source_path: &Path) -> String {
        match self {
            Self::ClaudeCode => project_label::claude_code(source_path)
                .unwrap_or_else(|| self.display_name().to_string()),
            Self::Codex => {
                project_label::codex(source_path).unwrap_or_else(|| self.display_name().to_string())
            }
            Self::Forge => {
                project_label::forge(source_path).unwrap_or_else(|| self.display_name().to_string())
            }
            Self::ChatGPT => "ChatGPT".to_string(),
            Self::ClaudeAI => "Claude".to_string(),
            Self::Gemini => "Gemini Apps".to_string(),
            Self::Grok => "Grok".to_string(),
            Self::HalluScribeAgentChat => "HalluScribe".to_string(),
            Self::OllamaChat => "Ollama".to_string(),
            Self::AppleMessages => "Messages".to_string(),
            Self::WhatsApp => "WhatsApp".to_string(),
            Self::WhatsAppBusiness => "WhatsApp Business".to_string(),
            Self::Viber => "Viber".to_string(),
        }
    }

    pub fn is_coding(&self) -> bool {
        matches!(self, Self::ClaudeCode | Self::Codex | Self::Forge)
    }

    /// The business-messaging providers (Apple Messages, WhatsApp, WhatsApp
    /// Business, Viber). These are the only ones that get the business summary
    /// prompt (Phase 8); every other provider keeps its existing prompt.
    pub fn is_business(&self) -> bool {
        matches!(
            self,
            Self::AppleMessages | Self::WhatsApp | Self::WhatsAppBusiness | Self::Viber
        )
    }

    pub fn provider_key(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::Forge => "forge",
            Self::ChatGPT => "chatgpt",
            Self::ClaudeAI => "claude_ai",
            Self::Gemini => "gemini",
            Self::Grok => "grok",
            Self::HalluScribeAgentChat => "halluscribe_agent_chat",
            Self::OllamaChat => "ollama_chat",
            Self::AppleMessages => "apple_messages",
            Self::WhatsApp => "whatsapp",
            Self::WhatsAppBusiness => "whatsapp_business",
            Self::Viber => "viber",
        }
    }

    /// The inverse of `provider_key`: recover a provider from its machine key
    /// (the `profile_sources` / archive-index `provider` string). `None` for a
    /// key no provider owns.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "claude_code" => Some(Self::ClaudeCode),
            "codex" => Some(Self::Codex),
            "forge" => Some(Self::Forge),
            "chatgpt" => Some(Self::ChatGPT),
            "claude_ai" => Some(Self::ClaudeAI),
            "gemini" => Some(Self::Gemini),
            "grok" => Some(Self::Grok),
            "halluscribe_agent_chat" => Some(Self::HalluScribeAgentChat),
            "ollama_chat" => Some(Self::OllamaChat),
            "apple_messages" => Some(Self::AppleMessages),
            "whatsapp" => Some(Self::WhatsApp),
            "whatsapp_business" => Some(Self::WhatsAppBusiness),
            "viber" => Some(Self::Viber),
            _ => None,
        }
    }
}

impl MessageRole {
    fn label(&self) -> &'static str {
        match self {
            Self::User => "User",
            Self::Assistant => "Assistant",
            Self::System => "System",
        }
    }
}

impl ParsedSession {
    /// Attach the verbatim slice of the multi-session source that belongs to
    /// this session alone. See [`ParsedSession::raw_slice`].
    pub(super) fn with_raw_slice(mut self, slice: String) -> Self {
        self.raw_slice = Some(slice);
        self
    }

    pub fn transcript(&self) -> String {
        if let Some(units) = &self.preprocessed_units {
            return units.join("\n");
        }
        self.messages
            .iter()
            .filter_map(|message| {
                let text = message.text.trim();
                if text.is_empty() {
                    None
                } else {
                    Some(format!("[{}]\n{}", message.label(), text))
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Whole chronological units used by large-session chunk planning. These
    /// are produced at parse time, never recovered from formatted transcript.
    pub fn transcript_units(&self) -> Vec<String> {
        if let Some(units) = &self.preprocessed_units {
            return units.clone();
        }
        self.messages
            .iter()
            .filter_map(|message| {
                let text = message.text.trim();
                (!text.is_empty()).then(|| format!("[{}]\n{}", message.label(), text))
            })
            .collect()
    }
}

/// Read the sessions for one `ScanTarget`: coding tools through the
/// preprocessor, chat exports through their own readers, and the
/// Apple-backup readers (Messages, WhatsApp, WhatsApp Business, Viber) through
/// their readers. `default_cc` is the business-messaging default country code
/// for phone normalization (D8); it is `None` outside business ingestion.
/// The dispatch lives in `business.rs` so this module stays under the LOC
/// ceiling as the business providers grow.
pub use business::read_target;

/// Chat exports carry no usage data, so this is the only number available for
/// them. It is also blind to thinking tokens: providers strip reasoning traces
/// before export, so a thinking-model session is undercounted several-fold and
/// the estimated flag is what keeps that from being mistaken for a real total.
fn estimate_message_tokens(messages: &[ParsedMessage]) -> TokenCount {
    let chars = messages
        .iter()
        .filter(|message| message.role == MessageRole::Assistant)
        .map(|message| message.text.len())
        .sum();
    TokenCount::estimated(crate::tokens::estimate_from_chars(chars))
}

fn estimate_fill_pct(transcript: &str) -> f64 {
    ((transcript.len() as f64 / 600_000.0) * 100.0).clamp(0.0, 100.0)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_session(
    id: String,
    title: String,
    created_at: DateTime<Utc>,
    updated_at: Option<DateTime<Utc>>,
    source_path: PathBuf,
    provider: ChatProvider,
    fill_pct: f64,
    fill_estimated: bool,
    messages: Vec<ParsedMessage>,
) -> Option<ParsedSession> {
    let transcript = messages
        .iter()
        .filter_map(|message| {
            let text = message.text.trim();
            (!text.is_empty()).then_some(text)
        })
        .collect::<Vec<_>>()
        .join("\n");
    if transcript.trim().is_empty() {
        return None;
    }

    Some(ParsedSession {
        id,
        title,
        created_at,
        updated_at,
        source_path,
        provider,
        fill_pct,
        fill_estimated,
        tokens: estimate_message_tokens(&messages),
        transcript_hash: stable_hash(&transcript),
        messages,
        raw_slice: None,
        project_override: None,
        preprocessed_units: None,
    })
}

pub(super) fn parse_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

pub(super) fn file_stem_or_hash(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| stable_hash(&path.to_string_lossy()))
}

pub(super) fn read_json(path: &Path) -> Result<String, ReaderError> {
    Ok(fs::read_to_string(path)?)
}

pub(super) fn stable_hash(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{ScanTarget, ScanTargetKind, ToolSource};
    use std::fs;
    use tempfile::tempdir;

    fn forge_target(root: &Path, file: &str, fill_pct: Option<f64>) -> ScanTarget {
        let path = root.join(file);
        fs::write(
            &path,
            "{\"role\":\"user\",\"content\":\"Fix the build\",\"timestamp_ms\":1}\n",
        )
        .unwrap();
        ScanTarget {
            path,
            kind: ScanTargetKind::Coding(ToolSource::Forge),
            fill_pct,
            mtime_secs: 0,
        }
    }

    #[test]
    fn forge_measured_fill_is_not_estimated() {
        let dir = tempdir().unwrap();
        let target = forge_target(dir.path(), "measured.jsonl", Some(66.0));
        let sessions = read_target(&target, None).unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(!sessions[0].fill_estimated);
        assert!((sessions[0].fill_pct - 66.0).abs() < 0.001);
    }

    #[test]
    fn forge_unknown_fill_is_estimated() {
        let dir = tempdir().unwrap();
        let target = forge_target(dir.path(), "unknown.jsonl", None);
        let sessions = read_target(&target, None).unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].fill_estimated);
    }

    #[test]
    fn coding_transcript_is_unchanged_golden() {
        // The coding path renders from preprocessed_units (a branch
        // `transcript()` did not change in the business-messaging work). This
        // golden pins that a coding session's transcript is byte-for-byte what
        // it was before.
        let dir = tempdir().expect("tempdir");
        let target = forge_target(dir.path(), "session.jsonl", None);
        let session = read_target(&target, None)
            .expect("read")
            .pop()
            .expect("session");
        assert_eq!(session.transcript(), "[User]\nFix the build");
    }
}

#[cfg(test)]
#[path = "reader_model_tests.rs"]
mod reader_model_tests;
