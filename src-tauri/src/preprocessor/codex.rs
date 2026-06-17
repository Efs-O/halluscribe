// HalluScribe - Codex session preprocessing.

use super::shared::{head_tail, truncate_text};
use serde_json::Value;

pub(super) fn preprocess(content: &str) -> String {
    let mut out = String::with_capacity(32 * 1024);
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let Some(payload) = v.get("payload") else {
            continue;
        };
        let ev = payload.get("type").and_then(Value::as_str).unwrap_or("");
        let frag = match ev {
            "user_message" => format_user_message(payload),
            "assistant_message" | "response_output_text" => format_assistant_message(payload),
            "tool_call" | "function_call" => Some(format_tool_call(payload)),
            "tool_output" | "function_output" => format_tool_output(payload),
            _ => None,
        };
        if let Some(fragment) = frag {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&fragment);
        }
    }
    out
}

fn format_user_message(payload: &Value) -> Option<String> {
    let msg = payload.get("message").and_then(Value::as_str).unwrap_or("");
    let t = strip_prefix(msg).trim();
    if t.is_empty() {
        None
    } else {
        Some(format!("[User]\n{}", truncate_text(t, 2000)))
    }
}

fn format_assistant_message(payload: &Value) -> Option<String> {
    let t = payload
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| payload.get("message").and_then(Value::as_str))
        .unwrap_or("")
        .trim();
    if t.is_empty() {
        None
    } else {
        Some(format!("[Assistant]\n{}", truncate_text(t, 2000)))
    }
}

fn format_tool_output(payload: &Value) -> Option<String> {
    let output = payload
        .get("output")
        .and_then(Value::as_str)
        .or_else(|| payload.get("text").and_then(Value::as_str))
        .unwrap_or("");
    if output.is_empty() {
        None
    } else {
        Some(format!("[Tool result]\n{}", head_tail(output, 10)))
    }
}

fn format_tool_call(payload: &Value) -> String {
    let name = payload.get("name").and_then(Value::as_str).unwrap_or("?");
    let args = payload.get("arguments").unwrap_or(&Value::Null);
    let detail = preferred_arg(args);
    if detail.is_empty() {
        format!("[Tool: {name}]")
    } else {
        format!("[Tool: {name}] {}", truncate_text(detail, 160))
    }
}

fn preferred_arg(args: &Value) -> &str {
    for key in ["command", "file_path", "pattern"] {
        if let Some(value) = args.get(key).and_then(Value::as_str) {
            let value = value.trim();
            if !value.is_empty() {
                return value;
            }
        }
    }
    ""
}

fn strip_prefix(msg: &str) -> &str {
    const MARKER: &str = "## My request for Codex:\n";
    msg.find(MARKER)
        .map_or(msg, |pos| &msg[pos + MARKER.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_strip_prefix_removes_marker() {
        let msg = "context\n## My request for Codex:\nactual request";
        assert_eq!(strip_prefix(msg), "actual request");
    }

    #[test]
    fn codex_strip_prefix_no_marker_passthrough() {
        assert_eq!(strip_prefix("direct message"), "direct message");
    }

    #[test]
    fn codex_user_message_extracted() {
        let line = r#"{"type":"event_msg","payload":{"type":"user_message","message":"Fix the auth bug"}}"#;
        let r = preprocess(line);
        assert!(r.contains("[User]") && r.contains("Fix the auth bug"));
    }

    #[test]
    fn codex_tool_call_includes_preferred_arg() {
        let line = r#"{"type":"event_msg","payload":{"type":"tool_call","name":"Bash","arguments":{"command":"cargo test --all"}}}"#;
        let r = preprocess(line);
        assert!(r.contains("[Tool: Bash] cargo test --all"));
    }

    #[test]
    fn codex_token_count_stripped() {
        let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":80000},"model_context_window":128000}}}"#;
        assert!(preprocess(line).is_empty());
    }

    #[test]
    fn codex_session_meta_stripped() {
        let line = r#"{"type":"session_meta","payload":{"id":"abc"}}"#;
        assert!(preprocess(line).is_empty());
    }

    #[test]
    fn codex_codex_prefix_stripped_in_pipeline() {
        let msg = "context\\n## My request for Codex:\\nthe real task";
        let line = format!(
            r#"{{"type":"event_msg","payload":{{"type":"user_message","message":"{msg}"}}}}"#
        );
        let r = preprocess(&line);
        assert!(!r.is_empty());
    }

    #[test]
    fn codex_tool_output_keeps_wider_window() {
        let body = (1..=30)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\\n");
        let line = format!(
            r#"{{"type":"event_msg","payload":{{"type":"tool_output","output":"{body}"}}}}"#
        );
        let r = preprocess(&line);
        assert!(r.contains("line 10"));
        assert!(r.contains("line 21"));
        assert!(!r.contains("line 11"));
    }
}
