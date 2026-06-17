// HalluScribe - streaming response parsers for Ollama and llama.cpp.

use super::tools::ToolCallResult;
use reqwest::blocking::Response;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};

const THINK_OPEN_MARKERS: &[&str] = &["thought<|channel>", "<|channel>", "<|thinking|>", "<think>"];
const THINK_CLOSE_MARKERS: &[&str] = &["<channel|>", "</|thinking|>", "</think>"];

pub(crate) fn consume_openai_stream(
    response: Response,
    emit: &mut impl FnMut(String, bool),
    cancel: &AtomicBool,
) -> Result<ToolCallResult, String> {
    let reader = BufReader::new(response);
    let mut router = ThinkRouter::new();
    let mut carry = String::new();
    let mut answer = String::new();
    let mut tool_calls = ToolCallAssembler::default();
    let mut prompt_tokens: u32 = 0;
    let mut completion_tokens: u32 = 0;

    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim();
        if !line.starts_with("data: ") {
            continue;
        }
        let data = &line[6..];
        if data == "[DONE]" {
            break;
        }
        let chunk: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Some(usage) = chunk.get("usage").filter(|v| !v.is_null()) {
            prompt_tokens = usage["prompt_tokens"].as_u64().unwrap_or(0) as u32;
            completion_tokens = usage["completion_tokens"].as_u64().unwrap_or(0) as u32;
        }
        let delta = &chunk["choices"][0]["delta"];
        tool_calls.push_openai_delta(delta);
        for reasoning in openai_reasoning_text(delta) {
            let sanitized = sanitize_reasoning_chunk(&reasoning);
            if !sanitized.is_empty() {
                emit(sanitized, true);
            }
        }
        let raw = delta["content"].as_str().unwrap_or("");
        if raw.is_empty() {
            continue;
        }

        let (content, new_carry) = strip_carry(raw, &carry);
        carry = new_carry;
        if content.is_empty() {
            continue;
        }
        flush_segments(&mut router, &content, emit, &mut answer);
    }

    if !carry.is_empty() {
        flush_segments(&mut router, &carry, emit, &mut answer);
    }
    finish_stream(answer, tool_calls, prompt_tokens, completion_tokens)
}

pub(crate) fn consume_ollama_stream(
    response: Response,
    emit: &mut impl FnMut(String, bool),
    cancel: &AtomicBool,
) -> Result<ToolCallResult, String> {
    let reader = BufReader::new(response);
    let mut router = ThinkRouter::new();
    let mut carry = String::new();
    let mut answer = String::new();
    let mut tool_calls = ToolCallAssembler::default();
    let mut prompt_tokens: u32 = 0;
    let mut completion_tokens: u32 = 0;

    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let chunk: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let is_done = chunk["done"].as_bool().unwrap_or(false);
        tool_calls.push_ollama_message(&chunk["message"]);
        let thinking = chunk["message"]["thinking"].as_str().unwrap_or("");
        if !thinking.is_empty() {
            let sanitized = sanitize_reasoning_chunk(thinking);
            if !sanitized.is_empty() {
                emit(sanitized, true);
            }
        }
        let raw = chunk["message"]["content"].as_str().unwrap_or("");
        if !raw.is_empty() {
            let (content, new_carry) = strip_carry(raw, &carry);
            carry = new_carry;
            if !content.is_empty() {
                flush_segments(&mut router, &content, emit, &mut answer);
            }
        }
        if is_done {
            prompt_tokens = chunk["prompt_eval_count"].as_u64().unwrap_or(0) as u32;
            completion_tokens = chunk["eval_count"].as_u64().unwrap_or(0) as u32;
            break;
        }
    }

    if !carry.is_empty() {
        flush_segments(&mut router, &carry, emit, &mut answer);
    }
    finish_stream(answer, tool_calls, prompt_tokens, completion_tokens)
}

fn flush_segments(
    router: &mut ThinkRouter,
    content: &str,
    emit: &mut impl FnMut(String, bool),
    answer: &mut String,
) {
    for (text, is_thinking) in router.push(content) {
        if !text.is_empty() {
            if !is_thinking {
                answer.push_str(&text);
            }
            emit(text, is_thinking);
        }
    }
}

fn finish_stream(
    answer: String,
    tool_calls: ToolCallAssembler,
    prompt_tokens: u32,
    completion_tokens: u32,
) -> Result<ToolCallResult, String> {
    match tool_calls.finish()? {
        Some(call) => Ok(call),
        None => Ok(ToolCallResult::Text {
            text: answer,
            prompt_tokens,
            completion_tokens,
        }),
    }
}

