// HalluScribe - briefing/chat tool schemas and tool-call helpers.

use super::ChatRuntimeOptions;
use crate::search;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

/// Default / hard-cap session groups returned by `search_raw_transcripts`. Raw
/// excerpts are verbose and Gemma 4's context window is small, so the model can
/// still page wider via the `limit` arg when it needs more groups.
const RAW_DEFAULT_LIMIT: usize = 40;
const RAW_LIMIT_CAP: usize = 120;

/// Default / hard-cap slice sizes for `read_raw_session`, matching the MCP
/// tool's paging so both surfaces behave identically.
const RAW_SESSION_DEFAULT_MAX_CHARS: usize = 20_000;
const RAW_SESSION_MAX_CHARS_CAP: usize = 50_000;

pub(crate) enum ToolCallResult {
    ToolCall {
        id: String,
        name: String,
        args: Value,
        prompt_tokens: u32,
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
    let mut tools = vec![
        provider_counts_tool(),
        search_sessions_tool(),
        read_session_tool(),
        raw_search_tool(),
        read_raw_session_tool(),
    ];
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
        "count_sessions_by_provider" => execute_provider_counts(archive_dir, runtime, args),
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
                offset: args["offset"].as_u64().map(|n| n as usize),
            };
            let allowed_ids = match &runtime.chat_scope {
                ChatScope::ArchiveWide => None,
                ChatScope::AllowedSessionIds(ids) => Some(ids),
            };
            let page = search::search_sessions_page(archive_dir, &params, allowed_ids);
            serde_json::to_string_pretty(&page).unwrap_or_default()
        }
        "read_session" => {
            let id = args["session_id"].as_str().unwrap_or("");
            let allowed_ids = match &runtime.chat_scope {
                ChatScope::ArchiveWide => None,
                ChatScope::AllowedSessionIds(ids) => Some(ids),
            };
            search::read_session_in_scope(archive_dir, id, allowed_ids).unwrap_or_else(|e| e)
        }
        "search_raw_transcripts" => execute_raw_search(archive_dir, runtime, args),
        "read_raw_session" => execute_read_raw_session(archive_dir, args),
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
            "description": "Search the session archive (matches title, tags, and summary body). Multi-word queries are AND-of-words: every word must appear somewhere in the session, in any order; wrap the query in double quotes to force exact-phrase matching instead. This makes it the right tool for 'which sessions mention X and Y' questions - total_matches covers the whole archive scope. COUNTING RULE: for 'how many sessions came from provider/tool X during a date range', set 'tool' plus 'date_from'/'date_to', OMIT 'query', and report 'total_matches' as the exact count. For an all-provider breakdown, use count_sessions_by_provider instead. A query such as 'Forge' counts textual mentions, not Forge-provided sessions; never use search_raw_transcripts for a provider/date count. Returns a JSON object: {searched, total_matches, returned, offset, results}. 'searched' is how many sessions were examined (the whole archive scope for this chat); 'total_matches' is how many of them matched the query; 'results' is only one page of metadata rows (at most 'limit', default 20, max 30). When asked how many sessions you searched, report 'searched' (not 'total_matches'). When total_matches is greater than returned there are more matches than shown - do not claim you have seen them all; page through them by re-calling with an increasing 'offset'.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query":     { "type": "string" },
                    "date_from": { "type": "string", "description": "ISO date lower bound, format YYYY-MM-DD e.g. 2026-04-15" },
                    "date_to":   { "type": "string", "description": "ISO date upper bound, format YYYY-MM-DD e.g. 2026-04-18" },
                    "tags":      { "type": "array", "items": { "type": "string" } },
                    "project":   { "type": "string" },
                    "tool":      { "type": "string", "description": "Source provider/tool filter, e.g. 'forge'. For provider/date counts use this field, not 'query'." },
                    "limit":     { "type": "integer", "description": "Max rows per page (default 20, capped at 30)" },
                    "offset":    { "type": "integer", "description": "Number of leading matches to skip; use with limit to page through all matches when total_matches exceeds returned" }
                }
            }
        }
    })
}

