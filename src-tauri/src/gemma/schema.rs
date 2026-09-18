// HalluScribe - Gemma tool schema and tool-call argument parsing.

use super::{GemmaError, GemmaOutput, SessionType};
use serde_json::Value;

/// A char-boundary-safe preview of raw model text for error messages.
/// Slicing by byte offset (`&s[..200]`) panics when the cut lands inside a
/// multibyte UTF-8 char — e.g. Greek content, where every char is 2 bytes —
/// which would unwind the whole sweep thread instead of surfacing a per-session
/// error. Taking chars is always valid and keeps the preview readable.
fn content_preview(content: &str) -> String {
    content.chars().take(200).collect()
}

pub(crate) fn save_session_summary_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "save_session_summary",
            "description": "Save a structured summary of the AI conversation session.",
            "parameters": {
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Short session title, max 10 words."
                    },
                    "summary": {
                        "type": "string",
                        "description": "Detailed session summary. Follow the system prompt for \
                                        the appropriate focus and structure. Be specific, concise, \
                                        and grounded in the transcript."
                    },
                    "session_type": {
                        "type": "string",
                        "enum": ["debugging", "building", "refactoring", "exploration"],
                        "description": "Primary type of work or exploration in this session."
                    },
                    "error_tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Specific technologies, APIs, filenames, or error types that are explicitly mentioned in the transcript. Do not infer tags."
                    },
                    "topic_tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Topics, libraries, frameworks, filenames, or domain areas explicitly mentioned in the transcript text. Prefer literal tags over inferred ones."
                    },
                    "verbatim_highlights": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Comparison tables, benchmark results, ratings, verdicts, or decision matrices copied VERBATIM from the transcript, each as one string, including column headings and conclusions. Do not paraphrase, summarise, or invent these — anything not present word-for-word in the transcript is discarded. This field is separate from the summary and is not subject to its word budget. Empty array if the transcript contains none."
                    }
                },
                "required": ["title", "summary", "session_type", "error_tags", "topic_tags", "verbatim_highlights"]
            }
        }
    })
}

/// llama.cpp-compatible instruction that requires the summary tool for an
/// unattended archive-sweep request. The sweep supplies exactly one tool, so
/// the string form forces `save_session_summary` without relying on the
/// OpenAI object form that some llama-server builds reject.
pub(crate) fn required_summary_tool_choice() -> Value {
    Value::String("required".to_string())
}

/// Extract and parse the raw tool-call arguments from an OpenAI-shaped
/// (`/v1/chat/completions`) response, without assuming which tool was called.
/// Shared by `parse_openai_tool_args` (session summaries) and the generic
/// `ToolSession` used by the profile distiller, which supplies its own tool
/// schema and interprets the returned `Value` itself.
pub(crate) fn extract_openai_tool_args(value: &Value) -> Result<Value, GemmaError> {
    let args_raw = value["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
        .as_str()
        .ok_or_else(|| {
            let preview = value["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("");
            GemmaError::BadToolCall(format!(
                "tool_calls absent; content preview: {}",
                content_preview(preview)
            ))
        })?;
    serde_json::from_str(args_raw).map_err(|e| {
        let truncated = value["choices"][0]["finish_reason"].as_str() == Some("length");
        let hint = if truncated {
            " (completion hit max_tokens — the tool-call JSON was cut off; raise the per-call token budget)"
        } else {
            ""
        };
        GemmaError::BadToolCall(format!("arguments parse error: {e}{hint}"))
    })
}

pub(crate) fn parse_openai_tool_args(value: &Value) -> Result<GemmaOutput, GemmaError> {
    parse_tool_args(&extract_openai_tool_args(value)?)
}

/// Ollama counterpart to `extract_openai_tool_args`: pulls the raw tool-call
/// arguments out of an `/api/chat` response.
pub(crate) fn extract_ollama_tool_args(value: &Value) -> Result<Value, GemmaError> {
    let args = &value["message"]["tool_calls"][0]["function"]["arguments"];
    if args.is_null() {
        let preview = value["message"]["content"].as_str().unwrap_or("");
        let hint = if value["done_reason"].as_str() == Some("length") {
            " (completion hit max_tokens — the tool call was cut off; raise the per-call token budget)"
        } else {
            ""
        };
        return Err(GemmaError::BadToolCall(format!(
            "tool_calls absent{hint}; content preview: {}",
            content_preview(preview)
        )));
    }
    Ok(args.clone())
}

pub(crate) fn parse_ollama_tool_args(value: &Value) -> Result<GemmaOutput, GemmaError> {
    parse_tool_args(&extract_ollama_tool_args(value)?)
}

fn parse_tool_args(args: &Value) -> Result<GemmaOutput, GemmaError> {
    let str_val = |key: &str| args[key].as_str().unwrap_or("").to_string();
    let tags = |key: &str| -> Vec<String> {
        args[key]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let session_type = match args["session_type"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "debugging" => SessionType::Debugging,
        "building" => SessionType::Building,
        "refactoring" => SessionType::Refactoring,
        _ => SessionType::Exploration,
    };
    let title = str_val("title");
    let summary = str_val("summary");
    if title.is_empty() && summary.is_empty() {
        return Err(GemmaError::EmptyResponse);
    }
    Ok(GemmaOutput {
        title,
        summary,
        session_type,
        error_tags: tags("error_tags"),
        topic_tags: tags("topic_tags"),
        verbatim_highlights: tags("verbatim_highlights"),
    })
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
