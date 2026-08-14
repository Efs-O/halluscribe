// HalluScribe - compare Ollama web search vs Tavily search with Gemma on llama.cpp.

use app_lib::settings::HalluScribeSettings;
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESULTS: u64 = 8;
const MAX_TOKENS: u32 = 1400;
const TEMPERATURE: f64 = 0.15;

fn main() {
    let archive_dir = archive_dir();
    let settings = HalluScribeSettings::default();
    let persisted =
        app_lib::settings::load_settings(&archive_dir).expect("failed to load settings.json");
    let model = env::var("GEMMA_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(persisted.gemma_model_path.trim()));
    let bin = env::var("LLAMA_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(persisted.llama_server_bin.trim()));
    let port: u16 = env::var("LLAMA_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(persisted.llama_server_port);
    let gpu_layers: i32 = env::var("LLAMA_GPU_LAYERS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(persisted.gpu_layers);

    let ollama_api_key = env::var("OLLAMA_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| persisted.ollama_api_key.trim().to_string());
    let tavily_api_key = env::var("TAVILY_API_KEY")
        .ok()
        .or_else(|| env::var("HALLUSCRIBE_TAVILY_API_KEY").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| persisted.tavily_api_key.trim().to_string());

    if bin.as_os_str().is_empty() || model.as_os_str().is_empty() {
        eprintln!("Missing llama.cpp binary or model path. Check settings or set LLAMA_BIN / GEMMA_MODEL.");
        std::process::exit(1);
    }
    if ollama_api_key.is_empty() || tavily_api_key.is_empty() {
        eprintln!(
            "Both Ollama and Tavily API keys must be available via settings or env vars (OLLAMA_API_KEY / TAVILY_API_KEY)."
        );
        std::process::exit(1);
    }

    let questions = [
        "As of April 2026, what changed in GPT-5.4 availability across ChatGPT, the API, and Codex, and what are two concrete capability gains over GPT-5.2?",
        "What is the latest status of OpenAI's Trusted Access for Cyber, and what does GPT-5.4-Cyber add that standard GPT-5.4 does not?",
        "What tools, context window, and token pricing are listed for GPT-5.4 in the OpenAI API docs, and how do those compare with GPT-5.2?",
    ];

    println!("=== HalluScribe - Web Provider Compare ===");
    println!("llama.cpp bin: {}", bin.display());
    println!("Gemma model: {}", model.display());
    println!("llama.cpp port: {port}");
    println!("Questions: {}", questions.len());
    println!("Search results per provider: {MAX_RESULTS}\n");

    let model_name = model
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("model")
        .to_string();
    let mut child = spawn_server(&bin, &model, port, gpu_layers, settings.ctx_size);
    wait_ready(port, &mut child);

    for (index, question) in questions.iter().enumerate() {
        println!(
            "\n{}\nQ{}: {}\n{}",
            "=".repeat(80),
            index + 1,
            question,
            "=".repeat(80)
        );

        let ollama_results = ollama_search(&ollama_api_key, question);
        let tavily_results = tavily_search(&tavily_api_key, question);

        print_results("Ollama", &ollama_results);
        print_results("Tavily", &tavily_results);

        let ollama_answer = gemma_answer(port, &model_name, "Ollama", question, &ollama_results);
        let tavily_answer = gemma_answer(port, &model_name, "Tavily", question, &tavily_results);

        println!("\n--- Gemma with Ollama search ---\n{}\n", ollama_answer);
        println!("--- Gemma with Tavily search ---\n{}\n", tavily_answer);
    }

    let _ = child.kill();
}

fn archive_dir() -> PathBuf {
    let home = env::var("USERPROFILE")
        .or_else(|_| env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".halluscribe")
}

fn spawn_server(bin: &Path, model: &Path, port: u16, gpu_layers: i32, ctx_size: u32) -> Child {
    let gpu_str = match gpu_layers {
        -1 => "all".to_string(),
        0 => "auto".to_string(),
        n => n.to_string(),
    };
    Command::new(bin)
        .args([
            "-m",
            &model.to_string_lossy(),
            "--port",
            &port.to_string(),
            "--n-gpu-layers",
            &gpu_str,
            "--ctx-size",
            &ctx_size.to_string(),
            "--parallel",
            "1",
            "--flash-attn",
            "on",
            "--threads",
            "6",
            "--reasoning",
            "off",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|error| panic!("failed to spawn llama-server: {error}"))
}

fn wait_ready(port: u16, child: &mut Child) {
    let client = Client::new();
    for _ in 0..90 {
        thread::sleep(Duration::from_secs(1));
        if child.try_wait().ok().flatten().is_some() {
            panic!("llama-server exited before becoming ready");
        }
        if client
            .get(format!("http://127.0.0.1:{port}/v1/models"))
            .timeout(Duration::from_secs(2))
            .send()
            .map(|response| response.status().is_success())
            .unwrap_or(false)
        {
            return;
        }
    }
    panic!("llama-server not ready within 90 seconds");
}

fn ollama_search(api_key: &str, query: &str) -> Value {
    Client::new()
        .post("https://ollama.com/api/web_search")
        .bearer_auth(api_key)
        .json(&json!({
            "query": query,
            "max_results": MAX_RESULTS,
        }))
        .timeout(TIMEOUT)
        .send()
        .and_then(|response| response.error_for_status())
        .unwrap_or_else(|error| panic!("Ollama search failed: {error}"))
        .json::<Value>()
        .unwrap_or_else(|error| panic!("Ollama search JSON decode failed: {error}"))
}

fn tavily_search(api_key: &str, query: &str) -> Value {
    Client::new()
        .post("https://api.tavily.com/search")
        .bearer_auth(api_key)
        .json(&json!({
            "query": query,
            "search_depth": "basic",
            "max_results": MAX_RESULTS,
        }))
        .timeout(TIMEOUT)
        .send()
        .and_then(|response| response.error_for_status())
        .unwrap_or_else(|error| panic!("Tavily search failed: {error}"))
        .json::<Value>()
        .unwrap_or_else(|error| panic!("Tavily search JSON decode failed: {error}"))
}

fn gemma_answer(
    port: u16,
    model_name: &str,
    provider: &str,
    question: &str,
    results: &Value,
) -> String {
    let messages = vec![
        json!({
            "role": "system",
            "content": "You are HalluScribe's web research assistant. Answer only from the supplied search results. If evidence is weak or conflicting, say so. End with a Sources: section listing exact URLs used."
        }),
        json!({
            "role": "user",
            "content": format!(
                "Provider: {provider}\nQuestion: {question}\n\nSearch results JSON:\n{}\n\nWrite a concise but specific answer grounded only in these results.",
                serde_json::to_string_pretty(results).unwrap_or_default()
            )
        }),
    ];
    let payload = json!({
        "model": model_name,
        "messages": messages,
        "temperature": TEMPERATURE,
        "max_tokens": MAX_TOKENS,
        "stream": false
    });
    Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&payload)
        .timeout(Duration::from_secs(600))
        .send()
        .and_then(|response| response.error_for_status())
        .unwrap_or_else(|error| panic!("llama.cpp completion failed: {error}"))
        .json::<Value>()
        .ok()
        .and_then(|value| {
            value["choices"][0]["message"]["content"]
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| "[empty answer]".to_string())
}

fn print_results(label: &str, response: &Value) {
    println!("\n--- {label} top results ---");
    if let Some(items) = response["results"].as_array() {
        for (index, item) in items.iter().take(3).enumerate() {
            let title = item["title"].as_str().unwrap_or("");
            let url = item["url"].as_str().unwrap_or("");
            println!("{}. {} | {}", index + 1, title, url);
        }
        println!("Total results returned: {}", items.len());
    } else {
        println!("No results");
    }
}
