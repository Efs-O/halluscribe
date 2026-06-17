// HalluScribe - Full briefing pipeline diagnostic.
// Calls collect_briefing_sessions with real archive data, prints the content,
// then streams it through the actual model path so we can see exactly what fails.
//
// Usage (Ollama):
//   cargo run --example briefing_pipeline_check
//
// Usage (llama.cpp):
//   BACKEND=llamacpp GEMMA_MODEL=<path> LLAMA_BIN=<path> \
//   cargo run --example briefing_pipeline_check

use app_lib::briefing::{collect_briefing_sessions, BriefingFilters, BriefingScope};
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
     The user has provided their recent AI coding session logs. \
     Write a concise narrative project briefing: how many sessions, \
     what was accomplished in each, recurring themes or blockers. \
     Be specific, use session titles and file names mentioned. \
     Keep it under 400 words.";

fn main() {
    let backend = env::var("BACKEND").unwrap_or_else(|_| "ollama".into());
    let window_hours: u32 = env::var("WINDOW_HOURS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    println!("=== HalluScribe - Briefing Pipeline Diagnostic ===");
    println!("Backend: {backend} | window_hours: {window_hours}\n");

    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .expect("cannot find home dir");
    let archive_dir = PathBuf::from(home).join(".halluscribe");
    println!("Archive dir: {}", archive_dir.display());

    println!("\n--- collect_briefing_sessions(window_hours={window_hours}, fallback=6) ---");
    let filters = BriefingFilters {
        date_from: None,
        date_to: None,
        fill_min: None,
        fill_max: None,
        keyword: None,
    };
    let (content, header, session_count) =
        match collect_briefing_sessions(&archive_dir, &BriefingScope::FilterBased(filters), 6) {
            Ok(v) => v,
            Err(error) => {
                println!("ERROR: {error}");
                return;
            }
        };

    println!(
        "Header: {header} | sessions: {session_count} | word_limit: {}",
        (session_count * 100).clamp(300, 1200)
    );
    println!(
        "Content length: {} chars (~{} tokens estimated)",
        content.len(),
        content.len() / 4
    );
    println!("\n--- First 500 chars of content ---");
    println!("{}", &content[..content.len().min(500)]);
    println!("--- End preview ---\n");

    let messages = vec![
        json!({"role": "system", "content": SYSTEM}),
        json!({"role": "user",   "content": content}),
    ];

    println!(
        "Total message payload size: {} chars",
        serde_json::to_string(&messages).unwrap().len()
    );
    println!("\nStarting stream...\n");

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

    println!("Ollama {model} at {host}:{port}...");

    let payload = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "think": false,
        "keep_alive": 0,
        "options": { "temperature": TEMPERATURE, "num_predict": MAX_TOKENS, "num_ctx": 32768 }
    });

    let resp = match Client::new()
        .post(format!("http://{host}:{port}/api/chat"))
        .json(&payload)
        .timeout(TIMEOUT)
        .send()
    {
        Ok(r) => r,
        Err(e) => {
            println!("REQUEST FAILED: {e}");
            return;
        }
    };

    println!("HTTP status: {}", resp.status());
    println!("--- tokens ---");
    let reader = BufReader::new(resp);
    let mut total = 0usize;
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                println!("[read error: {e}]");
                break;
            }
        };
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
        if let Some(err) = chunk["error"].as_str() {
            println!("\n!!! OLLAMA ERROR: {err}");
        }
        let text = chunk["message"]["content"].as_str().unwrap_or("");
        if !text.is_empty() {
            print!("{text}");
            total += text.len();
        }
        if chunk["done"].as_bool().unwrap_or(false) {
            println!("\n--- done ({total} chars) ---");
            if let Some(reason) = chunk["done_reason"].as_str() {
                println!("done_reason: {reason}");
            }
            break;
        }
    }
    if total == 0 {
        println!("\n!!! ZERO TOKENS received - model produced no output");
    }
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

    println!("Spawning llama-server {model_name} --reasoning off...");
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
            println!("!!! llama-server exited early");
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
        println!("!!! not ready in 90s");
        return;
    }
    println!("Server ready.\n--- tokens ---");

    let payload = json!({
        "model": model_name,
        "messages": messages,
        "temperature": TEMPERATURE,
        "max_tokens": MAX_TOKENS,
        "stream": true
    });

    let resp = match client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&payload)
        .timeout(TIMEOUT)
        .send()
    {
        Ok(r) => r,
        Err(e) => {
            println!("REQUEST FAILED: {e}");
            let _ = child.kill();
            return;
        }
    };

    println!("HTTP status: {}", resp.status());
    let reader = BufReader::new(resp);
    let mut total = 0usize;
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                println!("[read error: {e}]");
                break;
            }
        };
        let line = line.trim().to_string();
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
        if let Some(err) = chunk["error"].as_str() {
            println!("\n!!! LLAMA ERROR: {err}");
        }
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
    if total == 0 {
        println!("!!! ZERO TOKENS - model produced no output");
    }
}
