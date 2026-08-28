// HalluScribe - llama.cpp subprocess lifecycle and request handling for Gemma.

use super::schema::{
    extract_openai_tool_args, parse_openai_tool_args, required_summary_tool_choice,
    save_session_summary_tool,
};
use super::{GemmaError, GemmaOutput, INFER_TIMEOUT, STARTUP_TIMEOUT_SECS, TEMPERATURE};
use crate::llama_gpu::GpuConfig;
use crate::llama_runtime::{self, ServerWaitError};
use crate::llama_tuning::{resolve_host_tuning, ResolvedTuning, SamplingTuning};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// A llama-server held alive for the duration of a sweep so the model loads
/// once and serves every session, instead of a cold load per session (audit
/// P-1). Dropping it kills the subprocess, satisfying the amended rule that
/// the sweep unloads the model when it finishes.
pub(crate) struct LlamaServer {
    child: Child,
    port: u16,
    model_name: String,
    /// The sampler this model's architecture asks for. Resolved once at spawn
    /// and reused for every request, so a mid-sweep edit to the tuning file
    /// cannot make two sessions in one sweep run on different samplers.
    sampling: SamplingTuning,
}

impl LlamaServer {
    pub(crate) fn start(
        bin: &Path,
        model: &Path,
        port: u16,
        gpu: &GpuConfig,
        ctx_size: u32,
    ) -> Result<Self, GemmaError> {
        let bin = resolve_bin(bin)?;
        if !model.exists() {
            return Err(GemmaError::ModelNotFound(model.display().to_string()));
        }
        let port = llama_runtime::find_free_port(port);
        let model_name = model
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("model")
            .to_string();
        // Reap any llama-server this app orphaned on a prior hard-kill so it
        // releases VRAM before we load a fresh model (OPS-1).
        crate::llama_pids::reap_orphans();
        let resolved = resolve_host_tuning(model).map_err(GemmaError::TuningInvalid)?;
        let mut child = spawn_server(&bin, model, port, gpu, ctx_size, &resolved)?;
        if let Err(error) = wait_for_server(port, &mut child) {
            let _ = child.kill();
            return Err(error);
        }
        crate::llama_pids::register(child.id());
        Ok(Self {
            child,
            port,
            model_name,
            sampling: resolved.tuning.sampling,
        })
    }

    pub(crate) fn infer(
        &self,
        max_tokens: u32,
        system_prompt: &str,
        transcript: &str,
    ) -> Result<GemmaOutput, GemmaError> {
        call(
            self.port,
            &self.model_name,
            max_tokens,
            system_prompt,
            transcript,
            &self.sampling,
        )
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
        call_tool(
            self.port,
            &self.model_name,
            max_tokens,
            system_prompt,
            user_content,
            tool,
            &self.sampling,
        )
    }
}

impl Drop for LlamaServer {
    fn drop(&mut self) {
        let pid = self.child.id();
        let _ = self.child.kill();
        crate::llama_pids::unregister(pid);
    }
}

/// Single-shot inference: load the model, run one completion, unload. Used by
/// the public `run_inference` API (examples and ad-hoc callers). The sweep uses
/// `LlamaServer` directly to keep the model warm across sessions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    bin: &Path,
    model: &Path,
    port: u16,
    gpu: &GpuConfig,
    ctx_size: u32,
    max_tokens: u32,
    system_prompt: &str,
    transcript: &str,
) -> Result<GemmaOutput, GemmaError> {
    let server = LlamaServer::start(bin, model, port, gpu, ctx_size)?;
    server.infer(max_tokens, system_prompt, transcript)
}

fn resolve_bin(bin: &Path) -> Result<PathBuf, GemmaError> {
    llama_runtime::resolve_bin(bin)
        .ok_or_else(|| GemmaError::BinaryNotFound(bin.display().to_string()))
}

fn spawn_server(
    bin: &Path,
    model: &Path,
    port: u16,
    gpu: &GpuConfig,
    ctx_size: u32,
    resolved: &ResolvedTuning,
) -> Result<Child, GemmaError> {
    let mut cmd = Command::new(bin);
    llama_runtime::apply_serve_subcommand(&mut cmd, bin);
    // Path, port, reasoning and context stay here: Settings and this role own
    // them. Everything else comes from the model's block of llama-tuning.yaml.
    cmd.args([
        "-m",
        &model.to_string_lossy(),
        "--port",
        &port.to_string(),
        "--reasoning",
        "off",
        "--ctx-size",
        &ctx_size.to_string(),
    ]);
    resolved.tuning.apply(&mut cmd);
    gpu.apply(&mut cmd).map_err(GemmaError::GpuConfigInvalid)?;
    crate::llama_mtp::apply_mtp_flags(&mut cmd, model, resolved)
        .map_err(GemmaError::DrafterAmbiguous)?;
    llama_runtime::apply_output_capture(&mut cmd);
    llama_runtime::apply_no_window(&mut cmd);
    cmd.spawn()
        .map_err(|e| GemmaError::BinaryNotFound(e.to_string()))
}

fn wait_for_server(port: u16, child: &mut Child) -> Result<(), GemmaError> {
    llama_runtime::wait_until_ready(port, child, STARTUP_TIMEOUT_SECS).map_err(
        |error| match error {
            ServerWaitError::ExitedEarly => GemmaError::ServerExitedEarly,
            ServerWaitError::Timeout => GemmaError::ServerStartTimeout,
        },
    )
}

fn call(
    port: u16,
    model_name: &str,
    max_tokens: u32,
    system_prompt: &str,
    transcript: &str,
    sampling: &SamplingTuning,
) -> Result<GemmaOutput, GemmaError> {
    let mut payload = serde_json::json!({
        "model": model_name,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user",   "content": transcript}
        ],
        "tools": [save_session_summary_tool()],
        "tool_choice": required_summary_tool_choice(),
        "temperature": TEMPERATURE,
        "max_tokens": max_tokens,
        "stream": false
    });
    // The sweep's own temperature above stays: it is per-role, not per-model.
    sampling.apply_to_payload(&mut payload);
    let value: Value = reqwest::blocking::Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&payload)
        .timeout(INFER_TIMEOUT)
        .send()?
        .json()
        .map_err(|e| GemmaError::Http(e.to_string()))?;
    parse_openai_tool_args(&value)
}

#[allow(clippy::too_many_arguments)]
fn call_tool(
    port: u16,
    model_name: &str,
    max_tokens: u32,
    system_prompt: &str,
    user_content: &str,
    tool: &Value,
    sampling: &SamplingTuning,
) -> Result<Value, GemmaError> {
    let mut payload = serde_json::json!({
        "model": model_name,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user",   "content": user_content}
        ],
        "tools": [tool],
        "temperature": TEMPERATURE,
        "max_tokens": max_tokens,
        "stream": false
    });
    sampling.apply_to_payload(&mut payload);
    let value: Value = reqwest::blocking::Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&payload)
        .timeout(INFER_TIMEOUT)
        .send()?
        .json()
        .map_err(|e| GemmaError::Http(e.to_string()))?;
    extract_openai_tool_args(&value)
}
