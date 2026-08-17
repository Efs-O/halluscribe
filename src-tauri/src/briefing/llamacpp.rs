// HalluScribe - llama.cpp streaming and tool-call transport helpers.

use super::server;
use super::streaming;
use super::tools::ToolCallResult;
use super::{INFER_TIMEOUT, STARTUP_TIMEOUT_SECS, TEMPERATURE};
use crate::llama_gpu::GpuConfig;
use crate::llama_runtime::{self, ServerWaitError};
use crate::llama_tuning::{resolve_host_tuning, ResolvedTuning, SamplingTuning};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::atomic::AtomicBool;

#[allow(clippy::too_many_arguments)]
pub(crate) fn stream(
    bin: &Path,
    model: &Path,
    port: u16,
    gpu: &GpuConfig,
    ctx_size: u32,
    max_tokens: u32,
    reasoning_enabled: bool,
    messages: &[Value],
    tools: &[Value],
    mut emit: impl FnMut(String, bool),
    cancel: &AtomicBool,
) -> Result<ToolCallResult, String> {
    if !model.exists() {
        return Err(format!("model not found: {}", model.display()));
    }
    let multimodal = has_images(messages);
    let prepared_messages = prepare_messages(messages)?;
    // Resolved every turn, not only when a server is spawned: the sampler goes
    // into the request body, so a warm server must still pick up an edit to the
    // tuning file on the next message rather than at the next cold start.
    let resolved = resolve_host_tuning(model)?;
    // Reuse the warm server only when it was spawned with these exact settings.
    // A changed model path (or ctx/gpu/reasoning/multimodal flag) retires it, so
    // picking a different GGUF takes effect on this turn instead of whenever the
    // idle watchdog next fires.
    let spec = server::ServerSpec::new(model, gpu, ctx_size, reasoning_enabled, multimodal);
    let port = match server::reusable_port(&spec) {
        Some(port) => port,
        None => {
            let free = llama_runtime::find_free_port(port);
            // Reap a llama-server this app orphaned on a prior hard-kill so it frees
            // VRAM before we load the chat model (OPS-1).
            crate::llama_pids::reap_orphans();
            let mut child = spawn_server(
                bin,
                model,
                free,
                gpu,
                ctx_size,
                reasoning_enabled,
                multimodal,
                &resolved,
            )?;
            if let Err(e) = wait_ready(free, &mut child) {
                let _ = child.kill();
                return Err(e);
            }
            crate::llama_pids::register(child.id());
            server::store(child, spec, free);
            free
        }
    };
    let model_name = model
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_string();
    do_stream(
        port,
        &model_name,
        max_tokens,
        &prepared_messages,
        tools,
        &resolved.tuning.sampling,
        &mut emit,
        cancel,
    )
}

#[allow(clippy::too_many_arguments)]
fn do_stream(
    port: u16,
    model_name: &str,
    max_tokens: u32,
    messages: &[Value],
    tools: &[Value],
    sampling: &SamplingTuning,
    emit: &mut impl FnMut(String, bool),
    cancel: &AtomicBool,
) -> Result<ToolCallResult, String> {
    let mut payload = serde_json::json!({
        "model": model_name,
        "messages": messages,
        "temperature": TEMPERATURE,
        "max_tokens": max_tokens,
        "stream": true,
        "stream_options": { "include_usage": true }
    });
    if !tools.is_empty() {
        payload["tools"] = serde_json::json!(tools);
    }
    // Chat's own, warmer temperature above stays: it is per-role, not per-model.
    sampling.apply_to_payload(&mut payload);
    let response = reqwest::blocking::Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&payload)
        .timeout(INFER_TIMEOUT)
        .send()
        .map_err(|e| e.to_string())?;
    streaming::consume_openai_stream(response, emit, cancel)
}

#[allow(clippy::too_many_arguments)]
fn spawn_server(
    bin: &Path,
    model: &Path,
    port: u16,
    gpu: &GpuConfig,
    ctx_size: u32,
    reasoning_enabled: bool,
    multimodal: bool,
    resolved: &ResolvedTuning,
) -> Result<Child, String> {
    let mut command = Command::new(bin);
    llama_runtime::apply_serve_subcommand(&mut command, bin);
    // Path, port, context and the reasoning switch stay here: Settings and this
    // turn own them. The rest comes from the model's block of llama-tuning.yaml.
    command
        .args([
            "-m",
            &model.to_string_lossy(),
            "--port",
            &port.to_string(),
            "--ctx-size",
            &ctx_size.to_string(),
        ])
        .args(if reasoning_enabled {
            vec!["--reasoning", "on"]
        } else {
            vec!["--reasoning", "off"]
        });
    resolved.tuning.apply(&mut command);
    gpu.apply(&mut command)?;
    if multimodal {
        let mmproj = find_mmproj(model)?;
        command.args(["--mmproj", &mmproj.to_string_lossy()]);
    }
    crate::llama_mtp::apply_mtp_flags(&mut command, model, resolved)?;
    llama_runtime::apply_output_capture(&mut command);
    llama_runtime::apply_no_window(&mut command);
    command
        .spawn()
        .map_err(|e| format!("failed to spawn llama-server: {e}"))
}

