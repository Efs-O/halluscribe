// HalluScribe - hierarchical summarisation for oversized session transcripts.
// Plans only across complete preprocessor/message units, then makes sequential
// partial tool calls and one normal archive-summary synthesis call.

use super::{GemmaError, GemmaOutput, SweepSession};
use crate::readers::ChatProvider;
use serde_json::Value;

const PROMPT_HEADROOM_TOKENS: u32 = 4_096;
const PARTIAL_MAX_TOKENS: u32 = 4_096;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlannedChunk {
    text: String,
    first_unit: usize,
    last_unit: usize,
}

/// Session transcripts contain paths, ids, command lines, punctuation, and
/// code-like fragments that tokenise much more densely than normal prose. Two
/// characters per token is deliberately conservative; a live Qwen smoke test
/// measured a 45K four-char estimate as a 75K-token request.
pub(crate) fn estimated_tokens(text: &str) -> u32 {
    text.chars().count().div_ceil(2) as u32
}

pub(crate) fn needs_chunking(transcript: &str, ctx_size: u32, max_tokens: u32) -> bool {
    estimated_tokens(transcript) > input_budget(ctx_size, max_tokens)
}

/// Reserve room for the completion and the fixed system/tool prompt. The
/// remaining configured context is available to complete conversation turns;
/// this must scale with `ctx_size` so a deliberately larger context can admit
/// one large, unsplittable turn.
fn input_budget(ctx_size: u32, max_tokens: u32) -> u32 {
    ctx_size.saturating_sub(max_tokens.saturating_add(PROMPT_HEADROOM_TOKENS))
}

pub(crate) fn summarize(
    session: &SweepSession,
    provider: &ChatProvider,
    units: &[String],
    ctx_size: u32,
    max_tokens: u32,
    cancelled: &dyn Fn() -> bool,
    on_chunk: &mut dyn FnMut(usize, usize),
) -> Result<GemmaOutput, GemmaError> {
    let input_budget = input_budget(ctx_size, max_tokens);
    let chunks = plan_chunks(units, input_budget)?;
    let total = chunks.len();
    let mut partials = Vec::with_capacity(total);

    for (index, chunk) in chunks.iter().enumerate() {
        if cancelled() {
            return Err(GemmaError::Cancelled);
        }
        on_chunk(index + 1, total);
        let content = format!(
            "Session chunk {}/{}. Preserve this ordinal and cite only facts visible in this chunk.\n\n{}",
            index + 1,
            total,
            chunk.text
        );
        let value = session.infer_tool(
            PARTIAL_MAX_TOKENS,
            partial_system_prompt(),
            &content,
            &partial_summary_tool(),
        )?;
        let facts = parse_partial(&value, index + 1)?;
        partials.push(serde_json::json!({
            "chunk": index + 1,
            "turn_range": format!("{}-{}", chunk.first_unit, chunk.last_unit),
            "facts": facts,
        }));
    }

    if cancelled() {
        return Err(GemmaError::Cancelled);
    }
    let synthesis_input = render_partials(&partials);
    session.infer(max_tokens, provider, &synthesis_input)
}

fn plan_chunks(units: &[String], max_tokens: u32) -> Result<Vec<PlannedChunk>, GemmaError> {
    if units.is_empty() {
        return Err(GemmaError::EmptyResponse);
    }
    if max_tokens == 0 {
        return Err(GemmaError::BadToolCall(
            "large-session input budget is zero".to_string(),
        ));
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut first_unit = 1usize;
    for (index, unit) in units.iter().enumerate() {
        let tokens = estimated_tokens(unit);
        if tokens > max_tokens {
            return Err(GemmaError::BadToolCall(format!(
                "one complete conversation/tool turn is estimated at {tokens} tokens, above the {max_tokens}-token chunk budget; it was not split"
            )));
        }
        let separator = if !current.is_empty() { "\n" } else { "" };
        if !current.is_empty()
            && estimated_tokens(&current)
                .saturating_add(tokens)
                .saturating_add(1)
                > max_tokens
        {
            chunks.push(PlannedChunk {
                text: std::mem::take(&mut current),
                first_unit,
                last_unit: index,
            });
            first_unit = index + 1;
        }
        if !current.is_empty() {
            current.push_str(separator);
        }
        current.push_str(unit);
    }
    if !current.is_empty() {
        chunks.push(PlannedChunk {
            text: current,
            first_unit,
            last_unit: units.len(),
        });
    }
    Ok(chunks)
}

fn partial_system_prompt() -> &'static str {
    "You are preparing a factual partial record of one chronological chunk of an AI session. \
     Call save_session_partial with facts grounded only in this chunk. Preserve uncertainty. \
     Cover goals, actions, files, tests, decisions, risks, and open issues when present; retain \
     any comparison table, benchmark, rating, verdict, or decision matrix verbatim in \
     verbatim_highlights. \
     Do not create a final session title or infer facts from other chunks. Keep every list to at \
     most five concise entries, each entry below 160 characters, and return only the tool call."
}

