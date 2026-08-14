// HalluScribe - semantic retrieval smoke check against the real archive.
// Rebuilds embeddings if needed, then runs a semantic query and prints top hits.
//
// Usage:
//   LLAMA_SERVER_BIN=<path> EMBEDDING_MODEL_PATH=<path> QUERY="..." \
//   cargo run --example semantic_smoke_check
//
// Optional:
//   REBUILD_ALL=1    Rebuild the full archive after the single-session check.
//   EXPECTED_SESSION_ID=<id>         Fail unless this session appears in top matches.
//   EXPECTED_TITLE_SUBSTRING=<text>  Fail unless a top match title contains this text.
//   MAX_EXPECTED_RANK=<n>            Default 5. Expected match must land within this rank.

use app_lib::{retrieval, settings};
use std::env;
use std::path::PathBuf;

fn main() {
    let archive_dir = archive_dir();
    let mut config = settings::load_settings(&archive_dir).expect("failed to load settings.json");

    if let Ok(path) = env::var("LLAMA_SERVER_BIN") {
        config.llama_server_bin = path;
    }
    if let Ok(path) = env::var("EMBEDDING_MODEL_PATH") {
        config.embedding_model_path = path;
    }

    let query = env::var("QUERY")
        .unwrap_or_else(|_| "Find the session where I fixed a port in use bug".to_string());
    let rebuild_all = env::var("REBUILD_ALL")
        .ok()
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
    let expected_session_id = env::var("EXPECTED_SESSION_ID").ok();
    let expected_title_substring = env::var("EXPECTED_TITLE_SUBSTRING")
        .ok()
        .map(|text| text.to_lowercase());
    let max_expected_rank = env::var("MAX_EXPECTED_RANK")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(5);

    println!("=== HalluScribe Semantic Smoke Check ===");
    println!("Archive dir: {}", archive_dir.display());
    println!("Query: {query}");
    if let Some(expected) = &expected_session_id {
        println!("Expecting session id within top {max_expected_rank}: {expected}");
    }
    if let Some(expected) = &expected_title_substring {
        println!("Expecting title substring within top {max_expected_rank}: {expected}");
    }

    if !retrieval::embedding_runtime_ready(&config) {
        eprintln!("Embedding runtime is not configured.");
        std::process::exit(1);
    }

    let sessions = app_lib::archive::read_sessions(&archive_dir);
    if let Some(first) = sessions.first() {
        match retrieval::index_session(&archive_dir, &config, &first.id) {
            Ok(()) => println!("Single-session index check: ok ({})", first.id),
            Err(error) => {
                eprintln!(
                    "Single-session index check failed ({}): {}",
                    first.id, error
                );
                std::process::exit(1);
            }
        }
    }

    if rebuild_all {
        let rebuild = retrieval::rebuild_embeddings_with_progress(
            &archive_dir,
            &config,
            |done, total, result| {
                println!(
                    "Progress: {done}/{total} | indexed={} skipped={} failed={}",
                    result.indexed, result.skipped, result.failed
                );
            },
        )
        .unwrap_or_else(|error| {
            eprintln!("Rebuild failed: {error}");
            std::process::exit(1);
        });
        println!(
            "Rebuild: indexed={} skipped={} failed={}",
            rebuild.indexed, rebuild.skipped, rebuild.failed
        );
    } else {
        println!("Full rebuild skipped. Set REBUILD_ALL=1 to index the entire archive.");
    }

    match retrieval::semantic_search(
        &archive_dir,
        &config,
        &query,
        max_expected_rank.max(5),
        None,
    ) {
        Ok(results) => {
            if results.is_empty() {
                println!("No semantic matches.");
                if expected_session_id.is_some() || expected_title_substring.is_some() {
                    eprintln!("Expected a known semantic match, but no results were returned.");
                    std::process::exit(1);
                }
                return;
            }
            println!("\nTop semantic matches:");
            for (idx, result) in results.iter().enumerate() {
                println!(
                    "{}. {:.4} | {} | {} | {}",
                    idx + 1,
                    result.score,
                    result.entry.date,
                    result.entry.title,
                    result.entry.id
                );
            }
            if let Some(expected_id) = expected_session_id {
                let found_rank = results
                    .iter()
                    .position(|result| result.entry.id == expected_id)
                    .map(|idx| idx + 1);
                if let Some(rank) = found_rank {
                    println!("Expectation met: session id matched at rank {rank}.");
                } else {
                    eprintln!(
                        "Expected session id '{expected_id}' was not found within top {max_expected_rank}."
                    );
                    std::process::exit(1);
                }
            }
            if let Some(expected_title) = expected_title_substring {
                let found_rank = results
                    .iter()
                    .position(|result| result.entry.title.to_lowercase().contains(&expected_title))
                    .map(|idx| idx + 1);
                if let Some(rank) = found_rank {
                    println!("Expectation met: title substring matched at rank {rank}.");
                } else {
                    eprintln!(
                        "Expected title substring '{expected_title}' was not found within top {max_expected_rank}."
                    );
                    std::process::exit(1);
                }
            }
        }
        Err(error) => {
            eprintln!("Semantic search failed: {error}");
            std::process::exit(1);
        }
    }
}

fn archive_dir() -> PathBuf {
    let home = env::var("USERPROFILE")
        .or_else(|_| env::var("HOME"))
        .expect("missing home dir");
    PathBuf::from(home).join(".halluscribe")
}
