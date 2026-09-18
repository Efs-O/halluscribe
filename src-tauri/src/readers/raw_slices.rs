// HalluScribe - per-session raw slices for source files that hold many
// sessions (chat exports, the Ollama DB). Used by the raw backfill to recover
// correct raws for sessions archived before slicing existed.

use super::{chatgpt, claudeai, gemini, grok, ollama_chat};
use std::collections::HashMap;
use std::path::Path;

/// Whether one source file of `provider_key` yields many sessions. A raw
/// already stored for such a session cannot be trusted: builds before
/// per-session slicing existed preserved the whole export for every
/// conversation in it, so the repair path must recompute and compare rather
/// than assume any existing raw is correct.
pub fn is_multi_session_provider(provider_key: &str) -> bool {
    matches!(
        provider_key,
        "chatgpt" | "claude_ai" | "gemini" | "grok" | "ollama_chat" | "apple_messages"
    )
}

/// Per-session raw slices for a source file that holds many sessions, keyed by
/// session id. Returns `None` for providers whose source file already maps 1:1
/// to a session, where preserving the whole file is the correct behaviour.
///
/// The source is parsed once, so every session sharing it is filled from a
/// single read rather than one read per session.
pub fn raw_slices_for_source(source: &Path, provider_key: &str) -> Option<HashMap<String, String>> {
    if !is_multi_session_provider(provider_key) {
        // One file per session: preserving the whole file is the correct raw.
        return None;
    }
    let parsed = match provider_key {
        "chatgpt" => chatgpt::read(source),
        "claude_ai" => claudeai::read(source),
        "gemini" => gemini::read(source),
        "grok" => grok::read(source),
        "ollama_chat" => ollama_chat::read(source),
        _ => return None,
    };

    // A parse failure yields an empty map, never `None`: the provider is still
    // multi-session, and reporting "no slices" must not let the caller fall
    // back to copying the whole export as one session's raw.
    Some(
        parsed
            .map(|sessions| {
                sessions
                    .into_iter()
                    .filter_map(|session| {
                        let id = session.id;
                        session.raw_slice.map(|slice| (id, slice))
                    })
                    .collect()
            })
            .unwrap_or_default(),
    )
}