fn partial_summary_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "save_session_partial",
            "description": "Save factual evidence from one chronological session chunk.",
            "parameters": {
                "type": "object",
                "properties": {
                    "goals": {"type": "array", "maxItems": 5, "items": {"type": "string", "maxLength": 160}},
                    "actions": {"type": "array", "maxItems": 5, "items": {"type": "string", "maxLength": 160}},
                    "files_changed": {"type": "array", "maxItems": 5, "items": {"type": "string", "maxLength": 160}},
                    "tests": {"type": "array", "maxItems": 5, "items": {"type": "string", "maxLength": 160}},
                    "decisions": {"type": "array", "maxItems": 5, "items": {"type": "string", "maxLength": 160}},
                    "risks": {"type": "array", "maxItems": 5, "items": {"type": "string", "maxLength": 160}},
                    "open_issues": {"type": "array", "maxItems": 5, "items": {"type": "string", "maxLength": 160}},
                    "evidence": {"type": "array", "maxItems": 5, "items": {"type": "string", "maxLength": 160}},
                    "verbatim_highlights": {"type": "array", "maxItems": 2, "items": {"type": "string", "maxLength": 160}}
                },
                "required": ["goals", "actions", "files_changed", "tests", "decisions", "risks", "open_issues", "evidence", "verbatim_highlights"]
            }
        }
    })
}

fn parse_partial(value: &Value, ordinal: usize) -> Result<Value, GemmaError> {
    let object = value.as_object().ok_or_else(|| {
        GemmaError::BadToolCall(format!("partial summary {ordinal} was not an object"))
    })?;
    for key in [
        "goals",
        "actions",
        "files_changed",
        "tests",
        "decisions",
        "risks",
        "open_issues",
        "evidence",
        "verbatim_highlights",
    ] {
        if !object.get(key).is_some_and(Value::is_array) {
            return Err(GemmaError::BadToolCall(format!(
                "partial summary {ordinal} is missing array field {key}"
            )));
        }
    }
    Ok(value.clone())
}

fn render_partials(partials: &[Value]) -> String {
    let payload = serde_json::to_string_pretty(partials).unwrap_or_default();
    format!(
        "The following are ordered factual partial summaries of one session. Synthesize them into \
         the normal session archive summary. Preserve chronology, reconcile duplicates, and mark \
         unresolved or contradictory details instead of guessing.\n\n{payload}"
    )
}

#[cfg(test)]
mod tests {
    use super::{estimated_tokens, input_budget, needs_chunking, plan_chunks, summarize};
    use crate::gemma::start_sweep_session;
    use crate::llama_tuning;
    use crate::preprocessor::preprocess_session_units;
    use crate::readers::ChatProvider;
    use crate::scanner::ToolSource;
    use crate::settings;
    use std::path::PathBuf;

    #[test]
    fn exact_fit_is_one_chunk() {
        let units = vec!["abcd".repeat(100), "efgh".repeat(100)];
        assert_eq!(plan_chunks(&units, 401).unwrap().len(), 1);
    }

