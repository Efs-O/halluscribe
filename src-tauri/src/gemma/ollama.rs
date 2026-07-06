// HalluScribe - Ollama request handling for Gemma session summaries.

use super::schema::{extract_ollama_tool_args, parse_ollama_tool_args, save_session_summary_tool};
use super::{GemmaError, GemmaOutput, INFER_TIMEOUT, TEMPERATURE};
use serde_json::Value;
use std::time::Duration;

/// How long Ollama keeps the model resident between sweep sessions. Positive so
/// the model stays warm across a whole sweep (audit P-1); the session's `Drop`
/// then unloads it with `keep_alive=0` when the sweep finishes.
const SWEEP_KEEP_ALIVE_SECS: u32 = 600;

/// An Ollama model kept warm for the duration of one sweep. Unlike the
/// single-shot `run`, it does not set `keep_alive=0` per call - it unloads once,
/// on `Drop`, so the model loads a single time for all sessions.
pub(crate) struct OllamaSession {
    client: reqwest::blocking::Client,
    base: String,
    model: String,
    ctx_size: u32,
}

impl OllamaSession {
    pub(crate) fn start(
        host: &str,
        port: u16,
        model: &str,
        ctx_size: u32,
    ) -> Result<Self, GemmaError> {
        let base = format!("http://{host}:{port}");
        let client = reqwest::blocking::Client::new();
        // Clear any model another job left resident before we warm ours.
        force_unload_loaded_model(&client, &base, model);
        Ok(Self {
            client,
            base,
            model: model.to_string(),
            ctx_size,
        })
    }

    pub(crate) fn infer(
        &self,
        max_tokens: u32,
        system_prompt: &str,
        transcript: &str,
    ) -> Result<GemmaOutput, GemmaError> {
        let payload = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user",   "content": transcript}
            ],
            "tools": [save_session_summary_tool()],
            "stream": false,
            "think": false,
            "keep_alive": SWEEP_KEEP_ALIVE_SECS,
            "options": {
                "temperature": TEMPERATURE,
                "num_predict": max_tokens,
                "num_ctx": self.ctx_size
            }
        });
        let value: Value = self
            .client
            .post(format!("{}/api/chat", self.base))
            .json(&payload)
            .timeout(INFER_TIMEOUT)
            .send()?
            .json()
            .map_err(|e| GemmaError::Http(e.to_string()))?;
        parse_ollama_tool_args(&value)
    }

    /// Run one completion against an arbitrary caller-supplied tool schema,
    /// returning the raw parsed tool-call arguments. Used by the profile
    /// distiller's map/reduce steps, which do not share `save_session_summary`.
    pub(crate) fn infer_tool(
        &self,
        max_tokens: u32,
        system_prompt: &str,
        user_content: &str,
        tool: &Value,
    ) -> Result<Value, GemmaError> {
        let payload = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user",   "content": user_content}
            ],
            "tools": [tool],
            "stream": false,
            "think": false,
            "keep_alive": SWEEP_KEEP_ALIVE_SECS,
            "options": {
                "temperature": TEMPERATURE,
                "num_predict": max_tokens,
                "num_ctx": self.ctx_size
            }
        });
        let value: Value = self
            .client
            .post(format!("{}/api/chat", self.base))
            .json(&payload)
            .timeout(INFER_TIMEOUT)
            .send()?
            .json()
            .map_err(|e| GemmaError::Http(e.to_string()))?;
        extract_ollama_tool_args(&value)
    }
}

impl Drop for OllamaSession {
    fn drop(&mut self) {
        // Sweep finished - unload the model now (the amended "unload when the
        // sweep finishes" rule for the Ollama backend).
        let _ = self
            .client
            .post(format!("{}/api/generate", self.base))
            .json(&serde_json::json!({"model": self.model, "keep_alive": 0}))
            .timeout(Duration::from_secs(10))
            .send();
    }
}

pub(crate) fn run(
    host: &str,
    port: u16,
    model: &str,
    ctx_size: u32,
    max_tokens: u32,
    system_prompt: &str,
    transcript: &str,
) -> Result<GemmaOutput, GemmaError> {
    let base = format!("http://{host}:{port}");
    let client = reqwest::blocking::Client::new();
    force_unload_loaded_model(&client, &base, model);

    let payload = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user",   "content": transcript}
        ],
        "tools": [save_session_summary_tool()],
        "stream": false,
        "think": false,
        "keep_alive": 0,
        "options": {
            "temperature": TEMPERATURE,
            "num_predict": max_tokens,
            "num_ctx": ctx_size
        }
    });
    let value: Value = client
        .post(format!("{base}/api/chat"))
        .json(&payload)
        .timeout(INFER_TIMEOUT)
        .send()?
        .json()
        .map_err(|e| GemmaError::Http(e.to_string()))?;
    parse_ollama_tool_args(&value)
}

fn force_unload_loaded_model(client: &reqwest::blocking::Client, base: &str, model: &str) {
    if let Ok(response) = client
        .get(format!("{base}/api/ps"))
        .timeout(Duration::from_secs(3))
        .send()
    {
        if let Ok(value) = response.json::<Value>() {
            if let Some(models) = value["models"].as_array() {
                if !models.is_empty() {
                    let loaded = models
                        .first()
                        .and_then(|m| m["name"].as_str())
                        .unwrap_or(model);
                    let _ = client
                        .post(format!("{base}/api/generate"))
                        .json(&serde_json::json!({"model": loaded, "keep_alive": 0}))
                        .timeout(Duration::from_secs(10))
                        .send();
                }
            }
        }
    }
}
