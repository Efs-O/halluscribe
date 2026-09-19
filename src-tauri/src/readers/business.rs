// HalluScribe - the reader dispatch: every `ScanTarget` to its `ParsedSession`
// list.
//
// Phase 7b of the Business Messaging ingestion plan. Extracted from
// `readers/mod.rs` so that file stays under the LOC ceiling as the business
// providers (Messages, WhatsApp, WhatsApp Business, Viber) are added. This is
// the single place a `ScanTarget` is turned into sessions: coding tools go
// through the preprocessor, chat exports through their own readers, and the
// Apple-backup readers (which all read the same configured backup directory,
// the `ChatProvider` selecting the app's domain) through `read_backup_target`.
// Nothing here logs names, numbers or text.

use super::{
    apple_messages, chatgpt, claudeai, estimate_fill_pct, gemini, grok, halluscribe_agent_chat,
    ollama_chat, stable_hash, viber, whatsapp, ChatProvider, MessageRole, ParsedMessage,
    ParsedSession, ReaderError,
};
use crate::archive;
use crate::preprocessor;
use crate::scanner::{ScanTarget, ScanTargetKind, ToolSource};
use chrono::{DateTime, Utc};

/// Read the sessions for one `ScanTarget`: coding tools through the
/// preprocessor, chat exports through their own readers, and the
/// Apple-backup readers through `read_backup_target`.
pub fn read_target(
    target: &ScanTarget,
    default_cc: Option<&str>,
) -> Result<Vec<ParsedSession>, ReaderError> {
    match &target.kind {
        ScanTargetKind::Coding(tool) => read_coding_target(target, tool),
        ScanTargetKind::Import(provider) => match provider {
            ChatProvider::ChatGPT => chatgpt::read(&target.path),
            ChatProvider::ClaudeAI => claudeai::read(&target.path),
            ChatProvider::Gemini => gemini::read(&target.path),
            ChatProvider::Grok => grok::read(&target.path),
            ChatProvider::HalluScribeAgentChat => halluscribe_agent_chat::read(&target.path),
            ChatProvider::OllamaChat => ollama_chat::read(&target.path),
            // The Apple-backup readers (Messages, WhatsApp, WhatsApp Business,
            // Viber): one dispatch, the provider selects the app's domain.
            ChatProvider::AppleMessages
            | ChatProvider::WhatsApp
            | ChatProvider::WhatsAppBusiness
            | ChatProvider::Viber => read_backup_target(provider, target, default_cc),
            ChatProvider::ClaudeCode | ChatProvider::Codex | ChatProvider::Forge => Ok(Vec::new()),
        },
    }
}

/// Read the sessions for one Apple-backup-based business provider. The backup
/// directory is `target.path`; the provider selects the app's domain. A
/// provider that is not an Apple-backup reader yields no sessions.
fn read_backup_target(
    provider: &ChatProvider,
    target: &ScanTarget,
    default_cc: Option<&str>,
) -> Result<Vec<ParsedSession>, ReaderError> {
    match provider {
        // The Apple backup reader: `target.path` is the backup directory.
        ChatProvider::AppleMessages => apple_messages::read(&target.path, default_cc),
        // The WhatsApp readers: `target.path` is the backup directory and the
        // provider selects the app's domain.
        ChatProvider::WhatsApp | ChatProvider::WhatsAppBusiness => {
            whatsapp::read(&target.path, provider.clone(), default_cc)
        }
        // The Viber reader: `target.path` is the backup directory.
        ChatProvider::Viber => viber::read(&target.path, default_cc),
        // Any other provider is not an Apple-backup reader.
        _ => Ok(Vec::new()),
    }
}

/// Read one coding-tool session: preprocess the JSONL, collapse it to a single
/// `Assistant` blob, and build the `ParsedSession`. The tokens come from the
/// preprocessor (never from the blob), and the whole file is the correct raw
/// because one JSONL file is one coding session.
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
    // A Forge session whose fill came from a compaction row is a real
    // measurement, not an estimate; only a missing fill falls back to the
    // character heuristic and is flagged estimated.
    let fill_estimated = target.fill_pct.is_none();
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
            speaker: None,
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
        project_override: None,
        preprocessed_units: Some(preprocessed.units),
    }])
}
