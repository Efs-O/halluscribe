// HalluScribe - manual live A/B: does raising the summary word target actually
// change how many words Gemma delivers? Measured motivation: across a 200-session
// sample of the live archive, summaries average ~322 words and top out at ~723
// against a 900-word ask, i.e. the model fills ~30% of the budget it is given.
// If the ask is not the binding constraint, raising MAX_SUMMARY_WORDS is
// cosmetic and the structural fix (highlights outside the prose budget) is the
// only real lever. This measures that rather than assuming it.
//
// Runs the SAME transcript through the SAME shipped prompt builder at several
// word targets, on one warm server, and prints delivered word counts. Never runs
// in CI: gated on an env var AND #[ignore], matching the live-probe pattern in
// briefing/retrieval_probe_tests.rs.
//
//   HALLUSCRIBE_AB_ARCHIVE="C:\Users\<you>\.halluscribe" \
//   HALLUSCRIBE_AB_JSONL="<path1>;<path2>;<path3>" \
//     cargo test --release --manifest-path src-tauri/Cargo.toml \
//     summary_word_budget_ab -- --ignored --nocapture
//
// Reads llama-server bin/model/gpu/ctx from that archive's settings.json.
// Uses its own port so it cannot collide with a running HalluScribe.

use super::schema::save_session_summary_tool;
use super::{
    build_system_prompt, start_tool_session, InferenceBackend, MAX_SUMMARY_WORDS, MIN_SUMMARY_WORDS,
};
use crate::preprocessor::preprocess_session;
use crate::readers::ChatProvider;
use crate::scanner::ToolSource;
use crate::settings;
use std::path::PathBuf;

/// Port deliberately away from the app's (8091/8092) and the retrieval probe's
/// (8099) so this never fights a running HalluScribe for a port or GPU slot.
const AB_PORT: u16 = 8098;

/// Word targets to compare. `900` is the shipped ceiling; the rest probe whether
/// asking for more actually yields more.
const TARGETS: [usize; 3] = [900, 1500, 2000];

/// Repeats per (session, target) cell. Temperature is 0.2, not 0, so a single
/// sample per cell cannot distinguish a real effect from run-to-run noise.
const REPEATS: usize = 2;

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

#[test]
#[ignore = "manual live A/B; set HALLUSCRIBE_AB_ARCHIVE and HALLUSCRIBE_AB_JSONL"]
fn summary_word_budget_ab() {
    let Ok(dir) = std::env::var("HALLUSCRIBE_AB_ARCHIVE") else {
        eprintln!("HALLUSCRIBE_AB_ARCHIVE not set; skipping");
        return;
    };
    let Ok(jsonl_list) = std::env::var("HALLUSCRIBE_AB_JSONL") else {
        eprintln!("HALLUSCRIBE_AB_JSONL not set; skipping");
        return;
    };

    let archive_dir = PathBuf::from(&dir);
    let cfg = settings::load_settings(&archive_dir);
    let bin = PathBuf::from(&cfg.llama_server_bin);
    let model = PathBuf::from(&cfg.gemma_model_path);
    assert!(bin.exists(), "llama_server_bin missing: {}", bin.display());
    assert!(
        model.exists(),
        "gemma_model_path missing: {}",
        model.display()
    );

    // Preprocess every transcript up front so a bad path fails before the model
    // is loaded rather than 20 minutes into the run.
    let mut cases: Vec<(String, String)> = Vec::new();
    for raw in jsonl_list.split(';').filter(|s| !s.trim().is_empty()) {
        let path = PathBuf::from(raw.trim());
        assert!(path.exists(), "jsonl missing: {}", path.display());
        let transcript = preprocess_session(&path, &ToolSource::ClaudeCode)
            .unwrap_or_else(|e| panic!("preprocess failed for {}: {e}", path.display()));
        let label = path
            .file_stem()
            .map(|s| s.to_string_lossy().chars().take(8).collect::<String>())
            .unwrap_or_else(|| "?".into());
        println!(
            "case {label}: {} transcript chars (~{} est. tokens)",
            transcript.chars().count(),
            transcript.chars().count() / 4
        );
        cases.push((label, transcript));
    }
    assert!(!cases.is_empty(), "no usable transcripts");

    println!(
        "\nshipped clamp: MIN={MIN_SUMMARY_WORDS} MAX={MAX_SUMMARY_WORDS}; \
         targets under test: {TARGETS:?}; repeats: {REPEATS}\n"
    );

    let backend = InferenceBackend::LlamaCpp {
        bin,
        model,
        port: AB_PORT,
        gpu: cfg.gpu_config(),
    };
    let session =
        start_tool_session(&backend, cfg.ctx_size).expect("failed to start llama-server for A/B");
    let tool = save_session_summary_tool();

    // rows: (case, target, repeat, delivered words)
    let mut rows: Vec<(String, usize, usize, usize)> = Vec::new();

    for (label, transcript) in &cases {
        for target in TARGETS {
            for repeat in 0..REPEATS {
                // Use the SHIPPED prompt builder so this measures the real
                // prompt, not a paraphrase of it. `build_system_prompt` derives
                // the target from transcript size, so call the inner builder
                // with the target we want to test.
                let prompt = super::coding_system_prompt(target);
                let started = std::time::Instant::now();
                let args = match session.call_tool(&prompt, transcript, &tool, cfg.max_tokens) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("  {label} target={target} rep={repeat}: ERROR {e}");
                        continue;
                    }
                };
                let summary = args["summary"].as_str().unwrap_or("");
                let words = word_count(summary);
                println!(
                    "  {label} target={target:<5} rep={repeat} -> {words:>5} words \
                     ({:>3}% of ask, {:.0}s)",
                    words * 100 / target.max(1),
                    started.elapsed().as_secs_f64()
                );
                rows.push((label.clone(), target, repeat, words));
            }
        }
    }

    println!("\n=== delivered words by target ===");
    println!("{:<10} {:>8} {:>8} {:>8}", "case", "900", "1500", "2000");
    for (label, _) in &cases {
        let mean_for = |target: usize| -> f64 {
            let vals: Vec<usize> = rows
                .iter()
                .filter(|(l, t, _, _)| l == label && *t == target)
                .map(|(_, _, _, w)| *w)
                .collect();
            if vals.is_empty() {
                return f64::NAN;
            }
            vals.iter().sum::<usize>() as f64 / vals.len() as f64
        };
        println!(
            "{label:<10} {:>8.0} {:>8.0} {:>8.0}",
            mean_for(900),
            mean_for(1500),
            mean_for(2000)
        );
    }

    for target in TARGETS {
        let vals: Vec<usize> = rows
            .iter()
            .filter(|(_, t, _, _)| *t == target)
            .map(|(_, _, _, w)| *w)
            .collect();
        if vals.is_empty() {
            continue;
        }
        let mean = vals.iter().sum::<usize>() as f64 / vals.len() as f64;
        let spread = vals.iter().max().unwrap() - vals.iter().min().unwrap();
        println!(
            "target={target:<5} n={:<3} mean={mean:>6.0} words  fill={:>3.0}%  spread={spread}",
            vals.len(),
            mean * 100.0 / target as f64
        );
    }

    // Keep `build_system_prompt` referenced so the A/B stays honest about which
    // prompt path production uses if that indirection is ever changed.
    let _ = build_system_prompt(&ChatProvider::ClaudeCode, "probe");

    println!("\n(A/B server unloaded)");
}
