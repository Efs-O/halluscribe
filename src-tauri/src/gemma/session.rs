// HalluScribe - sweep-scoped inference session that keeps one model warm.
// The sweep loads the model once via `start_sweep_session`, runs every session
// through `SweepSession::infer`, then drops the session to unload the model
// (audit P-1). Strictly sequential and held under the single inference lock the
// sweep already owns, so it never weakens the no-concurrency guarantee.

use super::llamacpp::LlamaServer;
use super::ollama::OllamaSession;
use super::InferenceBackend;
use super::{build_system_prompt, ground_tags_in_transcript, GemmaError, GemmaOutput};
use crate::readers::ChatProvider;

/// One warm inference backend held for the lifetime of a single sweep.
pub struct SweepSession(Backend);

enum Backend {
    LlamaCpp(LlamaServer),
    Ollama(OllamaSession),
}

impl SweepSession {
    /// Summarise one session against the already-loaded model.
    pub fn infer(
        &self,
        max_tokens: u32,
        provider: &ChatProvider,
        transcript: &str,
    ) -> Result<GemmaOutput, GemmaError> {
        let system_prompt = build_system_prompt(provider, transcript);
        let output = match &self.0 {
            Backend::LlamaCpp(server) => server.infer(max_tokens, &system_prompt, transcript),
            Backend::Ollama(session) => session.infer(max_tokens, &system_prompt, transcript),
        }?;
        Ok(ground_tags_in_transcript(output, transcript))
    }
}

/// Load the model once for an entire sweep. `ctx_size` is fixed for the sweep
/// (llama.cpp bakes it into the server; Ollama passes it per request).
pub fn start_sweep_session(
    backend: &InferenceBackend,
    ctx_size: u32,
) -> Result<SweepSession, GemmaError> {
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
    Ok(SweepSession(inner))
}
