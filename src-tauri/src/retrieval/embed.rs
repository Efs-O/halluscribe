// HalluScribe - EmbeddingGemma bridge via a local llama.cpp embedding runtime.

use crate::llama_gpu::GpuConfig;
use crate::llama_runtime::{self, ServerWaitError};
use crate::settings::HalluScribeSettings;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const STARTUP_TIMEOUT_SECS: u32 = 60;
const EMBEDDING_TIMEOUT_SECS: u64 = 120;
const EMBEDDING_MODEL_NAME: &str = "embeddinggemma-300m";
const QUERY_PREFIX: &str = "task: search result | query: ";
const DEFAULT_DOCUMENT_TITLE: &str = "none";

enum EmbedMode {
    Query,
    Document,
}

pub struct EmbeddingRunner {
    port: u16,
    model_name: String,
}

impl EmbeddingRunner {
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    pub fn embed_query(&mut self, text: &str) -> Result<Vec<f32>, String> {
        let mut embeddings = self.embed_texts(&[text.to_string()], EmbedMode::Query)?;
        embeddings
            .pop()
            .ok_or_else(|| "embedding runtime returned no query vector".to_string())
    }

    pub fn embed_documents(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        self.embed_texts(texts, EmbedMode::Document)
    }

    fn embed_texts(&mut self, texts: &[String], mode: EmbedMode) -> Result<Vec<Vec<f32>>, String> {
        let prepared = texts
            .iter()
            .map(|text| match mode {
                EmbedMode::Query => prepare_query(text),
                EmbedMode::Document => prepare_document(text),
            })
            .collect::<Vec<_>>();

        call_embeddings(self.port, &self.model_name, &prepared)
    }
}

impl Drop for EmbeddingRunner {
    fn drop(&mut self) {
        kill_embedding_server();
    }
}

fn active_child() -> &'static Mutex<Option<Child>> {
    static CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
    CHILD.get_or_init(|| Mutex::new(None))
}

pub fn kill_embedding_server() {
    if let Ok(mut guard) = active_child().lock() {
        if let Some(mut child) = guard.take() {
            let pid = child.id();
            let _ = child.kill();
            crate::llama_pids::unregister(pid);
        }
    }
}

pub fn start_runner(settings: &HalluScribeSettings) -> Result<EmbeddingRunner, String> {
    let runtime = embedding_runtime(settings)?;
    let model_name = embedding_model_name(settings)?;
    let port = llama_runtime::find_free_port(runtime.port);
    let runtime = EmbeddingRuntime { port, ..runtime };
    // Reap a llama-server this app orphaned on a prior hard-kill so it frees
    // VRAM before we load the embedding model (OPS-1).
    crate::llama_pids::reap_orphans();
    let mut child = spawn_server(&runtime)?;
    if let Err(error) = wait_for_server(port, &mut child) {
        let _ = child.kill();
        return Err(error);
    }
    crate::llama_pids::register(child.id());
    let mut guard = active_child().lock().unwrap();
    if let Some(mut old) = guard.take() {
        let pid = old.id();
        let _ = old.kill();
        crate::llama_pids::unregister(pid);
    }
    *guard = Some(child);
    Ok(EmbeddingRunner { port, model_name })
}

pub fn embed_query(settings: &HalluScribeSettings, text: &str) -> Result<Vec<f32>, String> {
    let mut runner = start_runner(settings)?;
    runner.embed_query(text)
}

pub fn embedding_model_name(settings: &HalluScribeSettings) -> Result<String, String> {
    let model_path = PathBuf::from(settings.embedding_model_path.trim());
    if settings.embedding_model_path.trim().is_empty() {
        return Err(
            "Semantic search is not configured: set an EmbeddingGemma GGUF model path in Settings."
                .to_string(),
        );
    }
    let name = model_path
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(EMBEDDING_MODEL_NAME);
    Ok(name.to_string())
}

fn prepare_query(text: &str) -> String {
    format!("{QUERY_PREFIX}{}", text.trim())
}

fn prepare_document(text: &str) -> String {
    let title = extract_title(text).unwrap_or(DEFAULT_DOCUMENT_TITLE);
    format!("title: {title} | text: {}", text.trim())
}

