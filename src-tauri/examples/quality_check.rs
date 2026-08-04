// HalluScribe — Gemma quality checkpoint.
// Finds the most recent real JSONL, preprocesses it, and runs inference
// against both llama.cpp and Ollama backends, printing raw + parsed output.
//
// Usage:
//   cargo run --example quality_check
//
// Required env vars (llama.cpp backend):
//   GEMMA_MODEL   — absolute path to Gemma 4 GGUF file
//   LLAMA_BIN     — path/name of llama-server binary (default: "llama-server")
//   LLAMA_PORT    — server port (default: 8080)
//   LLAMA_GPU_LAYERS — GPU layers to offload (default: 35)
//
// Ollama backend (always attempted unless TEST_BACKEND=llamacpp):
//   OLLAMA_HOST   — default: localhost
//   OLLAMA_PORT   — default: 11434
//   OLLAMA_MODEL  — default: gemma4:26b
//
// Optional:
//   JSONL_PATH    — override: use this specific JSONL file instead of scanning
//   TEST_BACKEND  — "llamacpp" | "ollama" | "both" (default: "both")

use app_lib::{
    gemma::{run_inference, InferenceBackend},
    preprocessor::{preprocess_session, PreprocessError},
    readers::ChatProvider,
    scanner::{scan_sessions, ToolSource},
    settings,
};
use std::{env, path::PathBuf};

