// HalluScribe - Codex session preprocessing.

use super::shared::{head_tail, truncate_text};
use crate::tokens::TokenCount;
use serde_json::Value;

#[cfg(test)]
pub(super) fn preprocess(content: &str) -> String {
    preprocess_units(content).join("\n")
}

pub(super) fn preprocess_units(content: &str) -> Vec<String> {
    let mut out = Vec::new();
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
            out.push(fragment);
        }
    }
    out
}

/// Codex reports usage on `token_count` events. `total_token_usage` is already
/// cumulative, so the last one wins; where a session only carries per-turn
/// `last_token_usage` the turns are summed instead.
pub(super) fn token_count(content: &str) -> TokenCount {
    let mut cumulative: Option<u64> = None;
    let mut per_turn_total = 0u64;
    let mut saw_per_turn = false;
    let mut generated_chars = 0usize;

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
        match payload.get("type").and_then(Value::as_str).unwrap_or("") {
            "token_count" => {
                let Some(info) = payload.get("info").filter(|info| !info.is_null()) else {
                    continue;
                };
                if let Some(total) = output_tokens(info.get("total_token_usage")) {
                    cumulative = Some(total);
                } else if let Some(turn) = output_tokens(info.get("last_token_usage")) {
                    per_turn_total += turn;
                    saw_per_turn = true;
                }
            }
            "assistant_message" | "response_output_text" => {
                generated_chars += payload
                    .get("text")
                    .and_then(Value::as_str)
                    .or_else(|| payload.get("message").and_then(Value::as_str))
                    .map_or(0, str::len);
            }
            // The model wrote these arguments; for a coding agent they dwarf
            // the prose it emits alongside them.
            "tool_call" | "function_call" => {
                generated_chars += super::claude::emitted_json_len(
                    payload.get("arguments").or_else(|| payload.get("input")),
                );
            }
            _ => {}
        }
    }

    let reported = cumulative.or_else(|| saw_per_turn.then_some(per_turn_total));
    TokenCount::from_usage_or_chars(reported, generated_chars)
}

fn output_tokens(usage: Option<&Value>) -> Option<u64> {
    usage?.get("output_tokens")?.as_u64()
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
