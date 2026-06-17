// HalluScribe - ChatGPT export content parsing and message filtering helpers.
use super::MessageRole;
use serde_json::Value;

pub(super) fn should_skip_message(message: &Value, role: &MessageRole) -> bool {
    let metadata = message.get("metadata");
    let content_type = message
        .get("content")
        .and_then(|content| content.get("content_type"))
        .and_then(Value::as_str)
        .unwrap_or("");

    if metadata
        .and_then(|meta| meta.get("is_visually_hidden_from_conversation"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        return true;
    }

    if metadata
        .and_then(|meta| meta.get("is_user_system_message"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        return true;
    }

    matches!(role, MessageRole::System)
        && (content_type == "user_editable_context"
            || content_type.is_empty()
            || message_text_from_content(message).is_none())
}

pub(super) fn extract_message_text(message: &Value) -> Option<String> {
    let content = message.get("content")?;
    let mut parts = Vec::new();

    if let Some(text) = message_text_from_content(message) {
        parts.push(text);
    }

    let attachment_note = attachment_note(message);
    if !attachment_note.is_empty() {
        parts.push(attachment_note);
    }

    let joined = parts.join("\n\n");
    let normalized = normalize_whitespace_lines(&joined);
    (!normalized.is_empty())
        .then_some(normalized)
        .or_else(|| extract_text_from_parts(content.get("parts")?))
}

fn message_text_from_content(message: &Value) -> Option<String> {
    let content = message.get("content")?;
    let content_type = content
        .get("content_type")
        .and_then(Value::as_str)
        .unwrap_or("");

    let text = match content_type {
        "text" | "multimodal_text" => extract_text_from_parts(content.get("parts")?),
        "code" => content
            .get("text")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        _ => extract_text_from_parts(content.get("parts")?),
    }?;

    let normalized = normalize_whitespace_lines(&text);
    (!normalized.is_empty()).then_some(normalized)
}

fn extract_text_from_parts(parts: &Value) -> Option<String> {
    let parts = parts.as_array()?;
    let mut text_parts = Vec::new();
    for part in parts {
        collect_text_fragments(part, &mut text_parts);
    }
    let text = normalize_whitespace_lines(&text_parts.join("\n"));
    (!text.is_empty()).then_some(text)
}

fn collect_text_fragments(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => push_text(out, text),
        Value::Array(items) => {
            for item in items {
                collect_text_fragments(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(content_type) = map.get("content_type").and_then(Value::as_str) {
                if matches!(
                    content_type,
                    "image_asset_pointer" | "audio_asset_pointer" | "video_asset_pointer"
                ) {
                    return;
                }
            }

            for key in ["text", "result", "body"] {
                if let Some(text) = map.get(key).and_then(Value::as_str) {
                    push_text(out, text);
                }
            }

            for key in ["parts", "items"] {
                if let Some(child) = map.get(key) {
                    collect_text_fragments(child, out);
                }
            }
        }
        _ => {}
    }
}

fn attachment_note(message: &Value) -> String {
    let names = message
        .get("metadata")
        .and_then(|meta| meta.get("attachments"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|attachment| {
            attachment
                .get("name")
                .or_else(|| attachment.get("file_name"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if names.is_empty() {
        String::new()
    } else {
        format!("[Attachments: {}]", names.join(", "))
    }
}

fn push_text(out: &mut Vec<String>, text: &str) {
    let text = normalize_whitespace_lines(text);
    if !text.is_empty() {
        out.push(text);
    }
}

fn normalize_whitespace_lines(text: &str) -> String {
    text.replace("\r\n", "\n")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
