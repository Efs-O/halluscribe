// HalluScribe - manual discovery checks for real oversized source transcripts.
// These tests only read source JSONL files and never start a model or alter an
// archive. They live outside large_session.rs to retain the 350-LOC limit.

use super::estimated_tokens;
use crate::archive;
use crate::preprocessor::preprocess_session_units;
use crate::scanner::ToolSource;
use std::path::PathBuf;

#[test]
#[ignore = "manual source inventory; set HALLUSCRIBE_LARGE_SMOKE_ARCHIVE"]
fn find_clean_context_candidate() {
    let archive_dir = PathBuf::from(
        std::env::var("HALLUSCRIBE_LARGE_SMOKE_ARCHIVE")
            .expect("HALLUSCRIBE_LARGE_SMOKE_ARCHIVE is required"),
    );
    let minimum = std::env::var("HALLUSCRIBE_CANDIDATE_MIN_TOKENS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(200_000);
    let maximum = std::env::var("HALLUSCRIBE_CANDIDATE_MAX_TOKENS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(250_000);
    let mut largest = (0u32, String::new());
    let mut matches = Vec::new();

    for entry in archive::read_sessions(&archive_dir) {
        let tool = match entry.provider.as_str() {
            "claude_code" => ToolSource::ClaudeCode,
            "codex" => ToolSource::Codex,
            "forge" => ToolSource::Forge,
            _ => continue,
        };
        let source = PathBuf::from(&entry.source_jsonl);
        if !source.exists() {
            continue;
        }
        let Ok(cleaned) = preprocess_session_units(&source, &tool) else {
            continue;
        };
        let tokens = estimated_tokens(&cleaned.render());
        if tokens > largest.0 {
            largest = (tokens, source.display().to_string());
        }
        if (minimum..=maximum).contains(&tokens) {
            matches.push((tokens, source.display().to_string()));
        }
    }

    matches.sort_by_key(|(tokens, _)| *tokens);
    for (tokens, path) in &matches {
        println!("candidate ~{tokens} clean tokens: {path}");
    }
    println!(
        "largest available coding source: ~{} clean tokens: {}",
        largest.0, largest.1
    );
    assert!(
        !matches.is_empty(),
        "no source falls in the requested {minimum}–{maximum} clean-token range"
    );
}
