// HalluScribe - Claude Code session preprocessing.

use super::shared::{extract_content_text, head_tail_5, truncate_text};
use crate::tokens::TokenCount;
use serde_json::Value;
use std::collections::HashMap;

#[cfg(test)]
pub(super) fn preprocess(content: &str) -> String {
    preprocess_units(content).join("\n")
}

pub(super) fn preprocess_units(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut registry: HashMap<String, (String, Value)> = HashMap::new();

    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(msg) = v.get("message") else {
            continue;
        };
        let Some(role) = msg.get("role").and_then(Value::as_str) else {
            continue;
        };
        if role == "system" {
            continue;
        }
        let Some(items) = msg.get("content").and_then(Value::as_array) else {
            continue;
        };

        for item in items {
            register_tool_use(item, &mut registry);
        }

        let label = if role == "user" {
            "[User]"
        } else {
            "[Assistant]"
        };
        let turn = build_turn(items, &registry);
        if !turn.is_empty() {
            out.push(format!("{label}\n{turn}"));
        }
    }
    out
}

/// Every assistant turn carries `message.usage.output_tokens`, which already
/// includes thinking tokens — the sampled session showed 572 of 884 hidden. The
/// sum covers subagent turns too, since those land in the same file.
pub(super) fn token_count(content: &str) -> TokenCount {
    let mut reported: Option<u64> = None;
    let mut generated_chars = 0usize;

    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(msg) = v.get("message") else {
            continue;
        };
        if msg.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if let Some(output) = msg
            .get("usage")
            .and_then(|usage| usage.get("output_tokens"))
            .and_then(Value::as_u64)
        {
            *reported.get_or_insert(0) += output;
        }
        generated_chars += assistant_chars(msg);
    }

    TokenCount::from_usage_or_chars(reported, generated_chars)
}

/// Only what the model emitted. Tool *inputs* count — for a coding agent the
/// file bodies and shell scripts it writes are the bulk of its output, and
/// omitting them understated a measured session by roughly 28x against 6x with
/// them. Tool *results* do not count: those arrive on user-role lines, which
/// never reach here.
fn assistant_chars(msg: &Value) -> usize {
    let Some(items) = msg.get("content").and_then(Value::as_array) else {
        return 0;
    };
    items
        .iter()
        .map(|item| {
            let text: usize = ["text", "thinking"]
                .iter()
                .filter_map(|key| item.get(key).and_then(Value::as_str))
                .map(str::len)
                .sum();
            text + emitted_json_len(item.get("input"))
        })
        .sum()
}

/// Serialized length of a tool-call argument object, as the model emitted it.
pub(super) fn emitted_json_len(input: Option<&Value>) -> usize {
    match input {
        None | Some(Value::Null) => 0,
        Some(Value::String(s)) => s.len(),
        Some(value) => serde_json::to_string(value).map_or(0, |json| json.len()),
    }
}

fn register_tool_use(item: &Value, registry: &mut HashMap<String, (String, Value)>) {
    if item.get("type").and_then(Value::as_str) == Some("tool_use") {
        if let (Some(id), Some(name)) = (
            item.get("id").and_then(Value::as_str),
            item.get("name").and_then(Value::as_str),
        ) {
            registry.insert(
                id.to_string(),
                (
                    name.to_string(),
                    item.get("input").cloned().unwrap_or(Value::Null),
                ),
            );
        }
    }
}

fn build_turn(items: &[Value], registry: &HashMap<String, (String, Value)>) -> String {
    let mut turn = String::new();
    for item in items {
        let frag = match item.get("type").and_then(Value::as_str) {
            Some("text") => format_text(item),
            Some("tool_use") => Some(format_tool_use(item)),
            Some("tool_result") => Some(format_tool_result(item, registry)),
            _ => None,
        };
        if let Some(fragment) = frag {
            if !turn.is_empty() {
                turn.push('\n');
            }
            turn.push_str(&fragment);
        }
    }
    turn
}

fn format_text(item: &Value) -> Option<String> {
    let t = item
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if t.is_empty() {
        None
    } else {
        Some(truncate_text(t, 2000))
    }
}

