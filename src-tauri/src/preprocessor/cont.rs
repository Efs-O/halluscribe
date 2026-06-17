// HalluScribe - Continue session preprocessing.

use super::shared::{extract_content_text, truncate_text};
use serde_json::Value;
use std::{fs, path::Path};

/// Entry point. Accepts either a direct Continue session file path or the legacy
/// `tokensGenerated.jsonl` path used by the previous scanner implementation.
pub(super) fn preprocess(path: &Path) -> String {
    let Some(session_path) = resolve_session_path(path) else {
        return String::new();
    };
    let Ok(raw) = fs::read_to_string(&session_path) else {
        return String::new();
    };
    let Ok(session) = serde_json::from_str::<Value>(&raw) else {
        return String::new();
    };

    parse_session(&session)
}

fn resolve_session_path(path: &Path) -> Option<std::path::PathBuf> {
    let is_session_json = path.extension().and_then(|ext| ext.to_str()) == Some("json")
        && path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("sessions");
    if is_session_json {
        return Some(path.to_path_buf());
    }

    let root = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)?;
    let chat_path = root
        .join("dev_data")
        .join("0.2.0")
        .join("chatInteraction.jsonl");
    let session_id = last_session_id(&chat_path)?;
    Some(root.join("sessions").join(format!("{session_id}.json")))
}

fn last_session_id(chat_path: &Path) -> Option<String> {
    let content = fs::read_to_string(chat_path).ok()?;
    content.lines().rev().find_map(|line| {
        let v: Value = serde_json::from_str(line).ok()?;
        v.get("sessionId")?.as_str().map(str::to_string)
    })
}

fn parse_session(session: &Value) -> String {
    // Standard Continue format: { "history": [ { "message": { role, content } }, ... ] }
    if let Some(items) = session.get("history").and_then(Value::as_array) {
        let turns: Vec<String> = items.iter().filter_map(parse_history_item).collect();
        if !turns.is_empty() {
            return turns.join("\n");
        }
    }
    // Fallback: bare array of message objects
    if let Some(items) = session.as_array() {
        let turns: Vec<String> = items.iter().filter_map(parse_message).collect();
        return turns.join("\n");
    }
    String::new()
}

fn parse_history_item(item: &Value) -> Option<String> {
    parse_message(item.get("message")?)
}

fn parse_message(msg: &Value) -> Option<String> {
    let role = msg.get("role").and_then(Value::as_str)?;
    if role == "system" {
        return None;
    }
    let label = if role == "user" {
        "[User]"
    } else {
        "[Assistant]"
    };
    let text = extract_content_text(msg.get("content"));
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(format!("{label}\n{}", truncate_text(text, 2000)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &TempDir, rel: &str, content: &str) {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn tokens_path(dir: &TempDir) -> std::path::PathBuf {
        dir.path()
            .join("dev_data")
            .join("0.2.0")
            .join("tokensGenerated.jsonl")
    }

    #[test]
    fn continue_preprocess_extracts_conversation() {
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "dev_data/0.2.0/chatInteraction.jsonl",
            r#"{"sessionId":"abc123","modelName":"gemma"}"#,
        );
        write(
            &dir,
            "sessions/abc123.json",
            r#"{"history":[
                {"message":{"role":"user","content":"How do I fix this?"}},
                {"message":{"role":"assistant","content":"Here is the fix."}}
            ]}"#,
        );
        write(
            &dir,
            "dev_data/0.2.0/tokensGenerated.jsonl",
            r#"{"model":"gemma","promptTokens":1000}"#,
        );

        let result = preprocess(&tokens_path(&dir));
        assert!(result.contains("[User]"));
        assert!(result.contains("How do I fix this?"));
        assert!(result.contains("[Assistant]"));
        assert!(result.contains("Here is the fix."));
    }

    #[test]
    fn continue_preprocess_skips_system_role() {
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "dev_data/0.2.0/chatInteraction.jsonl",
            r#"{"sessionId":"s1","modelName":"gemma"}"#,
        );
        write(
            &dir,
            "sessions/s1.json",
            r#"{"history":[
                {"message":{"role":"system","content":"You are helpful."}},
                {"message":{"role":"user","content":"Hello"}}
            ]}"#,
        );
        write(&dir, "dev_data/0.2.0/tokensGenerated.jsonl", "");

        let result = preprocess(&tokens_path(&dir));
        assert!(!result.contains("You are helpful."));
        assert!(result.contains("Hello"));
    }

    #[test]
    fn continue_preprocess_uses_last_session_id() {
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "dev_data/0.2.0/chatInteraction.jsonl",
            "{ \"sessionId\":\"old\",\"modelName\":\"gemma\"}\n{\"sessionId\":\"new\",\"modelName\":\"gemma\"}",
        );
        write(
            &dir,
            "sessions/new.json",
            r#"{"history":[{"message":{"role":"user","content":"newest session"}}]}"#,
        );
        write(&dir, "dev_data/0.2.0/tokensGenerated.jsonl", "");

        let result = preprocess(&tokens_path(&dir));
        assert!(result.contains("newest session"));
    }

    #[test]
    fn continue_preprocess_returns_empty_when_no_chat_file() {
        let dir = TempDir::new().unwrap();
        write(&dir, "dev_data/0.2.0/tokensGenerated.jsonl", "");

        let result = preprocess(&tokens_path(&dir));
        assert!(result.is_empty());
    }

    #[test]
    fn continue_preprocess_array_content_blocks() {
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "dev_data/0.2.0/chatInteraction.jsonl",
            r#"{"sessionId":"blk","modelName":"gemma"}"#,
        );
        write(
            &dir,
            "sessions/blk.json",
            r#"{"history":[{"message":{"role":"user","content":[{"type":"text","text":"block content"}]}}]}"#,
        );
        write(&dir, "dev_data/0.2.0/tokensGenerated.jsonl", "");

        let result = preprocess(&tokens_path(&dir));
        assert!(result.contains("block content"));
    }

    #[test]
    fn continue_preprocess_reads_direct_session_path() {
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "sessions/direct.json",
            r#"{"history":[{"message":{"role":"assistant","content":"direct session"}}]}"#,
        );

        let result = preprocess(&dir.path().join("sessions").join("direct.json"));
        assert!(result.contains("direct session"));
    }
}
