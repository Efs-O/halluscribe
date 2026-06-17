// HalluScribe - briefing tool-call tests, including Tavily fallback probes.

use super::*;
use crate::briefing::web_search;
use serde_json::{json, Value};
use std::time::Duration;

fn runtime(web_search_enabled: bool, api_key: Option<&str>) -> ChatRuntimeOptions {
    ChatRuntimeOptions {
        chat_scope: ChatScope::ArchiveWide,
        web_search_enabled,
        ollama_api_key: api_key.map(str::to_string),
        tavily_api_key: None,
        reasoning_enabled: false,
    }
}

#[test]
fn chat_tools_only_include_web_tools_when_enabled_and_configured() {
    let disabled = chat_tools(&runtime(false, Some("key")));
    assert_eq!(disabled.len(), 2);

    let missing_key = chat_tools(&runtime(true, None));
    assert_eq!(missing_key.len(), 2);

    let enabled = chat_tools(&runtime(true, Some("key")));
    assert_eq!(enabled.len(), 4);
}

#[test]
fn clamp_web_search_limit_uses_defaults_and_caps_maximum() {
    assert_eq!(web_search::clamp_web_search_limit(None), 14);
    assert_eq!(web_search::clamp_web_search_limit(Some(1)), 1);
    assert_eq!(web_search::clamp_web_search_limit(Some(99)), 16);
}

#[test]
fn chat_tools_include_tavily_search_without_ollama_fetch() {
    let runtime = ChatRuntimeOptions {
        chat_scope: ChatScope::ArchiveWide,
        web_search_enabled: true,
        ollama_api_key: None,
        tavily_api_key: Some("tavily-key".to_string()),
        reasoning_enabled: false,
    };
    let tools = chat_tools(&runtime);
    let names = tools
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect::<Vec<_>>();

    assert!(names.contains(&"web_search"));
    assert!(names.contains(&"web_fetch"));
}

#[test]
fn tavily_search_results_match_halluscribe_web_search_shape() {
    let long_snippet = "a".repeat(2_050);
    let response = json!({
        "results": [
            {
                "title": "Gemma 4 docs",
                "url": "https://example.com/gemma",
                "content": long_snippet,
            },
            {
                "title": "Release notes",
                "url": "https://example.com/release",
                "content": "Second result",
            }
        ]
    });

    let normalized = web_search::normalize_tavily_search_results(&response, 1);

    assert_eq!(normalized["results"].as_array().map(Vec::len), Some(1));
    assert_eq!(normalized["results"][0]["title"], "Gemma 4 docs");
    assert_eq!(normalized["results"][0]["url"], "https://example.com/gemma");
    assert_eq!(
        normalized["results"][0]["content"]
            .as_str()
            .expect("normalized content should be a string")
            .chars()
            .count(),
        2_003,
    );
}

#[test]
fn tavily_search_results_default_to_empty_results() {
    let normalized = web_search::normalize_tavily_search_results(&json!({}), 5);
    assert_eq!(normalized, json!({ "results": [] }));
}

#[test]
fn tavily_extract_result_matches_halluscribe_web_fetch_shape() {
    let response = json!({
        "results": [
            {
                "url": "https://example.com/gemma",
                "title": "Gemma page",
                "raw_content": "x".repeat(12_100),
            }
        ]
    });

    let normalized =
        web_search::normalize_tavily_extract_result(&response, "https://fallback.test");

    assert_eq!(normalized["title"], "Gemma page");
    assert_eq!(normalized["url"], "https://example.com/gemma");
    assert_eq!(normalized["links"], json!([]));
    assert_eq!(
        normalized["content"]
            .as_str()
            .expect("normalized content should be a string")
            .chars()
            .count(),
        12_003, // FETCH_CONTENT_LIMIT (12_000) + 3 for the "..." suffix
    );
}

#[test]
#[ignore = "manual live probe; requires HALLUSCRIBE_TAVILY_API_KEY"]
fn tavily_search_live_probe_returns_results() {
    let api_key = std::env::var("HALLUSCRIBE_TAVILY_API_KEY")
        .expect("set HALLUSCRIBE_TAVILY_API_KEY to run the live Tavily probe");
    let max_results = 3;
    let response = reqwest::blocking::Client::new()
        .post("https://api.tavily.com/search")
        .bearer_auth(api_key.trim())
        .json(&json!({
            "query": "OpenAI GPT-5.4 release",
            "search_depth": "basic",
            "max_results": max_results,
        }))
        .timeout(Duration::from_secs(30))
        .send()
        .expect("Tavily search request should succeed");
    let status = response.status();
    let body = response
        .json::<Value>()
        .expect("Tavily search response should be valid JSON");

    assert!(
        status.is_success(),
        "expected success status from Tavily search, got {status}: {body}"
    );
    let normalized = web_search::normalize_tavily_search_results(&body, max_results);
    assert!(
        normalized["results"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false),
        "expected Tavily search to return at least one normalized result: {body}"
    );
}

#[test]
#[ignore = "manual live probe; requires HALLUSCRIBE_TAVILY_API_KEY"]
fn tavily_extract_live_probe_returns_content() {
    let api_key = std::env::var("HALLUSCRIBE_TAVILY_API_KEY")
        .expect("set HALLUSCRIBE_TAVILY_API_KEY to run the live Tavily extract probe");
    let response = reqwest::blocking::Client::new()
        .post("https://api.tavily.com/extract")
        .bearer_auth(api_key.trim())
        .json(&json!({
            "urls": "https://openai.com/index/introducing-gpt-5-4/",
            "extract_depth": "basic",
            "format": "markdown",
        }))
        .timeout(Duration::from_secs(30))
        .send()
        .expect("Tavily extract request should succeed");
    let status = response.status();
    let body = response
        .json::<Value>()
        .expect("Tavily extract response should be valid JSON");

    assert!(
        status.is_success(),
        "expected success status from Tavily extract, got {status}: {body}"
    );
    let normalized = web_search::normalize_tavily_extract_result(
        &body,
        "https://openai.com/index/introducing-gpt-5-4/",
    );
    assert!(
        normalized["content"]
            .as_str()
            .map(|content| !content.trim().is_empty())
            .unwrap_or(false),
        "expected Tavily extract to return content: {body}"
    );
}
