// HalluScribe — Chat tool-call loop diagnostic.
// Mimics run_chat_turn step-by-step and prints raw JSON at every step.
// Tells us exactly what Gemma returns after a tool result is injected.
//
// Usage (Ollama):
//   cargo run --example chat_loop_check
//
// Usage (llama.cpp):
//   BACKEND=llamacpp GEMMA_MODEL=<path> LLAMA_BIN=<path> \
//   cargo run --example chat_loop_check

use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::{env, path::PathBuf, process::Command, thread, time::Duration};

const TIMEOUT: Duration = Duration::from_secs(600);
const TEMPERATURE: f64 = 0.15;
const MAX_TOKENS: u32 = 2048;

fn search_sessions_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "search_sessions",
            "description": "Search the session archive. Returns metadata rows only.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query":   { "type": "string" },
                    "limit":   { "type": "integer" }
                }
            }
        }
    })
}

fn sep(label: &str) {
    println!("\n{}", "=".repeat(60));
    println!("  {label}");
    println!("{}\n", "=".repeat(60));
}

fn main() {
    let backend = env::var("BACKEND").unwrap_or_else(|_| "ollama".into());

    println!("=== HalluScribe — Chat Loop Diagnostic ===");
    println!("Backend: {backend}");

    let tools = vec![search_sessions_tool()];
    let system = json!({"role": "system", "content":
        "You are a helpful assistant with access to the user's session archive. \
         Use search_sessions to look up information before answering."
    });
    let user_msg = json!({"role": "user", "content":
        "What was the most recent port in use bug I fixed?"
    });

    let mut messages: Vec<Value> = vec![system, user_msg];

    println!(
        "\nInitial messages: {}",
        serde_json::to_string_pretty(&messages).unwrap()
    );

    for iteration in 0..4 {
        sep(&format!(
            "ITERATION {iteration} — calling Gemma (non-streaming)"
        ));

        let raw_response = match backend.as_str() {
            "llamacpp" => llamacpp_call(&messages, &tools),
            _ => ollama_call(&messages, &tools),
        };

        match raw_response {
            Err(e) => {
                println!("ERROR: {e}");
                break;
            }
            Ok(v) => {
                println!(
                    "RAW RESPONSE:\n{}",
                    serde_json::to_string_pretty(&v).unwrap()
                );

                // Thinking tag diagnosis
                let raw_content = if backend == "llamacpp" {
                    v["choices"][0]["message"]["content"].as_str().unwrap_or("")
                } else {
                    v["message"]["content"].as_str().unwrap_or("")
                };
                println!("\n--- THINKING TAG SCAN ---");
                for tag in &[
                    "<think>",
                    "</think>",
                    "<|channel>",
                    "<channel|>",
                    "<|thinking|>",
                    "</|thinking|>",
                ] {
                    if raw_content.contains(tag) {
                        println!("  FOUND: {tag}");
                    }
                }
                if !raw_content.contains('<') {
                    println!("  No XML-style tags found — plain text response");
                }
                println!("--- END SCAN ---");

                // Check for tool calls
                let (has_tool_call, tool_name, tool_args, tool_id, text_content) =
                    extract_response(&v, &backend);

                if has_tool_call {
                    println!("\n>>> TOOL CALL detected: {tool_name}({tool_args})");
                    println!(">>> Injecting fake tool result and looping...");

                    let fake_result = json!([{
                        "id": "sess_001",
                        "title": "Fix Port In Use Error on Launch",
                        "date": "2026-04-16",
                        "tool": "Claude Code",
                        "fill_pct": 72.0,
                        "topic_tags": ["port", "tauri", "vite", "halluscribe"]
                    }]);

                    // Ollama wants arguments as object; llama.cpp wants JSON string
                    let args_field = if backend == "llamacpp" {
                        serde_json::Value::String(tool_args.to_string())
                    } else {
                        tool_args.clone()
                    };
                    messages.push(json!({
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": tool_id,
                            "type": "function",
                            "function": { "name": tool_name, "arguments": args_field }
                        }]
                    }));
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_id,
                        "content": serde_json::to_string_pretty(&fake_result).unwrap()
                    }));

                    println!("\nMessages after tool injection:");
                    println!("{}", serde_json::to_string_pretty(&messages).unwrap());
                } else {
                    println!("\n>>> TEXT response (no tool call)");
                    println!(">>> content: {:?}", text_content);
                    if text_content.trim().is_empty() {
                        println!("!!! WARNING: content is EMPTY — this is the bug");
                    } else {
                        println!(">>> Loop complete — this is the final answer");
                    }
                    break;
                }
            }
        }
    }

    println!("\n=== Diagnostic complete ===");
}

