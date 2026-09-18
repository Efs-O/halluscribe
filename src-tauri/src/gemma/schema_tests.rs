// HalluScribe - unit tests for the Gemma tool schema and tool-call argument
// parsing across the llama.cpp and Ollama response shapes.

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
fn tool_call_absent_with_long_greek_content_does_not_panic() {
    // Reproduces the sweep-halting panic: when the model returns plain text
    // instead of a tool call, the error preview sliced the content at byte
    // 200. Greek is 2 bytes/char, so byte 200 fell mid-char and panicked,
    // killing the whole sweep thread. The preview must be char-safe.
    let greek = "Δώσε μου την περίληψη της συνομιλίας ".repeat(20); // >200 bytes
    assert!(greek.len() > 200 && !greek.is_char_boundary(200));

    // llama.cpp (/v1/chat/completions) shape.
    let openai = serde_json::json!({ "choices": [ { "message": { "content": greek } } ] });
    assert!(matches!(
        extract_openai_tool_args(&openai),
        Err(GemmaError::BadToolCall(_))
    ));

    // Ollama (/api/chat) shape.
    let ollama = serde_json::json!({ "message": { "content": greek } });
    assert!(matches!(
        extract_ollama_tool_args(&ollama),
        Err(GemmaError::BadToolCall(_))
    ));
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
fn required_summary_tool_choice_requires_the_only_available_tool() {
    let choice = required_summary_tool_choice();

    assert_eq!(choice, "required");
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

#[test]
fn save_session_summary_tool_requires_verbatim_highlights() {
    let tool = save_session_summary_tool();
    let required = tool["function"]["parameters"]["required"]
        .as_array()
        .unwrap();
    assert!(required
        .iter()
        .any(|v| v.as_str() == Some("verbatim_highlights")));
    let desc = tool["function"]["parameters"]["properties"]["verbatim_highlights"]["description"]
        .as_str()
        .unwrap();
    assert!(desc.contains("VERBATIM"), "got: {desc}");
}

#[test]
fn parse_tool_args_reads_verbatim_highlights() {
    let args = serde_json::json!({
        "title": "T",
        "summary": "S",
        "session_type": "exploration",
        "error_tags": [],
        "topic_tags": [],
        "verbatim_highlights": ["| task | who wins |", "gemma beats me"]
    });
    let out = parse_tool_args(&args).unwrap();
    assert_eq!(
        out.verbatim_highlights,
        vec!["| task | who wins |", "gemma beats me"]
    );
}

#[test]
fn parse_tool_args_absent_verbatim_highlights_is_empty_not_error() {
    // Older/looser model responses simply omit the field; that must degrade
    // to "no highlights", never to a failed summary.
    let args = serde_json::json!({
        "title": "T",
        "summary": "S",
        "session_type": "exploration",
        "error_tags": [],
        "topic_tags": []
    });
    assert!(parse_tool_args(&args)
        .unwrap()
        .verbatim_highlights
        .is_empty());
}
