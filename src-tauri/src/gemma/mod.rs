// HalluScribe - Gemma inference backends for structured session summaries.
// Keeps the public inference API stable while backend-specific logic lives in
// focused sibling modules.

mod llamacpp;
mod ollama;
mod schema;
mod session;
mod tool_session;

#[cfg(test)]
#[path = "word_budget_ab_tests.rs"]
mod word_budget_ab_tests;

pub use session::{start_sweep_session, SweepSession};
pub use tool_session::{start_tool_session, ToolSession};

use crate::readers::ChatProvider;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

pub(crate) const TEMPERATURE: f64 = 0.2;
pub(crate) const STARTUP_TIMEOUT_SECS: u32 = 60;
pub(crate) const INFER_TIMEOUT: Duration = Duration::from_secs(600);
pub(crate) const SUMMARY_WORD_FRACTION: f64 = 0.18;
pub(crate) const MIN_SUMMARY_WORDS: usize = 300;
pub(crate) const MAX_SUMMARY_WORDS: usize = 900;

#[derive(Debug, Clone)]
pub enum InferenceBackend {
    LlamaCpp {
        bin: PathBuf,
        model: PathBuf,
        port: u16,
        gpu_layers: i32,
    },
    Ollama {
        host: String,
        port: u16,
        model: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionType {
    Debugging,
    Building,
    Refactoring,
    #[default]
    Exploration,
}

#[derive(Debug, Clone, Serialize)]
pub struct GemmaOutput {
    pub title: String,
    pub summary: String,
    pub session_type: SessionType,
    pub error_tags: Vec<String>,
    pub topic_tags: Vec<String>,
    /// Verdicts, comparison tables, benchmark results and decision matrices
    /// copied verbatim out of the transcript. Rendered as its own `## Highlights`
    /// section and deliberately **excluded from the prose word budget**, so this
    /// content never competes with Goal / Files Changed for the same words.
    /// A measured A/B (`word_budget_ab_tests.rs`) showed the model ignores the
    /// word target, so a separate field — not a bigger budget — is what keeps
    /// this content in the archive and therefore findable by `search_sessions`.
    pub verbatim_highlights: Vec<String>,
}

#[derive(Debug)]
pub enum GemmaError {
    BinaryNotFound(String),
    ModelNotFound(String),
    ServerStartTimeout,
    ServerExitedEarly,
    Http(String),
    Conflict(String),
    /// Two MTP drafters next to the model both claim it. Distinct from
    /// `Conflict`, which the sweep treats as "defer and retry" — this one is a
    /// model-directory problem that retrying will never clear.
    DrafterAmbiguous(String),
    EmptyResponse,
    BadToolCall(String),
}

impl fmt::Display for GemmaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BinaryNotFound(s) => write!(f, "binary not found: {s}"),
            Self::ModelNotFound(s) => write!(f, "model not found: {s}"),
            Self::ServerStartTimeout => write!(f, "llama-server not ready within 60 s"),
            Self::ServerExitedEarly => write!(f, "llama-server exited before becoming ready"),
            Self::Http(s) => write!(f, "HTTP error: {s}"),
            Self::Conflict(s) => write!(f, "inference conflict: {s}"),
            Self::DrafterAmbiguous(s) => write!(f, "MTP drafter ambiguous: {s}"),
            Self::EmptyResponse => write!(f, "Gemma returned an empty response"),
            Self::BadToolCall(s) => write!(f, "tool-call response invalid: {s}"),
        }
    }
}

impl From<reqwest::Error> for GemmaError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e.to_string())
    }
}

pub fn run_inference(
    backend: &InferenceBackend,
    ctx_size: u32,
    max_tokens: u32,
    provider: &ChatProvider,
    transcript: &str,
) -> Result<GemmaOutput, GemmaError> {
    let system_prompt = build_system_prompt(provider, transcript);
    let output = match backend {
        InferenceBackend::LlamaCpp {
            bin,
            model,
            port,
            gpu_layers,
        } => llamacpp::run(
            bin,
            model,
            *port,
            *gpu_layers,
            ctx_size,
            max_tokens,
            &system_prompt,
            transcript,
        ),
        InferenceBackend::Ollama { host, port, model } => ollama::run(
            host,
            *port,
            model,
            ctx_size,
            max_tokens,
            &system_prompt,
            transcript,
        ),
    }?;
    Ok(ground_tags_in_transcript(output, transcript))
}

/// Select and render the system prompt for a session: coding vs. general,
/// sized to the transcript. Shared by single-shot `run_inference` and the
/// warm-server `SweepSession` so both prompt identically.
fn build_system_prompt(provider: &ChatProvider, transcript: &str) -> String {
    let target_words = summary_word_target(transcript);
    if provider.is_coding() {
        coding_system_prompt(target_words)
    } else {
        general_system_prompt(target_words)
    }
}

