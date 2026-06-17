use app_lib::gemma::{GemmaOutput, SessionType};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::{path::Path, process::Command, time::Duration};

const INFER_TIMEOUT: Duration = Duration::from_secs(600);
const TEMPERATURE: f64 = 0.2;
const MAX_TOKENS: u32 = 4_096;
const SEP: &str = "============================================================";

pub fn ollama_tool_call(
    host: &str,
    port: u16,
    model: &str,
    transcript: &str,
) -> Result<GemmaOutput, String> {
    let payload = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_prompt() },
            { "role": "user", "content": transcript }
        ],
        "tools": [save_session_summary_tool()],
        "stream": false,
        "think": false,
        "keep_alive": 0,
        "options": {
            "temperature": TEMPERATURE,
            "num_predict": MAX_TOKENS
        }
    });

    let response: Value = Client::new()
        .post(format!("http://{host}:{port}/api/chat"))
        .json(&payload)
        .timeout(INFER_TIMEOUT)
        .send()
        .map_err(|error| format!("HTTP error: {error}"))?
        .json()
        .map_err(|error| format!("JSON parse error: {error}"))?;

    let args = &response["message"]["tool_calls"][0]["function"]["arguments"];
    if args.is_null() {
        let fallback = response["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        return Err(format!(
            "No tool_calls in response - model returned plain text instead.\nContent preview: {}",
            &fallback[..fallback.len().min(300)]
        ));
    }
    parse_args(args)
}

pub fn llamacpp_tool_call(
    bin: &str,
    model: &Path,
    port: u16,
    gpu_layers: i32,
    transcript: &str,
) -> Result<GemmaOutput, String> {
    let model_name = model
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("model")
        .to_string();
    let mut child = spawn_llama_server(bin, model, port, gpu_layers)?;
    wait_for_llama_server(&mut child, port)?;

    let payload = json!({
        "model": model_name,
        "messages": [
            { "role": "system", "content": system_prompt() },
            { "role": "user", "content": transcript }
        ],
        "tools": [save_session_summary_tool()],
        "temperature": TEMPERATURE,
        "max_tokens": MAX_TOKENS,
        "stream": false
    });

    let client = Client::new();
    let result = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&payload)
        .timeout(INFER_TIMEOUT)
        .send()
        .map_err(|error| format!("HTTP error: {error}"))
        .and_then(|response| {
            response
                .json::<Value>()
                .map_err(|error| format!("JSON parse error: {error}"))
        });

    let _ = child.kill();
    let response = result?;
    let args_raw = response["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
        .as_str()
        .ok_or_else(|| {
            let content = response["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string();
            format!(
                "No tool_calls in response - model returned plain text instead.\nContent preview: {}",
                &content[..content.len().min(300)]
            )
        })?;

    let args: Value = serde_json::from_str(args_raw)
        .map_err(|error| format!("arguments JSON parse error: {error}"))?;
    parse_args(&args)
}

pub fn print_output(label: &str, method: &str, out: &GemmaOutput) {
    println!("\n{SEP}");
    println!("  {label} - Method {method}");
    println!("{SEP}");
    println!("title:        {:?}", out.title);
    println!("session_type: {:?}", out.session_type);
    println!("error_tags:   {:?}", out.error_tags);
    println!("topic_tags:   {:?}", out.topic_tags);
    println!("summary ({} chars):", out.summary.len());
    println!("{}", out.summary);
}

pub fn print_err(label: &str, method: &str, error: impl std::fmt::Display) {
    println!("\n{SEP}");
    println!("  {label} - Method {method}  *** FAILED ***");
    println!("{SEP}");
    println!("ERROR: {error}");
}

fn save_session_summary_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "save_session_summary",
            "description": "Save a structured summary of the AI coding session.",
            "parameters": {
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Short imperative title, max 10 words."
                    },
                    "summary": {
                        "type": "string",
                        "description": "Detailed developer log covering: Goal, What Was Done, Key Decisions, Files Changed, Open Issues, Suggested Next Step. Minimum 300 words."
                    },
                    "session_type": {
                        "type": "string",
                        "enum": ["debugging", "building", "refactoring", "exploration"],
                        "description": "Primary type of work done in this session."
                    },
                    "error_tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Specific technologies, APIs, or error types that caused problems."
                    },
                    "topic_tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Topics, libraries, frameworks, or domain areas covered."
                    }
                },
                "required": ["title", "summary", "session_type", "error_tags", "topic_tags"]
            }
        }
    })
}

fn system_prompt() -> &'static str {
    "You are a technical scribe. Analyse the AI coding session transcript the user provides \
     and call save_session_summary with a detailed, developer-quality result. \
     For the summary be specific and complete: cover Goal, What Was Done, Key Decisions, \
     Files Changed, Open Issues, and Suggested Next Step."
}

fn spawn_llama_server(
    bin: &str,
    model: &Path,
    port: u16,
    gpu_layers: i32,
) -> Result<std::process::Child, String> {
    let gpu_layers = match gpu_layers {
        -1 => "all".to_string(),
        0 => "auto".to_string(),
        value => value.to_string(),
    };

    Command::new(bin)
        .args([
            "-m",
            &model.to_string_lossy(),
            "--port",
            &port.to_string(),
            "--n-gpu-layers",
            &gpu_layers,
            "--ctx-size",
            "32768",
            "--batch-size",
            "512",
            "--cache-type-k",
            "q8_0",
            "--cache-type-v",
            "q8_0",
            "--parallel",
            "1",
            "--flash-attn",
            "on",
            "--threads",
            "6",
            "--threads-batch",
            "6",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("Failed to spawn llama-server: {error}"))
}

fn wait_for_llama_server(child: &mut std::process::Child, port: u16) -> Result<(), String> {
    let client = Client::new();
    let health_url = format!("http://127.0.0.1:{port}/v1/models");

    for _ in 0..60 {
        std::thread::sleep(Duration::from_secs(1));
        if child.try_wait().ok().flatten().is_some() {
            return Err("llama-server exited before becoming ready".into());
        }
        if client
            .get(&health_url)
            .timeout(Duration::from_secs(2))
            .send()
            .map(|response| response.status().is_success())
            .unwrap_or(false)
        {
            return Ok(());
        }
    }

    let _ = child.kill();
    Err("llama-server not ready within 60 s".into())
}

fn parse_args(args: &Value) -> Result<GemmaOutput, String> {
    let str_val = |key: &str| args[key].as_str().unwrap_or("").to_string();
    let tags = |key: &str| -> Vec<String> {
        args[key]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let session_type = match args["session_type"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "debugging" => SessionType::Debugging,
        "building" => SessionType::Building,
        "refactoring" => SessionType::Refactoring,
        _ => SessionType::Exploration,
    };

    Ok(GemmaOutput {
        title: str_val("title"),
        summary: str_val("summary"),
        session_type,
        error_tags: tags("error_tags"),
        topic_tags: tags("topic_tags"),
    })
}