fn has_images(messages: &[Value]) -> bool {
    messages.iter().any(|message| {
        message["images"]
            .as_array()
            .is_some_and(|images| !images.is_empty())
    })
}

fn prepare_messages(messages: &[Value]) -> Result<Vec<Value>, String> {
    messages
        .iter()
        .map(|message| {
            let Some(images) = message["images"].as_array() else {
                return Ok(message.clone());
            };
            if images.is_empty() {
                return Ok(message.clone());
            }
            let mut next = message.clone();
            let text = next["content"].as_str().unwrap_or("").to_string();
            let mut parts = vec![serde_json::json!({ "type": "text", "text": text })];
            for image in images {
                let mime = image["mime_type"].as_str().unwrap_or("image/png");
                let data = image["data"]
                    .as_str()
                    .ok_or_else(|| "chat image payload is missing base64 data".to_string())?;
                parts.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:{mime};base64,{data}") }
                }));
            }
            next["content"] = Value::Array(parts);
            if let Some(object) = next.as_object_mut() {
                object.remove("images");
            }
            Ok(next)
        })
        .collect()
}

fn find_mmproj(model: &Path) -> Result<std::path::PathBuf, String> {
    let dir = model
        .parent()
        .ok_or_else(|| format!("model path has no parent directory: {}", model.display()))?;
    let entries = fs::read_dir(dir)
        .map_err(|e| format!("failed to scan model directory {}: {e}", dir.display()))?;
    let mut candidates = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let lowered = name.to_ascii_lowercase();
        if lowered.contains("mmproj") && lowered.ends_with(".gguf") {
            candidates.push(path);
        }
    }
    if candidates.is_empty() {
        return Err(format!(
            "Image chat with llama.cpp requires an mmproj GGUF in the model directory. No mmproj*.gguf file was found next to {}.",
            model.display()
        ));
    }
    if candidates.len() == 1 {
        return Ok(candidates.remove(0));
    }
    let model_key = mmproj_match_key(model);
    if let Some(path) = candidates.iter().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.to_ascii_lowercase().contains(&model_key))
    }) {
        return Ok(path.clone());
    }
    let names = candidates
        .iter()
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "Found multiple mmproj GGUF files next to {} but could not choose one automatically: {}",
        model.display(),
        names
    ))
}

fn mmproj_match_key(model: &Path) -> String {
    let stem = model
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    stem.rsplit_once('-')
        .map(|(prefix, _)| prefix.to_string())
        .unwrap_or(stem)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)] // runtime helpers below intentionally follow the tests
mod tests {
    use super::{find_mmproj, has_images, mmproj_match_key, prepare_messages};
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn prepare_messages_converts_image_payloads_to_openai_content_parts() {
        let messages = vec![json!({
            "role": "user",
            "content": "Describe this image.",
            "images": [{ "mime_type": "image/jpeg", "data": "abc123" }]
        })];

        let prepared = prepare_messages(&messages).unwrap();
        assert_eq!(prepared.len(), 1);
        assert!(prepared[0]["images"].is_null());
        assert_eq!(prepared[0]["content"][0]["type"], "text");
        assert_eq!(prepared[0]["content"][0]["text"], "Describe this image.");
        assert_eq!(prepared[0]["content"][1]["type"], "image_url");
        assert_eq!(
            prepared[0]["content"][1]["image_url"]["url"],
            "data:image/jpeg;base64,abc123"
        );
    }

    #[test]
    fn has_images_detects_multimodal_turns() {
        assert!(!has_images(&[json!({"role": "user", "content": "hello"})]));
        assert!(has_images(&[json!({
            "role": "user",
            "content": "hello",
            "images": [{ "mime_type": "image/png", "data": "abc" }]
        })]));
    }

    #[test]
    fn mmproj_match_key_drops_quant_suffix() {
        let path = std::path::Path::new("gemma-4-26B-A4B-it-Q4_K_M.gguf");
        assert_eq!(mmproj_match_key(path), "gemma-4-26b-a4b-it");
    }

    #[test]
    fn find_mmproj_prefers_matching_sidecar_file() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("halluscribe-mmproj-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let model = dir.join("gemma-4-26B-A4B-it-Q4_K_M.gguf");
        let preferred = dir.join("mmproj-gemma-4-26B-A4B-it-f16.gguf");
        let other = dir.join("mmproj-other-model.gguf");
        fs::write(&model, "").unwrap();
        fs::write(&preferred, "").unwrap();
        fs::write(&other, "").unwrap();

        let found = find_mmproj(&model).unwrap();
        assert_eq!(found, preferred);

        let _ = fs::remove_file(model);
        let _ = fs::remove_file(preferred);
        let _ = fs::remove_file(other);
        let _ = fs::remove_dir(dir);
    }
}

fn wait_ready(port: u16, child: &mut Child) -> Result<(), String> {
    llama_runtime::wait_until_ready(port, child, STARTUP_TIMEOUT_SECS).map_err(
        |error| match error {
            ServerWaitError::ExitedEarly => "llama-server exited before becoming ready".to_string(),
            ServerWaitError::Timeout => {
                format!("llama-server not ready within {STARTUP_TIMEOUT_SECS} s")
            }
        },
    )
}
