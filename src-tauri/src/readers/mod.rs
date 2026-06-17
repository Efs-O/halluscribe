// HalluScribe - parsed chat import readers and shared session assembly.
mod chatgpt;
mod chatgpt_content;
mod claudeai;
mod gemini;
mod halluscribe_gemma_chat;
mod ollama_chat;

use crate::archive;
use crate::preprocessor::{self, PreprocessError};
use crate::scanner::{ScanTarget, ScanTargetKind, ToolSource};
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
    Continue,
    Forge,
    ChatGPT,
    ClaudeAI,
    Gemini,
    HalluScribeGemmaChat,
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
    pub transcript_hash: String,
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
            Self::Continue => "Continue",
            Self::Forge => "Forge",
            Self::ChatGPT => "ChatGPT",
            Self::ClaudeAI => "Claude.ai",
            Self::Gemini => "Gemini",
            Self::HalluScribeGemmaChat => "Gemma 4",
            Self::OllamaChat => "Ollama Chat",
        }
    }

    pub fn project_label(&self, source_path: &Path) -> String {
        match self {
            Self::ClaudeCode | Self::Codex | Self::Continue => project_from_parent(source_path),
            Self::Forge => "Forge".to_string(),
            Self::ChatGPT => "ChatGPT".to_string(),
            Self::ClaudeAI => "Claude".to_string(),
            Self::Gemini => "Gemini Apps".to_string(),
            Self::HalluScribeGemmaChat => "HalluScribe".to_string(),
            Self::OllamaChat => "Ollama".to_string(),
        }
    }

    pub fn is_coding(&self) -> bool {
        matches!(
            self,
            Self::ClaudeCode | Self::Codex | Self::Continue | Self::Forge
        )
    }

    pub fn provider_key(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::Continue => "continue",
            Self::Forge => "forge",
            Self::ChatGPT => "chatgpt",
            Self::ClaudeAI => "claude_ai",
            Self::Gemini => "gemini",
            Self::HalluScribeGemmaChat => "halluscribe_gemma_chat",
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
    pub fn transcript(&self) -> String {
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
}

pub fn read_target(target: &ScanTarget) -> Result<Vec<ParsedSession>, ReaderError> {
    match &target.kind {
        ScanTargetKind::Coding(tool) => read_coding_target(target, tool),
        ScanTargetKind::Import(provider) => match provider {
            ChatProvider::ChatGPT => chatgpt::read(&target.path),
            ChatProvider::ClaudeAI => claudeai::read(&target.path),
            ChatProvider::Gemini => gemini::read(&target.path),
            ChatProvider::HalluScribeGemmaChat => halluscribe_gemma_chat::read(&target.path),
            ChatProvider::OllamaChat => ollama_chat::read(&target.path),
            ChatProvider::ClaudeCode
            | ChatProvider::Codex
            | ChatProvider::Continue
            | ChatProvider::Forge => Ok(Vec::new()),
        },
    }
}

fn read_coding_target(
    target: &ScanTarget,
    tool: &ToolSource,
) -> Result<Vec<ParsedSession>, ReaderError> {
    let transcript = preprocessor::preprocess_session(&target.path, tool)?;
    let transcript = transcript.trim().to_string();
    if transcript.is_empty() {
        return Ok(Vec::new());
    }

    let provider = match tool {
        ToolSource::ClaudeCode => ChatProvider::ClaudeCode,
        ToolSource::Codex => ChatProvider::Codex,
        ToolSource::Continue => ChatProvider::Continue,
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
        transcript_hash: stable_hash(&transcript),
    }])
}

fn project_from_parent(source_path: &Path) -> String {
    source_path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string()
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
        transcript_hash: stable_hash(&transcript),
        messages,
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
