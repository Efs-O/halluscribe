// HalluScribe - streaming Gemma inference for auto-briefing and chat turns.
// Keeps the public briefing API thin while backend/runtime details live in
// focused sibling modules.

mod filters;
mod idle;
mod llamacpp;
mod ollama;
mod prompt;
mod streaming;
pub(crate) mod tools;
mod web_search;

use crate::gemma::InferenceBackend;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::Emitter;

pub use filters::{BriefingFilters, BriefingScope};
pub(crate) use llamacpp::kill_server;
use tools::ToolCallResult;

pub(crate) const TEMPERATURE: f64 = 0.15;
pub(crate) const INFER_TIMEOUT: Duration = Duration::from_secs(600);
pub(crate) const STARTUP_TIMEOUT_SECS: u32 = 90;

#[derive(Clone)]
pub(crate) struct ChatRuntimeOptions {
    pub chat_scope: tools::ChatScope,
    pub web_search_enabled: bool,
    pub ollama_api_key: Option<String>,
    pub tavily_api_key: Option<String>,
    pub reasoning_enabled: bool,
}

#[derive(Serialize, Clone)]
pub struct TokenPayload {
    pub text: String,
    pub is_thinking: bool,
}

#[derive(Serialize, Clone)]
pub struct ToolCallPayload {
    pub tool: String,
    pub args: Value,
}

#[derive(Serialize, Clone)]
pub struct ChatUsagePayload {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub ctx_size: u32,
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_chat_turn(
    app: &tauri::AppHandle,
    backend: &InferenceBackend,
    ctx_size: u32,
    max_tokens: u32,
    initial_messages: Vec<Value>,
    archive_dir: &Path,
    runtime: ChatRuntimeOptions,
    cancel: Arc<AtomicBool>,
) {
    let Some(_inference_guard) = crate::infer_lock::try_acquire() else {
        emit_chat_token(
            app,
            "[Busy: another job (a sweep, briefing, or embedding run) is using the model. Try again once it finishes.]".to_string(),
            false,
        );
        let _ = app.emit("chat-done", ());
        return;
    };
    idle::mark_active();
    let tools = tools::chat_tools(&runtime);
    let mut messages = initial_messages;
    let mut replied = false;
    let mut used_web_tools = false;
    let mut compliance_retry_used = false;

    for _ in 0..20 {
        if cancel.load(Ordering::Relaxed) {
            replied = true;
            break;
        }
        let pass_result = match stream_chat_pass(
            app,
            backend,
            ctx_size,
            max_tokens,
            &messages,
            &tools,
            runtime.reasoning_enabled,
            &cancel,
        ) {
            Ok(result) => result,
            Err(e) => {
                emit_chat_token(app, format!("[Error: {e}]"), false);
                replied = true;
                break;
            }
        };

        if cancel.load(Ordering::Relaxed) {
            replied = true;
            break;
        }

        match pass_result {
            ToolCallResult::ToolCall { id, name, args } => {
                if name == "web_search" || name == "web_fetch" {
                    used_web_tools = true;
                }
                let _ = app.emit(
                    "chat-tool-call",
                    ToolCallPayload {
                        tool: name.clone(),
                        args: args.clone(),
                    },
                );
                let result = tools::execute_tool(archive_dir, &runtime, &name, &args);
                if let Some(error) = tool_error_message(&result) {
                    emit_chat_token(app, format!("[{name} error: {error}] "), false);
                }
                let arguments_field = match backend {
                    InferenceBackend::Ollama { .. } => args.clone(),
                    InferenceBackend::LlamaCpp { .. } => Value::String(args.to_string()),
                };
                messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{ "id": &id, "type": "function",
                                     "function": { "name": &name, "arguments": arguments_field } }]
                }));
                messages.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": result
                }));
            }
            ToolCallResult::Text {
                text,
                prompt_tokens,
                completion_tokens,
            } => {
                let normalized = if text.trim().is_empty() {
                    None
                } else {
                    Some(text)
                };
                if let Some(issue) = web_answer_issue(used_web_tools, normalized.as_deref()) {
                    if !compliance_retry_used {
                        compliance_retry_used = true;
                        messages.push(serde_json::json!({
                            "role": "assistant",
                            "content": normalized.unwrap_or_default()
                        }));
                        messages.push(serde_json::json!({
                            "role": "user",
                            "content": retry_prompt_for_issue(issue)
                        }));
                        continue;
                    }
                }
                if normalized.is_none() {
                    emit_chat_token(
                        app,
                        "I gathered data from the archive but was unable to produce a final answer. \
                         Try asking a more specific question or reducing the number of sessions involved."
                            .to_string(),
                        false,
                    );
                } else if used_web_tools
                    && !contains_sources_block(normalized.as_deref().unwrap_or_default())
                {
                    emit_chat_token(
                        app,
                        "\n\n[Warning: web search was used, but the answer did not include an explicit Sources section with exact URLs.]".to_string(),
                        false,
                    );
                }
                if ctx_size > 0 {
                    let _ = app.emit(
                        "chat-usage",
                        ChatUsagePayload {
                            prompt_tokens,
                            completion_tokens,
                            ctx_size,
                        },
                    );
                }
                replied = true;
                break;
            }
        }
    }

    if cancel.load(Ordering::Relaxed) {
        emit_chat_token(app, "[Stopped]".to_string(), false);
    } else if !replied {
        emit_chat_token(
            app,
            "I used all available tool-call steps but could not produce a final answer. \
             Try a more focused question or ask about fewer sessions at once."
                .to_string(),
            false,
        );
    }
    idle::mark_active();
    let _ = app.emit("chat-done", ());
}

