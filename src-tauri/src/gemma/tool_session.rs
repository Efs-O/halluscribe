// HalluScribe - generic warm-server tool-call session for non-summary
// inference (the profile distiller's map and reduce steps). Mirrors
// `SweepSession`: one warm backend serves the caller's whole batch of calls
// and is unloaded when the session is dropped.

use super::llamacpp::LlamaServer;
use super::ollama::OllamaSession;
use super::{GemmaError, InferenceBackend};
use serde_json::Value;

/// One warm inference backend held across an arbitrary sequence of tool
/// calls (e.g. every map batch plus the final reduce call of a profile
/// refresh). Unlike `SweepSession`, the caller supplies its own tool schema
/// and receives the raw parsed tool-call arguments instead of a fixed
/// `GemmaOutput` shape.
pub struct ToolSession(Backend);

enum Backend {
    LlamaCpp(LlamaServer),
    Ollama(OllamaSession),
}

impl ToolSession {
    /// Run one tool-call completion against the already-loaded model.
    pub fn call_tool(
        &self,
        system_prompt: &str,
        user_content: &str,
        tool: &Value,
        max_tokens: u32,
    ) -> Result<Value, GemmaError> {
        match &self.0 {
            Backend::LlamaCpp(server) => {
                server.infer_tool(max_tokens, system_prompt, user_content, tool)
            }
            Backend::Ollama(session) => {
                session.infer_tool(max_tokens, system_prompt, user_content, tool)
            }
        }
    }
}

/// Load the model once for a whole batch of tool calls (map+reduce). Dropping
/// the returned session unloads the model, matching the sweep's "unload when
/// the batch job finishes" policy.
pub fn start_tool_session(
    backend: &InferenceBackend,
    ctx_size: u32,
) -> Result<ToolSession, GemmaError> {
    let inner = match backend {
        InferenceBackend::LlamaCpp {
            bin,
            model,
            port,
            gpu_layers,
        } => Backend::LlamaCpp(LlamaServer::start(
            bin,
            model,
            *port,
            *gpu_layers,
            ctx_size,
        )?),
        InferenceBackend::Ollama { host, port, model } => {
            Backend::Ollama(OllamaSession::start(host, *port, model, ctx_size)?)
        }
    };
    Ok(ToolSession(inner))
}