fn format_tool_use(item: &Value) -> String {
    let name = item.get("name").and_then(Value::as_str).unwrap_or("?");
    let input = item.get("input").unwrap_or(&Value::Null);
    let file_path = || {
        input
            .get("file_path")
            .and_then(Value::as_str)
            .unwrap_or("?")
    };
    match name {
        "Read" | "NotebookRead" => {
            let p = input
                .get("file_path")
                .or_else(|| input.get("notebook_path"))
                .and_then(Value::as_str)
                .unwrap_or("?");
            format!("[Read: {p}]")
        }
        "Write" => format!("[Write: {}]", file_path()),
        "Edit" | "MultiEdit" | "str_replace_based_edit" | "NotebookEdit" => {
            format!("[Edit: {}]", file_path())
        }
        "Bash" => {
            let cmd = input.get("command").and_then(Value::as_str).unwrap_or("");
            format!("[Bash: {}]", truncate_text(cmd, 80))
        }
        "Glob" => format!(
            "[Glob: {}]",
            input.get("pattern").and_then(Value::as_str).unwrap_or("?")
        ),
        "Grep" => format!(
            "[Grep: {}]",
            input.get("pattern").and_then(Value::as_str).unwrap_or("?")
        ),
        _ => format!("[Tool: {name}]"),
    }
}

fn format_tool_result(item: &Value, registry: &HashMap<String, (String, Value)>) -> String {
    let id = item
        .get("tool_use_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let is_error = item
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let text = extract_content_text(item.get("content"));
    let (tool_name, tool_input) = registry
        .get(id)
        .map(|(n, i)| (n.as_str(), i))
        .unwrap_or(("unknown", &Value::Null));
    let pfx = if is_error { "[Error" } else { "[Result" };
    let file_arg = |key: &str| tool_input.get(key).and_then(Value::as_str).unwrap_or("?");
    match tool_name {
        "Read" | "NotebookRead" => {
            let p = tool_input
                .get("file_path")
                .or_else(|| tool_input.get("notebook_path"))
                .and_then(Value::as_str)
                .unwrap_or("?");
            format!("{pfx}: {p} ({} lines)]", text.lines().count())
        }
        "Bash" => format!("{pfx}: bash]\n{}", head_tail_5(&text)),
        "Write" => format!("{pfx}: wrote {}]", file_arg("file_path")),
        "Edit" | "MultiEdit" | "str_replace_based_edit" | "NotebookEdit" => {
            format!("{pfx}: edited {}]", file_arg("file_path"))
        }
        _ => {
            if text.len() <= 500 {
                format!("{pfx}: {text}]")
            } else {
                format!("{pfx}: {} chars]", text.len())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_user_text_kept() {
        let line =
            r#"{"message":{"role":"user","content":[{"type":"text","text":"Fix the bug"}]}}"#;
        let r = preprocess(line);
        assert!(r.contains("[User]") && r.contains("Fix the bug"));
    }

    #[test]
    fn claude_assistant_text_kept() {
        let line = r#"{"message":{"role":"assistant","content":[{"type":"text","text":"I will fix it"}],"usage":{"input_tokens":100,"output_tokens":10}}}"#;
        let r = preprocess(line);
        assert!(r.contains("[Assistant]") && r.contains("I will fix it"));
    }

    #[test]
    fn claude_system_role_stripped() {
        let line =
            r#"{"message":{"role":"system","content":[{"type":"text","text":"Tool defs"}]}}"#;
        assert!(preprocess(line).is_empty());
    }

    #[test]
    fn claude_metadata_lines_stripped() {
        let content = "{\"type\":\"summary\",\"summary\":\"compact\"}\n{\"type\":\"ai-title\",\"aiTitle\":\"Fix\"}";
        assert!(preprocess(content).is_empty());
    }

    #[test]
    fn claude_tool_use_formatted() {
        let line = r#"{"message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"src/main.rs"}}]}}"#;
        let r = preprocess(line);
        assert!(r.contains("[Read: src/main.rs]"));
    }

    #[test]
    fn claude_file_read_result_stripped_to_summary() {
        let assistant = r#"{"message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"src/main.rs"}}]}}"#;
        let body = (1..=100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\\n");
        let user = format!(
            r#"{{"message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","content":"{body}"}}]}}}}"#
        );
        let r = preprocess(&format!("{assistant}\n{user}"));
        assert!(r.contains("src/main.rs"));
        assert!(r.contains("100 lines"));
        assert!(!r.contains("line 50"));
    }

    #[test]
    fn claude_bash_result_uses_head_tail() {
        let assistant = r#"{"message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"cargo test"}}]}}"#;
        let body = (1..=30)
            .map(|i| format!("test_{i}"))
            .collect::<Vec<_>>()
            .join("\\n");
        let user = format!(
            r#"{{"message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t2","content":"{body}"}}]}}}}"#
        );
        let r = preprocess(&format!("{assistant}\n{user}"));
        assert!(r.contains("bash"));
        assert!(r.contains("omitted"));
    }
}
