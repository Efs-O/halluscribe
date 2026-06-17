// HalluScribe — Briefing stream diagnostic.
// Tests the exact streaming path used by run_briefing_stream for both backends.
// No tools, plain text completion, streaming only.
//
// Usage (Ollama):
//   cargo run --example briefing_stream_check
//
// Usage (llama.cpp):
//   BACKEND=llamacpp GEMMA_MODEL=<path> LLAMA_BIN=<path> \
//   cargo run --example briefing_stream_check

use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::{
    env,
    io::{BufRead, BufReader},
    path::PathBuf,
    process::Command,
    thread,
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_secs(600);
const TEMPERATURE: f64 = 0.15;
const MAX_TOKENS: u32 = 4096;

const SYSTEM: &str = "You are a senior developer's personal assistant. \
     Write a concise narrative project briefing based on the session logs provided. \
     Keep it under 200 words.";

const USER: &str = "Session: Fix Port In Use Error on Launch (2026-04-18)\n\
     Tool: Claude Code | Fill: 72% | Tags: port, tauri, vite\n\
     Summary: Fixed a port conflict that prevented the dev server from starting.";

fn main() {
    let backend = env::var("BACKEND").unwrap_or_else(|_| "ollama".into());
    println!("=== HalluScribe — Briefing Stream Diagnostic ===");
    println!("Backend: {backend}\n");

    let messages = vec![
        json!({"role": "system", "content": SYSTEM}),
        json!({"role": "user",   "content": USER}),
    ];

    match backend.as_str() {
        "llamacpp" => run_llamacpp(&messages),
        _ => run_ollama(&messages),
    }
}

fn run_ollama(messages: &[Value]) {
    let host = env::var("OLLAMA_HOST").unwrap_or_else(|_| "localhost".into());
    let port: u16 = env::var("OLLAMA_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(11434);
    let model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "gemma4:26b".into());

    println!("Calling Ollama {model} at {host}:{port} — stream:true, think:false, no tools...\n");

    let payload = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "think": false,
        "keep_alive": 0,
        "options": { "temperature": TEMPERATURE, "num_predict": MAX_TOKENS, "num_ctx": 32768 }
    });

    let resp = Client::new()
        .post(format!("http://{host}:{port}/api/chat"))
        .json(&payload)
        .timeout(TIMEOUT)
        .send()
        .expect("request failed");

    println!("--- streaming tokens ---");
    let reader = BufReader::new(resp);
    let mut total = 0usize;
    for line in reader.lines() {
        let line = line.expect("read error");
        if line.trim().is_empty() {
            continue;
        }
        let chunk: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                println!("[parse error: {e}] raw: {line}");
                continue;
            }
        };
        let text = chunk["message"]["content"].as_str().unwrap_or("");
        if !text.is_empty() {
            print!("{text}");
            total += text.len();
        }
        if chunk["done"].as_bool().unwrap_or(false) {
            break;
        }
    }
    println!("\n--- done ({total} chars) ---");
}

fn run_llamacpp(messages: &[Value]) {
    let model_path = PathBuf::from(env::var("GEMMA_MODEL").expect("GEMMA_MODEL not set"));
    let bin = env::var("LLAMA_BIN").unwrap_or_else(|_| "llama-server".into());
    let port: u16 = env::var("LLAMA_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);
    let gpu_layers = env::var("LLAMA_GPU_LAYERS")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(-1);
    let gpu_str = match gpu_layers {
        -1 => "all".into(),
        0 => "auto".into(),
        n => n.to_string(),
    };
    let model_name = model_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_string();

    println!("Spawning llama-server {model_name} on port {port} --reasoning off...");
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
            "--threads-batch",
            "6",
            "--reasoning",
            "off",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn failed");

    let client = Client::new();
    let health = format!("http://127.0.0.1:{port}/v1/models");
    let mut ready = false;
    for _ in 0..90 {
        thread::sleep(Duration::from_secs(1));
        if child.try_wait().ok().flatten().is_some() {
            eprintln!("llama-server exited early");
            return;
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
        eprintln!("not ready in 90s");
        return;
    }
    println!("Server ready. Sending streaming request — no tools...\n");

    let payload = json!({
        "model": model_name,
        "messages": messages,
        "temperature": TEMPERATURE,
        "max_tokens": MAX_TOKENS,
        "stream": true
    });

    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&payload)
        .timeout(TIMEOUT)
        .send()
        .expect("request failed");

    println!("--- streaming tokens ---");
    let reader = BufReader::new(resp);
    let mut total = 0usize;
    for line in reader.lines() {
        let line = line.expect("read error");
        let line = line.trim();
        if !line.starts_with("data: ") {
            continue;
        }
        let data = &line[6..];
        if data == "[DONE]" {
            break;
        }
        let chunk: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(e) => {
                println!("[parse error: {e}]");
                continue;
            }
        };
        let text = chunk["choices"][0]["delta"]["content"]
            .as_str()
            .unwrap_or("");
        if !text.is_empty() {
            print!("{text}");
            total += text.len();
        }
    }
    let _ = child.kill();
    println!("\n--- done ({total} chars) ---");
}