#[allow(clippy::too_many_arguments)]
fn stream_chat_pass(
    app: &tauri::AppHandle,
    backend: &InferenceBackend,
    ctx_size: u32,
    max_tokens: u32,
    messages: &[Value],
    tools: &[Value],
    reasoning_enabled: bool,
    cancel: &AtomicBool,
) -> Result<ToolCallResult, String> {
    let mut emit = |text: String, is_thinking: bool| {
        emit_chat_token(app, text, is_thinking);
    };
    match backend {
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
            reasoning_enabled,
            messages,
            tools,
            &mut emit,
            cancel,
        ),
        InferenceBackend::Ollama { host, port, model } => ollama::stream(
            host,
            *port,
            model,
            ctx_size,
            max_tokens,
            reasoning_enabled,
            messages,
            tools,
            &mut emit,
            cancel,
        ),
    }
}

fn emit_chat_token(app: &tauri::AppHandle, text: String, is_thinking: bool) {
    let _ = app.emit("chat-token", TokenPayload { text, is_thinking });
}

fn tool_error_message(result: &str) -> Option<String> {
    let value: Value = serde_json::from_str(result).ok()?;
    value["error"].as_str().map(str::to_string)
}

fn contains_sources_block(text: &str) -> bool {
    text.lines().any(|line| {
        let normalized = line
            .trim()
            .trim_start_matches(['*', '-', '•', '`', '#', '>', ' '])
            .trim();
        let normalized = normalized.replace(['*', '`'], "");
        let normalized = normalized.trim().to_ascii_lowercase();
        normalized == "sources:"
            || normalized == "sources"
            || normalized.starts_with("sources:")
            || normalized.starts_with("sources ")
    })
}

fn contains_source_urls(text: &str) -> bool {
    text.contains("https://") || text.contains("http://")
}

#[derive(Clone, Copy)]
enum WebAnswerIssue {
    Empty,
    MissingSources,
}

fn web_answer_issue(used_web_tools: bool, text: Option<&str>) -> Option<WebAnswerIssue> {
    if !used_web_tools {
        return None;
    }
    match text {
        Some(text) if contains_sources_block(text) || contains_source_urls(text) => None,
        Some(_) => Some(WebAnswerIssue::MissingSources),
        None => Some(WebAnswerIssue::Empty),
    }
}

fn retry_prompt_for_issue(issue: WebAnswerIssue) -> &'static str {
    match issue {
        WebAnswerIssue::Empty => {
            "Your previous reply was empty. Using the archive evidence and the existing tool results already in the conversation, produce a complete final answer now. Do not call tools again unless absolutely necessary. If web tools were used, end with a `Sources:` section listing the exact URLs, one per line."
        }
        WebAnswerIssue::MissingSources => {
            "Your previous reply is already visible to the user, but it did not satisfy the web-evidence requirements. Do not restate the full answer. Append only a `Sources:` section for that reply now, listing the exact URLs used, one per line. If needed, add one short clarifying sentence immediately before `Sources:`."
        }
    }
}

pub fn archive_dir_path(home: PathBuf) -> PathBuf {
    home.join(".halluscribe")
}