fn extract_response(v: &Value, backend: &str) -> (bool, String, Value, String, String) {
    if backend == "llamacpp" {
        let msg = &v["choices"][0]["message"];
        if let Some(calls) = msg["tool_calls"].as_array() {
            if let Some(call) = calls.first() {
                let id = call["id"].as_str().unwrap_or("call_1").to_string();
                let name = call["function"]["name"].as_str().unwrap_or("").to_string();
                let args_raw = call["function"]["arguments"].as_str().unwrap_or("{}");
                let args: Value = serde_json::from_str(args_raw).unwrap_or(json!({}));
                return (true, name, args, id, String::new());
            }
        }
        let text = msg["content"].as_str().unwrap_or("").to_string();
        (false, String::new(), json!({}), String::new(), text)
    } else {
        // Ollama
        let msg = &v["message"];
        if let Some(calls) = msg["tool_calls"].as_array() {
            if let Some(call) = calls.first() {
                let name = call["function"]["name"].as_str().unwrap_or("").to_string();
                let args = call["function"]["arguments"].clone();
                return (true, name, args, "call_1".into(), String::new());
            }
        }
        let text = msg["content"].as_str().unwrap_or("").to_string();
        (false, String::new(), json!({}), String::new(), text)
    }
}

fn ollama_call(messages: &[Value], tools: &[Value]) -> Result<Value, String> {
    let host = env::var("OLLAMA_HOST").unwrap_or_else(|_| "localhost".into());
    let port: u16 = env::var("OLLAMA_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(11434);
    let model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "gemma4:26b".into());

    println!("Calling Ollama {model} at {host}:{port}...");

    let payload = json!({
        "model": model,
        "messages": messages,
        "tools": tools,
        "stream": false,
        "keep_alive": 0,
        "options": { "temperature": TEMPERATURE, "num_predict": MAX_TOKENS }
    });

    Client::new()
        .post(format!("http://{host}:{port}/api/chat"))
        .json(&payload)
        .timeout(TIMEOUT)
        .send()
        .map_err(|e| e.to_string())?
        .json::<Value>()
        .map_err(|e| e.to_string())
}

fn llamacpp_call(messages: &[Value], tools: &[Value]) -> Result<Value, String> {
    let model_path =
        PathBuf::from(env::var("GEMMA_MODEL").map_err(|_| "GEMMA_MODEL not set".to_string())?);
    let bin = env::var("LLAMA_BIN").unwrap_or_else(|_| "llama-server".into());
    let port: u16 = env::var("LLAMA_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);
    let gpu_layers = env::var("LLAMA_GPU_LAYERS")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(-1);

    let model_name = model_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_string();

    let gpu_str = match gpu_layers {
        -1 => "all".into(),
        0 => "auto".into(),
        n => n.to_string(),
    };

    // REASONING_MODE: "off" suppresses <think> tokens, "on" enables them (default)
    let reasoning = env::var("REASONING_MODE").unwrap_or_else(|_| "off".into());
    println!("Spawning llama-server {model_name} on port {port} (--reasoning {reasoning})...");
    let mut child = Command::new(&bin)
        .args([
            "-m",
            &model_path.to_string_lossy(),
            "--port",
            &port.to_string(),
            "--n-gpu-layers",
            &gpu_str,
            "--ctx-size",
            "32768",
            "--batch-size",
            "512",
            "--parallel",
            "1",
            "--flash-attn",
            "on",
            "--threads",
            "6",
            "--reasoning",
            &reasoning,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn failed: {e}"))?;

    let client = Client::new();
    let health = format!("http://127.0.0.1:{port}/v1/models");
    let mut ready = false;
    for _ in 0..90 {
        thread::sleep(Duration::from_secs(1));
        if child.try_wait().ok().flatten().is_some() {
            return Err("llama-server exited early".into());
        }
        if client
            .get(&health)
            .timeout(Duration::from_secs(2))
            .send()
            .map(|r| r.status().is_success())
            .unwrap_or(false)
        {
            ready = true;
            break;
        }
    }
    if !ready {
        let _ = child.kill();
        return Err("llama-server not ready within 90s".into());
    }

    println!("Server ready. Sending request...");

    let payload = json!({
        "model": model_name,
        "messages": messages,
        "tools": tools,
        "temperature": TEMPERATURE,
        "max_tokens": MAX_TOKENS,
        "stream": false
    });

    let result = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&payload)
        .timeout(TIMEOUT)
        .send()
        .map_err(|e| e.to_string())
        .and_then(|r| r.json::<Value>().map_err(|e| e.to_string()));

    let _ = child.kill();
    result
}