fn openai_reasoning_text(delta: &Value) -> Vec<String> {
    let mut result = Vec::new();

    for key in ["reasoning_content", "reasoning", "thinking"] {
        if let Some(text) = delta[key].as_str() {
            result.push(text.to_string());
        }
    }

    if let Some(items) = delta["reasoning_content"].as_array() {
        for item in items {
            if let Some(text) = item["text"].as_str() {
                result.push(text.to_string());
            } else if let Some(text) = item.as_str() {
                result.push(text.to_string());
            }
        }
    }

    result
}

struct ThinkRouter {
    thinking: bool,
}

impl ThinkRouter {
    fn new() -> Self {
        Self { thinking: false }
    }

    fn push(&mut self, content: &str) -> Vec<(String, bool)> {
        let mut result = Vec::new();
        let mut rest = content;
        loop {
            let (markers, flip) = if self.thinking {
                (THINK_CLOSE_MARKERS, false)
            } else {
                (THINK_OPEN_MARKERS, true)
            };
            if let Some((pos, needle)) = find_next_marker(rest, markers) {
                let before = &rest[..pos];
                if !before.is_empty() {
                    result.push((before.to_string(), self.thinking));
                }
                self.thinking = flip;
                rest = &rest[pos + needle.len()..];
            } else {
                if !rest.is_empty() {
                    result.push((rest.to_string(), self.thinking));
                }
                break;
            }
        }
        result
    }
}

#[derive(Default)]
struct ToolCallAssembler {
    calls: BTreeMap<usize, PartialToolCall>,
}

impl ToolCallAssembler {
    fn push_openai_delta(&mut self, delta: &Value) {
        let Some(calls) = delta["tool_calls"].as_array() else {
            return;
        };
        for (fallback_index, call) in calls.iter().enumerate() {
            let index = call["index"].as_u64().unwrap_or(fallback_index as u64) as usize;
            let entry = self.calls.entry(index).or_default();
            if let Some(id) = call["id"].as_str() {
                entry.id = Some(id.to_string());
            }
            if let Some(name) = call["function"]["name"].as_str() {
                entry.name.push_str(name);
            }
            if let Some(arguments) = call["function"]["arguments"].as_str() {
                entry.arguments.push_str(arguments);
            }
        }
    }

    fn push_ollama_message(&mut self, message: &Value) {
        let Some(calls) = message["tool_calls"].as_array() else {
            return;
        };
        for (index, call) in calls.iter().enumerate() {
            let entry = self.calls.entry(index).or_default();
            if let Some(id) = call["id"].as_str() {
                entry.id = Some(id.to_string());
            }
            if let Some(name) = call["function"]["name"].as_str() {
                entry.name = name.to_string();
            }
            match &call["function"]["arguments"] {
                Value::String(arguments) => entry.arguments.push_str(arguments),
                Value::Object(_) | Value::Array(_) => {
                    entry.arguments = call["function"]["arguments"].to_string();
                }
                _ => {}
            }
        }
    }

    fn finish(self) -> Result<Option<ToolCallResult>, String> {
        let Some((index, call)) = self.calls.into_iter().next() else {
            return Ok(None);
        };
        let name = call.name.trim().to_string();
        if name.is_empty() {
            return Err(format!(
                "stream ended with incomplete tool call at index {index}"
            ));
        }
        let args = if call.arguments.trim().is_empty() {
            Value::Object(Default::default())
        } else {
            serde_json::from_str(&call.arguments)
                .map_err(|error| format!("bad streamed tool arguments JSON: {error}"))?
        };
        Ok(Some(ToolCallResult::ToolCall {
            id: call.id.unwrap_or_else(|| format!("call_{}", index + 1)),
            name,
            args,
        }))
    }
}

#[derive(Default)]
struct PartialToolCall {
    id: Option<String>,
    name: String,
    arguments: String,
}

fn strip_carry(raw: &str, carry: &str) -> (String, String) {
    let content = format!("{carry}{raw}");
    for tag in THINK_OPEN_MARKERS
        .iter()
        .chain(THINK_CLOSE_MARKERS.iter())
        .copied()
    {
        let max_partial = content.len().min(tag.len().saturating_sub(1));
        for i in 1..=max_partial {
            if content.ends_with(&tag[..i]) {
                let processed = content[..content.len() - i].to_string();
                let new_carry = content[content.len() - i..].to_string();
                return (processed, new_carry);
            }
        }
    }
    (content, String::new())
}