fn summary_word_target(transcript: &str) -> usize {
    let estimated_tokens = transcript.chars().count() / 4;
    ((estimated_tokens as f64 * SUMMARY_WORD_FRACTION).round() as usize)
        .clamp(MIN_SUMMARY_WORDS, MAX_SUMMARY_WORDS)
}

fn coding_system_prompt(target_words: usize) -> String {
    format!(
        "You are a technical scribe. Analyse the AI coding session transcript the user provides \
         and call save_session_summary with a detailed, developer-quality result. \
         For the summary be specific and complete: cover Goal, What Was Done, Key Decisions, \
         Files Changed, Open Issues, and Suggested Next Step. \
         Aim for approximately {target_words} words, staying concise but complete. \
         If the transcript contains a comparison table, benchmark result, rating, verdict, or \
         decision matrix, copy it into verbatim_highlights exactly as written — including its \
         column headings and its conclusions. Do not paraphrase it and do not count it against \
         the word budget above; verbatim_highlights is separate from the summary. \
         For error_tags and topic_tags, only use short tags that are explicitly grounded in the \
         transcript text itself, such as literal technologies, filenames, APIs, libraries, or \
         error names that appear in the transcript. Do not infer broad languages, frameworks, or \
         domains unless they are directly mentioned."
    )
}

fn general_system_prompt(target_words: usize) -> String {
    format!(
        "You are summarizing an AI conversation session. Analyse the normalized transcript and \
         call save_session_summary with a clear result. For the summary be specific: cover the \
         main topic, key questions, key answers or decisions, unresolved follow-ups, and any \
         notable next steps. Aim for approximately {target_words} words, staying concise but \
         complete. If the transcript contains a comparison table, rating, verdict, or decision \
         matrix, copy it into verbatim_highlights exactly as written — including its column \
         headings and its conclusions. Do not paraphrase it and do not count it against the word \
         budget above; verbatim_highlights is separate from the summary. \
         Do not assume the conversation is about coding unless it clearly is. \
         For error_tags and topic_tags, only use short tags that are explicitly grounded in the \
         transcript text itself. Do not invent inferred tags that are not literally supported by \
         the transcript."
    )
}

/// Upper bounds on the highlights list. These exist to stop the model from
/// dumping large spans of transcript into a field that bypasses the word budget
/// — the point is a handful of verdicts, not a second copy of the session.
const MAX_HIGHLIGHTS: usize = 12;
const MAX_HIGHLIGHT_CHARS: usize = 2_000;

fn ground_tags_in_transcript(mut output: GemmaOutput, transcript: &str) -> GemmaOutput {
    output.error_tags = retain_grounded_tags(&output.error_tags, transcript);
    output.topic_tags = retain_grounded_tags(&output.topic_tags, transcript);
    output.verbatim_highlights =
        retain_grounded_highlights(&output.verbatim_highlights, transcript);
    output
}

/// Keep only highlights that genuinely appear in the transcript. A highlight is
/// a *verbatim* quote, so the bar is the whole string, not a token: after
/// normalisation (which collapses table pipes, newlines and padding to single
/// spaces) it must be a substring of the normalised transcript. Anything the
/// model paraphrased or invented is dropped rather than archived as a quote.
fn retain_grounded_highlights(highlights: &[String], transcript: &str) -> Vec<String> {
    let transcript_norm = normalize_text(transcript);
    let mut kept: Vec<String> = Vec::new();
    for highlight in highlights {
        if kept.len() >= MAX_HIGHLIGHTS {
            break;
        }
        let trimmed = highlight.trim();
        if trimmed.is_empty() || trimmed.chars().count() > MAX_HIGHLIGHT_CHARS {
            continue;
        }
        let norm = normalize_text(trimmed);
        if norm.is_empty() || !transcript_norm.contains(&norm) {
            continue;
        }
        if !kept.iter().any(|existing| normalize_text(existing) == norm) {
            kept.push(trimmed.to_string());
        }
    }
    kept
}

fn retain_grounded_tags(tags: &[String], transcript: &str) -> Vec<String> {
    let transcript_norm = normalize_text(transcript);
    let mut kept = Vec::new();
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() || trimmed.chars().count() > 40 {
            continue;
        }
        let tag_norm = normalize_text(trimmed);
        if tag_norm.is_empty() {
            continue;
        }
        let grounded = if tag_norm.contains(' ') {
            transcript_norm.contains(&tag_norm)
        } else {
            transcript_norm
                .split_whitespace()
                .any(|word| word == tag_norm)
        };
        if grounded
            && !kept
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(trimmed))
        {
            kept.push(trimmed.to_string());
        }
    }
    kept
}

fn normalize_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_space = true;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_space = false;
        } else if !last_was_space {
            out.push(' ');
            last_was_space = true;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