    #[test]
    fn chunks_keep_complete_units_and_order() {
        let units = vec![
            "first".repeat(100),
            "second".repeat(100),
            "third".repeat(100),
        ];
        let chunks = plan_chunks(&units, 300).unwrap();
        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].text.starts_with("first"));
        assert!(chunks[1].text.starts_with("second"));
        assert!(chunks[2].text.starts_with("third"));
        assert_eq!((chunks[1].first_unit, chunks[1].last_unit), (2, 2));
    }

    #[test]
    fn oversized_single_unit_is_rejected_without_splitting() {
        let error = plan_chunks(&["x".repeat(1_000)], 10).unwrap_err();
        assert!(error.to_string().contains("was not split"));
    }

    #[test]
    fn admission_reserves_output_and_prompt_headroom() {
        let transcript = "x".repeat(200_000);
        assert!(needs_chunking(&transcript, 58_000, 8_192));
        assert_eq!(estimated_tokens("abcd"), 2);
    }

    #[test]
    fn chunk_budget_scales_with_the_configured_context() {
        let budget = input_budget(128_000, 8_192);
        assert_eq!(budget, 115_712);
        assert!(plan_chunks(&["x".repeat(112_192)], budget).is_ok());
    }

    /// Manual end-to-end check of the chunked path against one real source.
    /// It never creates, moves, or indexes archive files: it only preprocesses
    /// the supplied JSONL and calls the warm inference session.
    ///
    /// HALLUSCRIBE_LARGE_SMOKE_ARCHIVE="C:\\Users\\you\\.halluscribe" \
    /// HALLUSCRIBE_LARGE_SMOKE_JSONL="C:\\path\\session.jsonl" \
    /// HALLUSCRIBE_LARGE_SMOKE_TOOL="claude" \
    /// HALLUSCRIBE_LARGE_SMOKE_REPEAT="4" \
    /// cargo test --release --manifest-path src-tauri/Cargo.toml \
    ///   live_large_session_smoke -- --ignored --nocapture
    #[test]
    #[ignore = "manual live Qwen check; set HALLUSCRIBE_LARGE_SMOKE_ARCHIVE and HALLUSCRIBE_LARGE_SMOKE_JSONL"]
    fn live_large_session_smoke() {
        let archive_dir = PathBuf::from(
            std::env::var("HALLUSCRIBE_LARGE_SMOKE_ARCHIVE")
                .expect("HALLUSCRIBE_LARGE_SMOKE_ARCHIVE is required"),
        );
        let source = PathBuf::from(
            std::env::var("HALLUSCRIBE_LARGE_SMOKE_JSONL")
                .expect("HALLUSCRIBE_LARGE_SMOKE_JSONL is required"),
        );
        assert!(source.exists(), "smoke JSONL missing: {}", source.display());
        let (tool, provider) = match std::env::var("HALLUSCRIBE_LARGE_SMOKE_TOOL")
            .unwrap_or_else(|_| "codex".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "claude" => (ToolSource::ClaudeCode, ChatProvider::ClaudeCode),
            "codex" => (ToolSource::Codex, ChatProvider::Codex),
            "forge" => (ToolSource::Forge, ChatProvider::Forge),
            other => panic!("unsupported HALLUSCRIBE_LARGE_SMOKE_TOOL: {other}"),
        };

        llama_tuning::set_host_root(archive_dir.clone());
        let config = settings::load_settings(&archive_dir).expect("load smoke settings");
        let backend = config
            .to_inference_backend()
            .expect("configured inference backend is required");
        let units = preprocess_session_units(&source, &tool).expect("preprocess smoke JSONL");
        let repeat = std::env::var("HALLUSCRIBE_LARGE_SMOKE_REPEAT")
            .ok()
            .map(|value| {
                value
                    .parse::<usize>()
                    .expect("smoke repeat must be a positive integer")
            })
            .unwrap_or(1);
        assert!(repeat > 0, "smoke repeat must be above zero");
        let smoke_units: Vec<String> = (0..repeat)
            .flat_map(|_| units.units.iter().cloned())
            .collect();
        let transcript = smoke_units.join("\n");
        println!(
            "preprocessed {} real units; smoke uses {} repeats / {} units, {} chars, ~{} tokens",
            units.units.len(),
            repeat,
            smoke_units.len(),
            transcript.chars().count(),
            estimated_tokens(&transcript)
        );
        assert!(
            needs_chunking(&transcript, config.ctx_size, config.max_tokens),
            "selected transcript fits the active context; choose a larger source"
        );

        let session = start_sweep_session(&backend, config.ctx_size).expect("start Qwen session");
        let started = std::time::Instant::now();
        let output = summarize(
            &session,
            &provider,
            &smoke_units,
            config.ctx_size,
            config.max_tokens,
            &|| false,
            &mut |chunk, total| println!("partial chunk {chunk} of {total}"),
        )
        .expect("chunked summary should succeed");
        println!(
            "chunked smoke succeeded in {:.1}s: title={:?}, summary_chars={}",
            started.elapsed().as_secs_f64(),
            output.title,
            output.summary.chars().count()
        );
        assert!(!output.title.is_empty() || !output.summary.is_empty());
    }
}

#[cfg(test)]
#[path = "large_session_live_tests.rs"]
mod live_tests;