fn find_next_marker<'a>(content: &str, markers: &'a [&str]) -> Option<(usize, &'a str)> {
    markers
        .iter()
        .filter_map(|marker| content.find(marker).map(|pos| (pos, *marker)))
        .min_by_key(|(pos, marker)| (*pos, marker.len()))
}

fn sanitize_reasoning_chunk(text: &str) -> String {
    let mut sanitized = text.to_string();
    for marker in THINK_OPEN_MARKERS
        .iter()
        .chain(THINK_CLOSE_MARKERS.iter())
        .copied()
    {
        sanitized = sanitized.replace(marker, "");
    }
    if sanitized.trim().is_empty() {
        String::new()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::{
        openai_reasoning_text, sanitize_reasoning_chunk, strip_carry, ThinkRouter,
        ToolCallAssembler, ToolCallResult,
    };
    use serde_json::json;

    #[test]
    fn think_router_splits_reasoning_and_answer_segments() {
        let mut router = ThinkRouter::new();
        let segments = router.push("<|channel>plan<channel|>answer");
        assert_eq!(
            segments,
            vec![("plan".to_string(), true), ("answer".to_string(), false)]
        );
    }

    #[test]
    fn think_router_treats_thought_prefixed_channel_as_reasoning_marker() {
        let mut router = ThinkRouter::new();
        let segments = router.push("thought<|channel>plan<channel|>answer");
        assert_eq!(
            segments,
            vec![("plan".to_string(), true), ("answer".to_string(), false)]
        );
    }

    #[test]
    fn strip_carry_holds_partial_think_tag_until_next_chunk() {
        let tag = "<|channel>";
        let partial = &tag[..tag.len() - 3];
        let (content, carry) = strip_carry(partial, "");
        assert!(content.is_empty());
        assert_eq!(carry, partial);

        let remainder = &tag[partial.len()..];
        let (content, carry) = strip_carry(remainder, &carry);
        assert_eq!(content, tag);
        assert!(carry.is_empty());
    }

    #[test]
    fn tool_call_assembler_rebuilds_openai_streamed_arguments() {
        let mut assembler = ToolCallAssembler::default();
        assembler.push_openai_delta(&json!({
            "tool_calls": [{
                "index": 0,
                "id": "call_123",
                "function": { "name": "search_sessions", "arguments": "{\"query\":\"gem" }
            }]
        }));
        assembler.push_openai_delta(&json!({
            "tool_calls": [{
                "index": 0,
                "function": { "arguments": "ma\"}" }
            }]
        }));

        let result = assembler.finish().unwrap();
        match result {
            Some(ToolCallResult::ToolCall { id, name, args }) => {
                assert_eq!(id, "call_123");
                assert_eq!(name, "search_sessions");
                assert_eq!(args["query"], "gemma");
            }
            _ => panic!("expected streamed tool call"),
        }
    }

    #[test]
    fn tool_call_assembler_reads_ollama_object_arguments() {
        let mut assembler = ToolCallAssembler::default();
        assembler.push_ollama_message(&json!({
            "tool_calls": [{
                "function": {
                    "name": "web_fetch",
                    "arguments": { "url": "https://example.com" }
                }
            }]
        }));

        let result = assembler.finish().unwrap();
        match result {
            Some(ToolCallResult::ToolCall { name, args, .. }) => {
                assert_eq!(name, "web_fetch");
                assert_eq!(args["url"], "https://example.com");
            }
            _ => panic!("expected ollama tool call"),
        }
    }

    #[test]
    fn openai_reasoning_text_reads_reasoning_content_string() {
        let values = openai_reasoning_text(&json!({
            "reasoning_content": "Let me think this through."
        }));
        assert_eq!(values, vec!["Let me think this through.".to_string()]);
    }

    #[test]
    fn openai_reasoning_text_reads_reasoning_content_parts_array() {
        let values = openai_reasoning_text(&json!({
            "reasoning_content": [
                { "text": "First" },
                { "text": " second" }
            ]
        }));
        assert_eq!(values, vec!["First".to_string(), " second".to_string()]);
    }

    #[test]
    fn sanitize_reasoning_chunk_strips_marker_noise() {
        assert_eq!(sanitize_reasoning_chunk("thought<|channel>"), "");
        assert_eq!(sanitize_reasoning_chunk("<think>plan</think>"), "plan");
        assert_eq!(
            sanitize_reasoning_chunk("<|thinking|>plan</|thinking|>"),
            "plan"
        );
    }
}
