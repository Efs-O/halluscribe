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

pub(crate) fn parse_openai_tool_args(value: &Value) -> Result<GemmaOutput, GemmaError> {
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
    let args: Value = serde_json::from_str(args_raw)
        .map_err(|e| GemmaError::BadToolCall(format!("arguments parse error: {e}")))?;
    parse_tool_args(&args)
}

pub(crate) fn parse_ollama_tool_args(value: &Value) -> Result<GemmaOutput, GemmaError> {
    let args = &value["message"]["tool_calls"][0]["function"]["arguments"];
    if args.is_null() {
        let preview = value["message"]["content"].as_str().unwrap_or("");
        return Err(GemmaError::BadToolCall(format!(
            "tool_calls absent; content preview: {}",
            &preview[..preview.len().min(200)]
        )));
    }
    parse_tool_args(args)
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