fn main() {
    let archive_dir = archive_dir();
    // Generation limits MUST come from the real settings file: `HalluScribeSettings::default()`
    // sets ctx_size and max_tokens to 0 ("must be set before generation can run"), and passing
    // those zeros to llama.cpp makes every tool call come back as bare `<|tool_call>` content
    // with no parsed tool_calls. Everything else can stay on defaults.
    let defaults = settings::load_settings(&archive_dir);
    assert!(
        defaults.ctx_size > 0 && defaults.max_tokens > 0,
        "ctx_size/max_tokens are unset in {}/settings.json — configure them in the app first",
        archive_dir.display()
    );
    println!("=== HalluScribe — Gemma Quality Checkpoint ===\n");

    let test_backend = env::var("TEST_BACKEND").unwrap_or_else(|_| "both".into());
    let run_llama = test_backend == "both" || test_backend == "llamacpp";
    let run_ollama = test_backend == "both" || test_backend == "ollama";

    // --- Resolve JSONL file ---------------------------------------------------
    let (jsonl_path, tool) = match env::var("JSONL_PATH") {
        Ok(p) => {
            let path = PathBuf::from(&p);
            // Guess tool from path.
            let tool = if p.contains(".claude") || p.contains("claude") {
                ToolSource::ClaudeCode
            } else {
                ToolSource::Codex
            };
            println!("Using provided JSONL: {}", path.display());
            (path, tool)
        }
        Err(_) => {
            println!("Scanning for most recent session (last 30 days, any fill)...");
            // 30-day lookback, 0% fill gate — we want any session for the test.
            let sessions = scan_sessions(&archive_dir, &defaults, 30 * 24 * 3600, 0.0, false);
            if sessions.is_empty() {
                eprintln!("ERROR: No session files found. Set JSONL_PATH to a specific file.");
                std::process::exit(1);
            }
            let s = sessions.into_iter().next().unwrap();
            let tool = match s.kind {
                app_lib::scanner::ScanTargetKind::Coding(tool) => tool,
                app_lib::scanner::ScanTargetKind::Import(_) => ToolSource::Codex,
            };
            println!(
                "Selected: {} (fill={:.1}%, tool={:?})",
                s.path.display(),
                s.fill_pct.unwrap_or(0.0),
                tool
            );
            (s.path, tool)
        }
    };

    // --- Preprocess -----------------------------------------------------------
    println!("\n--- Preprocessing ---");
    let transcript = match preprocess_session(&jsonl_path, &tool) {
        Ok(t) => t,
        Err(PreprocessError::UnsupportedTool) => {
            eprintln!("ERROR: Continue tool not yet supported for preprocessing.");
            eprintln!("Set JSONL_PATH to a Claude Code or Codex session.");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("ERROR preprocessing: {e}");
            std::process::exit(1);
        }
    };

    let char_count = transcript.len();
    // Rough token estimate: ~4 chars per token.
    let est_tokens = char_count / 4;
    println!("Transcript: {char_count} chars (~{est_tokens} tokens estimated)");
    println!("\n--- First 800 chars of preprocessed transcript ---");
    println!("{}", &transcript[..transcript.len().min(800)]);
    println!("...\n");

    // --- llama.cpp backend ---------------------------------------------------
    if run_llama {
        println!("========================================");
        println!("Backend: llama.cpp");
        println!("========================================");

        let model_path = match env::var("GEMMA_MODEL") {
            Ok(p) => PathBuf::from(p),
            Err(_) => {
                eprintln!("SKIP: GEMMA_MODEL not set — skipping llama.cpp test.");
                println!();
                run_ollama_if_needed(
                    &transcript,
                    &provider_for_tool(&tool),
                    run_ollama,
                    defaults.ctx_size,
                    defaults.max_tokens,
                );
                return;
            }
        };
        let bin_name = env::var("LLAMA_BIN").unwrap_or_else(|_| "llama-server".into());
        let port: u16 = env::var("LLAMA_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8080);
        // -1 = offload all layers, 0 = auto, >0 = exact count. Default: all.
        let gpu_layers: i32 = env::var("LLAMA_GPU_LAYERS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(-1);

        println!(
            "Config: bin={bin_name}, model={}, port={port}, gpu_layers={gpu_layers}",
            model_path.display()
        );
        println!("Starting llama-server and running inference (may take up to 10 min)...\n");

        let backend = InferenceBackend::LlamaCpp {
            bin: PathBuf::from(bin_name),
            model: model_path,
            port,
            gpu_layers,
        };
        match run_inference(
            &backend,
            defaults.ctx_size,
            defaults.max_tokens,
            &provider_for_tool(&tool),
            &transcript,
        ) {
            Ok(out) => print_output("llama.cpp", &out),
            Err(e) => eprintln!("llama.cpp ERROR: {e}"),
        }
        println!();
    }

    // --- Ollama backend ------------------------------------------------------
    run_ollama_if_needed(
        &transcript,
        &provider_for_tool(&tool),
        run_ollama,
        defaults.ctx_size,
        defaults.max_tokens,
    );
}

fn run_ollama_if_needed(
    transcript: &str,
    provider: &ChatProvider,
    run_ollama: bool,
    ctx_size: u32,
    max_tokens: u32,
) {
    if !run_ollama {
        return;
    }
    println!("========================================");
    println!("Backend: Ollama");
    println!("========================================");

    let host = env::var("OLLAMA_HOST").unwrap_or_else(|_| "localhost".into());
    let port: u16 = env::var("OLLAMA_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(11434);
    let model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "gemma4:26b".into());

    println!("Config: host={host}, port={port}, model={model}");
    println!("Running Ollama inference (may take up to 10 min)...\n");

    let backend = InferenceBackend::Ollama { host, port, model };
    match run_inference(&backend, ctx_size, max_tokens, provider, transcript) {
        Ok(out) => print_output("Ollama", &out),
        Err(e) => eprintln!("Ollama ERROR: {e}"),
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

fn print_output(backend_name: &str, out: &app_lib::gemma::GemmaOutput) {
    println!("--- {backend_name} raw parsed output ---");
    println!("title:        {:?}", out.title);
    println!("session_type: {:?}", out.session_type);
    println!("error_tags:   {:?}", out.error_tags);
    println!("topic_tags:   {:?}", out.topic_tags);
    println!(
        "verbatim_highlights ({} kept after grounding):",
        out.verbatim_highlights.len()
    );
    for (i, highlight) in out.verbatim_highlights.iter().enumerate() {
        println!("  [{i}] {highlight}");
    }
    println!("summary ({} chars):", out.summary.len());
    println!("{}", out.summary);
}

fn archive_dir() -> PathBuf {
    let home = env::var("USERPROFILE")
        .or_else(|_| env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".halluscribe")
}
