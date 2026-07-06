// HalluScribe - Native tool-calling comparison test.
// Runs the same preprocessed transcript through two methods and prints both
// outputs side by side so you can compare before switching gemma.rs.

mod tool_calling_check_support;

use app_lib::{
    gemma::{run_inference, InferenceBackend},
    preprocessor::{preprocess_session, PreprocessError},
    readers::ChatProvider,
    scanner::{scan_sessions, ToolSource},
    settings::HalluScribeSettings,
};
use std::{
    env,
    path::{Path, PathBuf},
};
use tool_calling_check_support::{llamacpp_tool_call, ollama_tool_call, print_err, print_output};

fn main() {
    let defaults = HalluScribeSettings::default();
    let archive_dir = archive_dir();
    println!("=== HalluScribe - Tool-Calling Comparison Test ===");
    println!("Method A = current plain JSON prompting (run_inference)");
    println!("Method B = native tool-calling (/api/chat + tools array)\n");

    let test_backend = env::var("TEST_BACKEND").unwrap_or_else(|_| "ollama".into());
    let run_llama = test_backend == "both" || test_backend == "llamacpp";
    let run_ollama = test_backend == "both" || test_backend == "ollama";

    let (jsonl_path, tool) = resolve_jsonl_path(&archive_dir);
    let transcript = preprocess_transcript(&jsonl_path, &tool);

    if run_ollama {
        run_ollama_checks(&transcript, defaults.ctx_size, defaults.max_tokens);
    }
    if run_llama {
        run_llamacpp_checks(&transcript, defaults.ctx_size, defaults.max_tokens);
    }

    println!("\n=== Done - compare A vs B above before switching gemma.rs ===");
}

fn resolve_jsonl_path(archive_dir: &std::path::Path) -> (PathBuf, ToolSource) {
    match env::var("JSONL_PATH") {
        Ok(path_str) => {
            let path = PathBuf::from(&path_str);
            let tool = if path_str.contains(".claude") || path_str.contains("claude") {
                ToolSource::ClaudeCode
            } else {
                ToolSource::Codex
            };
            println!("Using provided JSONL: {}", path.display());
            (path, tool)
        }
        Err(_) => {
            println!("Scanning for most recent session (last 30 days, any fill)...");
            let sessions = scan_sessions(
                archive_dir,
                &HalluScribeSettings::default(),
                30 * 24 * 3600,
                0.0,
                false,
            );
            if sessions.is_empty() {
                eprintln!("ERROR: No session files found. Set JSONL_PATH to a specific file.");
                std::process::exit(1);
            }
            let session = sessions.into_iter().next().unwrap();
            let tool = match session.kind {
                app_lib::scanner::ScanTargetKind::Coding(tool) => tool,
                app_lib::scanner::ScanTargetKind::Import(_) => ToolSource::Codex,
            };
            println!(
                "Selected: {} (fill={:.1}%, tool={:?})",
                session.path.display(),
                session.fill_pct.unwrap_or(0.0),
                tool
            );
            (session.path, tool)
        }
    }
}

fn preprocess_transcript(jsonl_path: &Path, tool: &ToolSource) -> String {
    println!("\n--- Preprocessing ---");
    let transcript = match preprocess_session(jsonl_path, tool) {
        Ok(transcript) => transcript,
        Err(PreprocessError::UnsupportedTool) => {
            eprintln!(
                "ERROR: Continue tool not yet supported. Set JSONL_PATH to Claude Code or Codex."
            );
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("ERROR preprocessing: {error}");
            std::process::exit(1);
        }
    };
    println!(
        "Transcript: {} chars (~{} tokens estimated)\n",
        transcript.len(),
        transcript.len() / 4
    );
    transcript
}

fn run_ollama_checks(transcript: &str, ctx_size: u32, max_tokens: u32) {
    let host = env::var("OLLAMA_HOST").unwrap_or_else(|_| "localhost".into());
    let port: u16 = env::var("OLLAMA_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(11434);
    let model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "gemma4:26b".into());
    println!("--- Ollama: {model} on {host}:{port} ---");

    println!("\n[A] Running plain JSON prompting (current method)...");
    let backend = InferenceBackend::Ollama {
        host: host.clone(),
        port,
        model: model.clone(),
    };
    match run_inference(
        &backend,
        ctx_size,
        max_tokens,
        &provider_for_tool(&ToolSource::Codex),
        transcript,
    ) {
        Ok(output) => print_output("Ollama", "A (JSON prompt)", &output),
        Err(error) => print_err("Ollama", "A (JSON prompt)", error),
    }

    println!("\n[B] Running native tool-calling (new method)...");
    match ollama_tool_call(&host, port, &model, transcript) {
        Ok(output) => print_output("Ollama", "B (tool-calling)", &output),
        Err(error) => print_err("Ollama", "B (tool-calling)", &error),
    }
}

fn run_llamacpp_checks(transcript: &str, ctx_size: u32, max_tokens: u32) {
    let model_path = match env::var("GEMMA_MODEL") {
        Ok(path) => PathBuf::from(path),
        Err(_) => {
            eprintln!("SKIP llama.cpp: GEMMA_MODEL not set.");
            return;
        }
    };
    let bin = env::var("LLAMA_BIN").unwrap_or_else(|_| "llama-server".into());
    let port: u16 = env::var("LLAMA_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8080);
    let gpu_layers: i32 = env::var("LLAMA_GPU_LAYERS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(-1);
    let model_name = model_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("model")
        .to_string();

    println!("\n--- llama.cpp: {model_name} on port {port} ---");

    println!("\n[A] Running plain JSON prompting (current method)...");
    let backend = InferenceBackend::LlamaCpp {
        bin: PathBuf::from(&bin),
        model: model_path.clone(),
        port,
        gpu_layers,
    };
    match run_inference(
        &backend,
        ctx_size,
        max_tokens,
        &provider_for_tool(&ToolSource::Codex),
        transcript,
    ) {
        Ok(output) => print_output("llama.cpp", "A (JSON prompt)", &output),
        Err(error) => print_err("llama.cpp", "A (JSON prompt)", error),
    }

    println!("\n[B] Running native tool-calling (new method)...");
    match llamacpp_tool_call(&bin, &model_path, port, gpu_layers, transcript) {
        Ok(output) => print_output("llama.cpp", "B (tool-calling)", &output),
        Err(error) => print_err("llama.cpp", "B (tool-calling)", &error),
    }
}

fn provider_for_tool(tool: &ToolSource) -> ChatProvider {
    match tool {
        ToolSource::ClaudeCode => ChatProvider::ClaudeCode,
        ToolSource::Codex => ChatProvider::Codex,
        ToolSource::Continue => ChatProvider::Continue,
        ToolSource::Forge => ChatProvider::Forge,
    }
}

fn archive_dir() -> PathBuf {
    let home = env::var("USERPROFILE")
        .or_else(|_| env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".halluscribe")
}
