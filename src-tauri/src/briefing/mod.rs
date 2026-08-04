// HalluScribe - streaming Gemma inference for auto-briefing and chat turns.
// Keeps the public briefing API thin while backend/runtime details live in
// focused sibling modules.

mod chat;
mod filters;
mod idle;
mod llamacpp;
mod ollama;
mod prompt;
mod streaming;
pub(crate) mod tools;
mod web_search;

// Manual live probe (never runs in CI): lives inside `briefing` so it can reach
// the private llamacpp/chat/tools internals the real chat loop uses.
#[cfg(test)]
#[path = "retrieval_probe_tests.rs"]
mod retrieval_probe_tests;

use crate::gemma::InferenceBackend;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use tauri::Emitter;

pub(crate) use chat::{run_chat_turn, ChatRuntimeOptions};
pub use chat::{ChatUsagePayload, ToolCallPayload};
pub use filters::{BriefingFilters, BriefingScope};
pub(crate) use llamacpp::kill_server;
use tools::ToolCallResult;

pub(crate) const TEMPERATURE: f64 = 0.15;
pub(crate) const INFER_TIMEOUT: Duration = Duration::from_secs(600);
pub(crate) const STARTUP_TIMEOUT_SECS: u32 = 90;

#[derive(Serialize, Clone)]
pub struct TokenPayload {
    pub text: String,
    pub is_thinking: bool,
}

pub fn collect_briefing_sessions(
    archive_dir: &Path,
    scope: &BriefingScope,
    fallback_count: usize,
) -> Result<(String, String, usize), String> {
    filters::collect_briefing_sessions(archive_dir, scope, fallback_count)
}

#[allow(clippy::too_many_arguments)]
pub fn run_briefing_stream(
    app: &tauri::AppHandle,
    backend: &InferenceBackend,
    ctx_size: u32,
    max_tokens: u32,
    session_count: usize,
    content: &str,
    header: &str,
    min_word_limit: usize,
    max_word_limit: usize,
    cancel: Arc<AtomicBool>,
) {
    let Some(_inference_guard) = crate::infer_lock::try_acquire() else {
        let _ = app.emit(
            "briefing-token",
            TokenPayload {
                text: "[Busy: another job (a sweep, chat, or embedding run) is using the model. Try again once it finishes.]".to_string(),
                is_thinking: false,
            },
        );
        let _ = app.emit("briefing-done", ());
        return;
    };
    idle::mark_active();
    let _ = app.emit("briefing-header", header.to_string());
    let system_prompt = prompt::briefing_system_prompt(min_word_limit, max_word_limit);
    let user_prompt = format!(
        "Briefing scope: exactly {session_count} archived sessions are included below. \
         The authoritative session count is {session_count}. Use that number exactly in the briefing.\n\n{content}"
    );
    let messages = vec![
        serde_json::json!({"role": "system", "content": system_prompt}),
        serde_json::json!({"role": "user",   "content": user_prompt}),
    ];

    let result = match backend {
        InferenceBackend::LlamaCpp {
            bin,
            model,
            port,
            gpu_layers,
        } => llamacpp::stream(
            bin,
            model,
            *port,
            *gpu_layers,
            ctx_size,
            max_tokens,
            false,
            &messages,
            &[],
            |text, _is_thinking| {
                let _ = app.emit(
                    "briefing-token",
                    TokenPayload {
                        text,
                        is_thinking: false,
                    },
                );
            },
            &cancel,
        ),
        InferenceBackend::Ollama { host, port, model } => ollama::stream(
            host,
            *port,
            model,
            ctx_size,
            max_tokens,
            false,
            &messages,
            &[],
            |text, _is_thinking| {
                let _ = app.emit(
                    "briefing-token",
                    TokenPayload {
                        text,
                        is_thinking: false,
                    },
                );
            },
            &cancel,
        ),
    };

    match result {
        Err(e) => {
            let _ = app.emit(
                "briefing-token",
                TokenPayload {
                    text: format!("\n\n[Error: {e}]"),
                    is_thinking: false,
                },
            );
        }
        Ok(ToolCallResult::ToolCall { name, .. }) => {
            let _ = app.emit(
                "briefing-token",
                TokenPayload {
                    text: format!(
                        "\n\n[Error: unexpected tool call during briefing stream: {name}]"
                    ),
                    is_thinking: false,
                },
            );
        }
        Ok(ToolCallResult::Text { .. }) => {}
    }
    idle::mark_active();
    let _ = app.emit("briefing-done", ());
}

pub fn archive_dir_path(home: PathBuf) -> PathBuf {
    home.join(".halluscribe")
}
