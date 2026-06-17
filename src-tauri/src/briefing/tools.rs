// HalluScribe - briefing/chat tool schemas and tool-call helpers.

use super::ChatRuntimeOptions;
use crate::search;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

pub(crate) enum ToolCallResult {
    ToolCall {
        id: String,
        name: String,
        args: Value,
    },
    Text {
        text: String,
        prompt_tokens: u32,
        completion_tokens: u32,
    },
}

#[derive(Clone)]
pub(crate) enum ChatScope {
    ArchiveWide,
    AllowedSessionIds(HashSet<String>),
}

pub(crate) fn chat_tools(runtime: &ChatRuntimeOptions) -> Vec<Value> {
    let mut tools = vec![search_sessions_tool(), read_session_tool()];
    if runtime.web_search_enabled && super::web_search::web_search_available(runtime) {
        tools.push(web_search_tool());
    }
    if runtime.web_search_enabled && super::web_search::web_fetch_available(runtime) {
        tools.push(web_fetch_tool());
    }
    tools
}

pub(crate) fn execute_tool(
    archive_dir: &Path,
    runtime: &ChatRuntimeOptions,
    name: &str,
    args: &Value,
) -> String {
    match name {
        "search_sessions" => {
            let params = search::SearchParams {
                query: args["query"].as_str().map(str::to_string),
                date_from: args["date_from"].as_str().map(str::to_string),
                date_to: args["date_to"].as_str().map(str::to_string),
                tags: args["tags"].as_array().map(|items| {
                    items
                        .iter()
                        .filter_map(|value| value.as_str())
                        .map(str::to_string)
                        .collect()
                }),
                project: args["project"].as_str().map(str::to_string),
                tool: args["tool"].as_str().map(str::to_string),
                limit: args["limit"].as_u64().map(|n| n as usize),
            };
            let allowed_ids = match &runtime.chat_scope {
                ChatScope::ArchiveWide => None,
                ChatScope::AllowedSessionIds(ids) => Some(ids),
            };
            let results = search::search_sessions_in_scope(archive_dir, &params, allowed_ids);
            serde_json::to_string_pretty(&results).unwrap_or_default()
        }
        "read_session" => {
            let id = args["session_id"].as_str().unwrap_or("");
            let allowed_ids = match &runtime.chat_scope {
                ChatScope::ArchiveWide => None,
                ChatScope::AllowedSessionIds(ids) => Some(ids),
            };
            search::read_session_in_scope(archive_dir, id, allowed_ids).unwrap_or_else(|e| e)
        }
        "web_search" => execute_web_search(runtime, args),
        "web_fetch" => execute_web_fetch(runtime, args),
        _ => format!("unknown tool: {name}"),
    }
}

pub(crate) fn validate_ollama_api_key(api_key: &str) -> Result<(), String> {
    super::web_search::validate_ollama_api_key(api_key)
}

fn search_sessions_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "search_sessions",
            "description": "Search the session archive. Returns metadata rows only.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query":     { "type": "string" },
                    "date_from": { "type": "string", "description": "ISO date lower bound, format YYYY-MM-DD e.g. 2026-04-15" },
                    "date_to":   { "type": "string", "description": "ISO date upper bound, format YYYY-MM-DD e.g. 2026-04-18" },
                    "tags":      { "type": "array", "items": { "type": "string" } },
                    "project":   { "type": "string" },
                    "tool":      { "type": "string" },
                    "limit":     { "type": "integer" }
                }
            }
        }
    })
}

fn read_session_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "read_session",
            "description": "Read the full markdown content of a session by its session_id.",
            "parameters": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" }
                },
                "required": ["session_id"]
            }
        }
    })
}

fn web_search_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "web_search",
            "description": "Search the public web for recent or current information.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "max_results": { "type": "integer" }
                },
                "required": ["query"]
            }
        }
    })
}

fn web_fetch_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "web_fetch",
            "description": "Fetch the main content of a specific public web page URL.",
            "parameters": {
                "type": "object",
                "properties": {
                    "url": { "type": "string" }
                },
                "required": ["url"]
            }
        }
    })
}

fn execute_web_search(runtime: &ChatRuntimeOptions, args: &Value) -> String {
    super::web_search::execute_web_search(runtime, args)
}

fn execute_web_fetch(runtime: &ChatRuntimeOptions, args: &Value) -> String {
    super::web_search::execute_web_fetch(runtime, args)
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