fn provider_counts_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "count_sessions_by_provider",
            "description": "Return the exhaustive session count for EVERY source provider/tool in a date range. ALWAYS use this for 'all providers', provider breakdowns, or provider distributions. Do not guess provider names, page search_sessions results, or infer counts from excerpts. Returns {searched, total_sessions, providers}, where providers is every matching provider with its exact session_count and total_sessions is their exact sum.",
            "parameters": {
                "type": "object",
                "properties": {
                    "date_from": { "type": "string", "description": "ISO date lower bound, format YYYY-MM-DD e.g. 2026-07-01" },
                    "date_to": { "type": "string", "description": "ISO date upper bound, format YYYY-MM-DD e.g. 2026-07-31" }
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

fn raw_search_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "search_raw_transcripts",
            "description": "Brute-force search the PRESERVED RAW TRANSCRIPTS - the verbatim, unredacted text of each session, including full tool output and code that never survives into the summaries. Use this only when search_sessions (cheaper - it searches the distilled summaries) misses something you believe was actually said or done: an exact error string, a variable/function name, a path, a command. 'query' is matched as a LITERAL case-insensitive substring, NOT tokenized, so spaces and punctuation matter and there are no AND/OR operators. For 'which sessions mention X and Y' questions use search_sessions instead: its unquoted words are ANDed and its total_matches covers the whole archive, whereas raw groups are capped and newest-first, so intersecting raw results is NOT exhaustive. It is slower than search_sessions because it decompresses every retained transcript, so try search_sessions first. Returns JSON {sessions, sessions_scanned, sessions_without_raw, sessions_failed, total_hits, results_truncated}. 'sessions' are per-session match groups, NEWEST FIRST, each {session_id, total_hits, excerpts:[{line_no, excerpt}], excerpts_truncated, summarised, title, date}; pass a session_id to read_session for the full body only when 'summarised' is true. A group with summarised:false is an unsummarised raw - a session captured before any sweep summarised it (no read_session body exists), so use its 'title' (source filename) and 'date' instead and treat the excerpts as the only detail available. 'total_hits' counts all matches across the archive even when only some groups are returned. At most 'limit' groups are returned (default 40, capped at 120); when results_truncated is true, narrow the query rather than assuming you have seen everything, and tell the user any session list you report only covers the newest matching sessions.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "description": "Max session groups returned (default 40, capped at 120)" }
                },
                "required": ["query"]
            }
        }
    })
}

fn read_raw_session_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "read_raw_session",
            "description": "Read one session's PRESERVED RAW TRANSCRIPT verbatim - the untouched source file, not a summary. Use it to open the full raw of a search_sessions or search_raw_transcripts hit. Also works for unsummarised captured sessions (search_raw_transcripts groups with summarised:false) that have no read_session body. Paged by BYTE offsets clamped to UTF-8 character boundaries: pass offset/max_chars (default 20000, capped at 50000). Returns JSON {session_id, text, offset, next_offset, total_bytes, truncated}; if truncated is true, call again with offset set to next_offset. For multi-chat providers the file holds only this session's own conversation, sliced out of the export - never sibling chats. Errors instead of fabricating content when no raw exists or it isn't valid UTF-8.",
            "parameters": {
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "offset": { "type": "integer", "description": "Byte offset to continue from (default 0)" },
                    "max_chars": { "type": "integer", "description": "Max bytes per page (default 20000, capped at 50000)" }
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

fn execute_raw_search(archive_dir: &Path, runtime: &ChatRuntimeOptions, args: &Value) -> String {
    let query = args["query"].as_str().unwrap_or("");
    let allowed_ids = match &runtime.chat_scope {
        ChatScope::ArchiveWide => None,
        ChatScope::AllowedSessionIds(ids) => Some(ids),
    };
    match search::search_raw(archive_dir, query, allowed_ids) {
        Ok(mut result) => {
            let limit = args["limit"]
                .as_u64()
                .map(|n| n as usize)
                .unwrap_or(RAW_DEFAULT_LIMIT)
                .clamp(1, RAW_LIMIT_CAP);
            if result.sessions.len() > limit {
                result.sessions.truncate(limit);
                result.results_truncated = true;
            }
            serde_json::to_string_pretty(&result).unwrap_or_default()
        }
        Err(error) => error.to_string(),
    }
}

fn execute_provider_counts(
    archive_dir: &Path,
    runtime: &ChatRuntimeOptions,
    args: &Value,
) -> String {
    let allowed_ids = match &runtime.chat_scope {
        ChatScope::ArchiveWide => None,
        ChatScope::AllowedSessionIds(ids) => Some(ids),
    };
    let result = search::count_sessions_by_provider_in_scope(
        archive_dir,
        args["date_from"].as_str(),
        args["date_to"].as_str(),
        allowed_ids,
    );
    serde_json::to_string_pretty(&result).unwrap_or_default()
}

fn execute_read_raw_session(archive_dir: &Path, args: &Value) -> String {
    let session_id = args["session_id"].as_str().unwrap_or("");
    let offset = args["offset"].as_u64().map(|n| n as usize).unwrap_or(0);
    let max_chars = args["max_chars"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(RAW_SESSION_DEFAULT_MAX_CHARS)
        .clamp(1, RAW_SESSION_MAX_CHARS_CAP);
    match crate::archive::read_raw_session(archive_dir, session_id, offset, max_chars) {
        Ok(page) => serde_json::to_string_pretty(&page).unwrap_or_default(),
        Err(error) => error.to_string(),
    }
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