fn extract_title(text: &str) -> Option<&str> {
    text.lines()
        .find_map(|line| line.strip_prefix("title: "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn call_embeddings(port: u16, model_name: &str, input: &[String]) -> Result<Vec<Vec<f32>>, String> {
    let payload = serde_json::json!({
        "model": model_name,
        "input": input
    });
    let value: Value = reqwest::blocking::Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/embeddings"))
        .json(&payload)
        .timeout(Duration::from_secs(EMBEDDING_TIMEOUT_SECS))
        .send()
        .map_err(|error| format!("embedding request failed: {error}"))?
        .json()
        .map_err(|error| format!("embedding runtime returned invalid JSON: {error}"))?;
    if let Some(items) = value["data"].as_array() {
        return items
            .iter()
            .map(|item| parse_embedding_row(&item["embedding"]))
            .collect();
    }
    if let Some(items) = value["value"].as_array() {
        return items
            .iter()
            .map(|item| parse_legacy_embedding_row(&item["embedding"]))
            .collect();
    }
    Err(format!(
        "embedding runtime returned no data array: {}",
        value
    ))
}

fn parse_embedding_row(value: &Value) -> Result<Vec<f32>, String> {
    let mut embedding = value
        .as_array()
        .ok_or_else(|| "embedding runtime returned a non-vector row".to_string())?
        .iter()
        .map(|value| {
            value
                .as_f64()
                .map(|n| n as f32)
                .ok_or_else(|| "embedding runtime returned a non-numeric value".to_string())
        })
        .collect::<Result<Vec<f32>, String>>()?;
    normalize_embedding(&mut embedding);
    Ok(embedding)
}

fn parse_legacy_embedding_row(value: &Value) -> Result<Vec<f32>, String> {
    let nested = value
        .as_array()
        .and_then(|items| items.first())
        .ok_or_else(|| "embedding runtime returned a legacy empty embedding row".to_string())?;
    parse_embedding_row(nested)
}

fn normalize_embedding(embedding: &mut [f32]) {
    let norm = embedding
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if norm > f32::EPSILON {
        for value in embedding {
            *value /= norm;
        }
    }
}

fn embedding_runtime(settings: &HalluScribeSettings) -> Result<EmbeddingRuntime, String> {
    let bin_path = settings.llama_server_bin.trim();
    let model_path = settings.embedding_model_path.trim();
    if bin_path.is_empty() {
        return Err(
            "Semantic search is not configured: set a llama-server binary path in Settings."
                .to_string(),
        );
    }
    if model_path.is_empty() {
        return Err(
            "Semantic search is not configured: set an EmbeddingGemma GGUF model path in Settings."
                .to_string(),
        );
    }
    let bin = llama_runtime::resolve_bin(Path::new(bin_path))
        .ok_or_else(|| format!("llama-server binary not found: {bin_path}"))?;
    let model = PathBuf::from(model_path);
    if !model.exists() {
        return Err(format!(
            "EmbeddingGemma GGUF model not found: {}",
            model.display()
        ));
    }
    Ok(EmbeddingRuntime {
        bin,
        model,
        port: settings.llama_server_port.saturating_add(1),
        gpu: settings.gpu_config(),
    })
}

fn spawn_server(runtime: &EmbeddingRuntime) -> Result<Child, String> {
    // EmbeddingGemma reports `gemma-embedding`, a different architecture from
    // the `gemma4` the sweep and chat run, so it gets its own block of
    // llama-tuning.yaml and its much larger batch size is a value the user can
    // now see and change.
    let resolved = crate::llama_tuning::resolve_host_tuning(&runtime.model)?;
    let mut cmd = Command::new(&runtime.bin);
    llama_runtime::apply_serve_subcommand(&mut cmd, &runtime.bin);
    // The context and the embedding switches are this role's own: an embedding
    // server that is not in embedding mode is not an embedding server.
    cmd.args([
        "-m",
        &runtime.model.to_string_lossy(),
        "--port",
        &runtime.port.to_string(),
        "--ctx-size",
        "4096",
        "--embedding",
        "--pooling",
        "mean",
    ]);
    resolved.tuning.apply(&mut cmd);
    runtime.gpu.apply(&mut cmd)?;
    llama_runtime::apply_output_capture(&mut cmd);
    llama_runtime::apply_no_window(&mut cmd);
    cmd.spawn()
        .map_err(|error| format!("failed to start embedding runtime: {error}"))
}

fn wait_for_server(port: u16, child: &mut Child) -> Result<(), String> {
    llama_runtime::wait_until_ready(port, child, STARTUP_TIMEOUT_SECS).map_err(
        |error| match error {
            ServerWaitError::ExitedEarly => {
                "embedding llama-server exited before becoming ready".to_string()
            }
            ServerWaitError::Timeout => {
                format!("embedding llama-server not ready within {STARTUP_TIMEOUT_SECS} s")
            }
        },
    )
}

struct EmbeddingRuntime {
    bin: PathBuf,
    model: PathBuf,
    port: u16,
    gpu: GpuConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_query_uses_retrieval_prefix() {
        assert_eq!(
            prepare_query("port already in use"),
            "task: search result | query: port already in use"
        );
    }

    #[test]
    fn prepare_document_includes_title_when_present() {
        assert_eq!(
            prepare_document("title: Port fix\nsummary: fixed the port bug"),
            "title: Port fix | text: title: Port fix\nsummary: fixed the port bug"
        );
    }

    #[test]
    fn normalize_embedding_scales_unit_length() {
        let mut values = vec![3.0, 4.0];
        normalize_embedding(&mut values);
        assert!((values[0] - 0.6).abs() < 0.0001);
        assert!((values[1] - 0.8).abs() < 0.0001);
    }
}
