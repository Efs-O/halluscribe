// HalluScribe - Ollama streaming and tool-call transport helpers.

use super::streaming;
use super::tools::ToolCallResult;
use super::{INFER_TIMEOUT, TEMPERATURE};
use serde_json::Value;
use std::sync::atomic::AtomicBool;

#[allow(clippy::too_many_arguments)]
pub(crate) fn stream(
    host: &str,
    port: u16,
    model: &str,
    ctx_size: u32,
    max_tokens: u32,
    thinking_enabled: bool,
    messages: &[Value],
    tools: &[Value],
    mut emit: impl FnMut(String, bool),
    cancel: &AtomicBool,
) -> Result<ToolCallResult, String> {
    let mut payload = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "think": thinking_enabled,
        "keep_alive": 0,
        "options": { "temperature": TEMPERATURE, "num_predict": max_tokens, "num_ctx": ctx_size }
    });
    if !tools.is_empty() {
        payload["tools"] = serde_json::json!(tools);
    }
    let response = reqwest::blocking::Client::new()
        .post(format!("http://{host}:{port}/api/chat"))
        .json(&payload)
        .timeout(INFER_TIMEOUT)
        .send()
        .map_err(|e| e.to_string())?;
    streaming::consume_ollama_stream(response, &mut emit, cancel)
}
