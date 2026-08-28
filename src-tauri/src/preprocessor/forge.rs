// HalluScribe - Forge VS Code extension session preprocessing.
// Parses JSONL written by the Forge extension SessionLogger.
// Line types:
//   session_start  — metadata header, skipped (no role)
//   message lines  — {"role":"user"|"assistant","content":"...","timestamp_ms":...}
//   tool call      — {"role":"assistant","content":null,"tool_calls":[{"name":"...","input":{...}}]}
//   usage          — {"type":"usage","output_tokens":N,...} (Forge >= 0.13.17)

use super::shared::truncate_text;
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

        // Skip metadata header lines (no role field)
        let Some(role) = v.get("role").and_then(Value::as_str) else {
            continue;
        };

        match role {
            "user" | "assistant" => {}
            _ => continue, // skip tool/system roles
        }

        // Tool call lines
        if let Some(tool_calls) = v.get("tool_calls").and_then(Value::as_array) {
            for tc in tool_calls {
                let name = tc.get("name").and_then(Value::as_str).unwrap_or("?");
                let detail = preferred_arg(tc.get("input").unwrap_or(&Value::Null));
                if detail.is_empty() {
                    out.push(format!("[Tool: {name}]"));
                } else {
                    out.push(format!("[Tool: {name}] {}", truncate_text(&detail, 160)));
                }
            }
            continue;
        }

        // Text content lines
        let content_str = match v.get("content") {
            Some(Value::String(s)) => s.trim().to_string(),
            _ => continue,
        };
        if content_str.is_empty() {
            continue;
        }

        let label = if role == "user" {
            "[User]"
        } else {
            "[Assistant]"
        };
        out.push(format!("{label}\n{}", truncate_text(&content_str, 2000)));

        // Include reasoning block if present
        if let Some(reasoning) = v.get("reasoning").and_then(Value::as_str) {
            let r = reasoning.trim();
            if !r.is_empty() {
                out.push(format!("[Thinking]\n{}", truncate_text(r, 500)));
            }
        }
    }

    out
}

/// Forge 0.13.17 and later writes cumulative `usage` lines; anything older has
/// no counters at all and falls back to the character estimate.
///
/// The largest total is taken rather than the last. The totals are cumulative
/// and normally only climb, but they live on the conversation and a reader
/// should not report a smaller number than it has already seen if one ever
/// resets mid-file.
pub(super) fn token_count(content: &str) -> TokenCount {
    let mut reported: Option<u64> = None;
    let mut generated_chars = 0usize;

    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) == Some("usage") {
            if let Some(output) = v.get("output_tokens").and_then(Value::as_u64) {
                reported = Some(reported.map_or(output, |seen: u64| seen.max(output)));
            }
            continue;
        }
        if v.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        // Reasoning counts as generated: it is the bulk of a thinking model's
        // output, and a tool-call turn carries reasoning with no content at all.
        for key in ["content", "reasoning"] {
            generated_chars += v.get(key).and_then(Value::as_str).map_or(0, str::len);
        }
        // So do tool arguments — an edit_file call carries the whole new file.
        if let Some(calls) = v.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                generated_chars += super::claude::emitted_json_len(call.get("input"));
            }
        }
    }

    TokenCount::from_usage_or_chars(reported, generated_chars)
}

fn preferred_arg(input: &Value) -> String {
    for key in [
        "path",
        "command",
        "query",
        "pattern",
        "search_term",
        "target_directory",
    ] {
        if let Some(v) = input.get(key).and_then(Value::as_str) {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_session_start_skipped() {
        let line = r#"{"type":"session_start","session_id":"abc","title":"Chat","model":"gemma4","timestamp_ms":1}"#;
        assert!(preprocess(line).is_empty());
    }

    #[test]
    fn forge_user_message_extracted() {
        let line = r#"{"role":"user","content":"Fix the build","timestamp_ms":1}"#;
        let r = preprocess(line);
        assert!(r.contains("[User]"));
        assert!(r.contains("Fix the build"));
    }

    #[test]
    fn forge_assistant_message_extracted() {
        let line =
            r#"{"role":"assistant","content":"I will fix it","timestamp_ms":1,"model":"gemma4"}"#;
        let r = preprocess(line);
        assert!(r.contains("[Assistant]"));
        assert!(r.contains("I will fix it"));
    }

    #[test]
    fn forge_tool_call_formatted() {
        let line = r#"{"role":"assistant","content":null,"tool_calls":[{"name":"read_file","input":{"path":"src/main.ts"}}],"timestamp_ms":1}"#;
        let r = preprocess(line);
        assert!(r.contains("[Tool: read_file] src/main.ts"));
    }

    #[test]
    fn forge_reasoning_included() {
        let line = r#"{"role":"assistant","content":"Done.","reasoning":"I checked the file first.","timestamp_ms":1}"#;
        let r = preprocess(line);
        assert!(r.contains("[Thinking]"));
        assert!(r.contains("I checked the file first."));
    }

    #[test]
    fn forge_system_role_skipped() {
        let line = r#"{"role":"system","content":"You are a coding assistant.","timestamp_ms":1}"#;
        assert!(preprocess(line).is_empty());
    }

    #[test]
    fn forge_invalid_lines_skipped() {
        assert!(preprocess("not-json\n{\"broken\":true}").is_empty());
    }
}
