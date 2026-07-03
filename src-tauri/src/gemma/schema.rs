// HalluScribe - Gemma tool schema and tool-call argument parsing.

use super::{GemmaError, GemmaOutput, SessionType};
use serde_json::Value;

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
                    }
                },
                "required": ["title", "summary", "session_type", "error_tags", "topic_tags"]
            }
        }
    })
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
                &preview[..preview.len().min(200)]
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
            &preview[..preview.len().min(200)]
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_args(session_type: &str) -> Value {
        serde_json::json!({
            "title": "Fix auth bug",
            "summary": "Fixed a JWT expiry bug in the auth middleware.",
            "session_type": session_type,
            "error_tags": ["JWT", "middleware"],
            "topic_tags": ["auth", "Rust"]
        })
    }

    #[test]
    fn parse_tool_args_debugging() {
        let out = parse_tool_args(&make_args("debugging")).unwrap();
        assert_eq!(out.title, "Fix auth bug");
        assert_eq!(out.session_type, SessionType::Debugging);
        assert_eq!(out.error_tags, vec!["JWT", "middleware"]);
        assert_eq!(out.topic_tags, vec!["auth", "Rust"]);
    }

    #[test]
    fn parse_tool_args_building() {
        let out = parse_tool_args(&make_args("building")).unwrap();
        assert_eq!(out.session_type, SessionType::Building);
    }

    #[test]
    fn parse_tool_args_refactoring() {
        let out = parse_tool_args(&make_args("refactoring")).unwrap();
        assert_eq!(out.session_type, SessionType::Refactoring);
    }

    #[test]
    fn parse_tool_args_unknown_type_defaults_exploration() {
        let out = parse_tool_args(&make_args("unknown_type")).unwrap();
        assert_eq!(out.session_type, SessionType::Exploration);
    }

    #[test]
    fn parse_tool_args_empty_tags_ok() {
        let args = serde_json::json!({
            "title": "T",
            "summary": "S",
            "session_type": "exploration",
            "error_tags": [],
            "topic_tags": []
        });
        let out = parse_tool_args(&args).unwrap();
        assert!(out.error_tags.is_empty());
        assert!(out.topic_tags.is_empty());
    }

    #[test]
    fn parse_tool_args_both_empty_is_err() {
        let args = serde_json::json!({
            "title": "",
            "summary": "",
            "session_type": "exploration",
            "error_tags": [],
            "topic_tags": []
        });
        assert!(matches!(
            parse_tool_args(&args),
            Err(GemmaError::EmptyResponse)
        ));
    }

    #[test]
    fn openai_truncated_arguments_report_max_tokens_hint() {
        let value = serde_json::json!({
            "choices": [{
                "finish_reason": "length",
                "message": {
                    "tool_calls": [{
                        "function": { "arguments": "{\"facts\": [{\"fact\": \"cut off mid-str" }
                    }]
                }
            }]
        });
        let error = extract_openai_tool_args(&value).unwrap_err();
        let msg = error.to_string();
        assert!(msg.contains("arguments parse error"), "got: {msg}");
        assert!(msg.contains("hit max_tokens"), "got: {msg}");
    }

    #[test]
    fn openai_bad_arguments_without_length_have_no_hint() {
        let value = serde_json::json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "tool_calls": [{
                        "function": { "arguments": "not json at all" }
                    }]
                }
            }]
        });
        let msg = extract_openai_tool_args(&value).unwrap_err().to_string();
        assert!(!msg.contains("hit max_tokens"), "got: {msg}");
    }

    #[test]
    fn ollama_missing_tool_call_reports_max_tokens_hint_on_length() {
        let value = serde_json::json!({
            "done_reason": "length",
            "message": { "content": "partial prose output" }
        });
        let msg = extract_ollama_tool_args(&value).unwrap_err().to_string();
        assert!(msg.contains("tool_calls absent"), "got: {msg}");
        assert!(msg.contains("hit max_tokens"), "got: {msg}");
    }

    #[test]
    fn save_session_summary_tool_has_required_fields() {
        let tool = save_session_summary_tool();
        let required = &tool["function"]["parameters"]["required"];
        let fields = [
            "title",
            "summary",
            "session_type",
            "error_tags",
            "topic_tags",
        ];
        for field in fields {
            assert!(
                required
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v.as_str() == Some(field)),
                "missing required field: {field}"
            );
        }
    }

    #[test]
    fn save_session_summary_tool_uses_provider_agnostic_descriptions() {
        let tool = save_session_summary_tool();
        let function = &tool["function"];
        assert_eq!(
            function["description"].as_str(),
            Some("Save a structured summary of the AI conversation session.")
        );
        assert_eq!(
            function["parameters"]["properties"]["summary"]["description"].as_str(),
            Some(
                "Detailed session summary. Follow the system prompt for the appropriate focus and structure. Be specific, concise, and grounded in the transcript."
            )
        );
    }
}
