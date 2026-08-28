// HalluScribe - parsed chat import readers and shared session assembly.
mod chatgpt;
mod chatgpt_content;
mod claudeai;
mod gemini;
mod grok;
#[path = "halluscribe_gemma_chat.rs"]
mod halluscribe_agent_chat;
mod ollama_chat;
mod project_label;
#[cfg(test)]
mod raw_slice_tests;
mod raw_slices;

pub use raw_slices::{is_multi_session_provider, raw_slices_for_source};

use crate::archive;
use crate::preprocessor::{self, PreprocessError};
use crate::scanner::{ScanTarget, ScanTargetKind, ToolSource};
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
            Self::HalluScribeAgentChat => "HalluScribe Agent",
            Self::OllamaChat => "Ollama Chat",
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
        }
    }

    pub fn is_coding(&self) -> bool {
        matches!(self, Self::ClaudeCode | Self::Codex | Self::Forge)
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
                    Some(format!("[{}]\n{}", message.role.label(), text))
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
                (!text.is_empty()).then(|| format!("[{}]\n{}", message.role.label(), text))
            })
            .collect()
    }
}

pub fn read_target(target: &ScanTarget) -> Result<Vec<ParsedSession>, ReaderError> {
    match &target.kind {
        ScanTargetKind::Coding(tool) => read_coding_target(target, tool),
        ScanTargetKind::Import(provider) => match provider {
            ChatProvider::ChatGPT => chatgpt::read(&target.path),
            ChatProvider::ClaudeAI => claudeai::read(&target.path),
            ChatProvider::Gemini => gemini::read(&target.path),
            ChatProvider::Grok => grok::read(&target.path),
            ChatProvider::HalluScribeAgentChat => halluscribe_agent_chat::read(&target.path),
            ChatProvider::OllamaChat => ollama_chat::read(&target.path),
            ChatProvider::ClaudeCode | ChatProvider::Codex | ChatProvider::Forge => Ok(Vec::new()),
        },
    }
}

fn read_coding_target(
    target: &ScanTarget,
    tool: &ToolSource,
) -> Result<Vec<ParsedSession>, ReaderError> {
    let preprocessed = preprocessor::preprocess_session_units(&target.path, tool)?;
    let transcript = preprocessed.render();
    let transcript = transcript.trim().to_string();
    if transcript.is_empty() {
        return Ok(Vec::new());
    }

    let provider = match tool {
        ToolSource::ClaudeCode => ChatProvider::ClaudeCode,
        ToolSource::Codex => ChatProvider::Codex,
        ToolSource::Forge => ChatProvider::Forge,
    };

    let created_at = DateTime::from_timestamp(target.mtime_secs, 0).unwrap_or_else(Utc::now);
    let fill_estimated = matches!(tool, ToolSource::Forge) || target.fill_pct.is_none();
    let fill_pct = target
        .fill_pct
        .unwrap_or_else(|| estimate_fill_pct(&transcript));
    Ok(vec![ParsedSession {
        id: archive::session_id(&target.path),
        title: String::new(),
        created_at,
        updated_at: None,
        messages: vec![ParsedMessage {
            role: MessageRole::Assistant,
            text: transcript.clone(),
            timestamp: Some(created_at),
        }],
        source_path: target.path.clone(),
        provider,
        fill_pct,
        fill_estimated,
        // Taken from the preprocessor, never from `messages` below: a coding
        // session is collapsed into one blob labelled `Assistant`, so estimating
        // from it would count the user's own prompts and every tool result as
        // model output.
        tokens: preprocessed.tokens,
        transcript_hash: stable_hash(&transcript),
        // One JSONL file == one coding session, so the whole-file copy the
        // sweep falls back to is already the correct raw for this session.
        raw_slice: None,
        preprocessed_units: Some(preprocessed.units),
    }])
}

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
