// HalluScribe - live web search/fetch providers for chat tools.

use super::ChatRuntimeOptions;
use serde_json::Value;
use std::time::Duration;

const WEB_TIMEOUT_SECS: u64 = 30;
const DEFAULT_WEB_SEARCH_RESULTS: u64 = 14;
const MAX_WEB_SEARCH_RESULTS: u64 = 16;
const SEARCH_CONTENT_LIMIT: usize = 2_000;
const FETCH_CONTENT_LIMIT: usize = 12_000;

pub(crate) fn web_search_available(runtime: &ChatRuntimeOptions) -> bool {
    runtime.ollama_api_key.is_some() || runtime.tavily_api_key.is_some()
}

pub(crate) fn web_fetch_available(runtime: &ChatRuntimeOptions) -> bool {
    runtime.ollama_api_key.is_some() || runtime.tavily_api_key.is_some()
}

pub(crate) fn validate_ollama_api_key(api_key: &str) -> Result<(), String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("missing Ollama API key".to_string());
    }
    let body = serde_json::json!({
        "query": "ollama",
        "max_results": 1,
    });
    ollama_web_request("web_search", trimmed, &body).map(|_| ())
}

pub(crate) fn execute_web_search(runtime: &ChatRuntimeOptions, args: &Value) -> String {
    let query = args["query"].as_str().unwrap_or("").trim();
    if query.is_empty() {
        return tool_error("web_search requires a non-empty query");
    }
    let max_results = clamp_web_search_limit(args["max_results"].as_u64());

    if let Some(api_key) = runtime.ollama_api_key.as_deref() {
        let body = serde_json::json!({
            "query": query,
            "max_results": max_results,
        });
        match ollama_web_request("web_search", api_key, &body) {
            Ok(value) => return normalize_ollama_search_results(&value, max_results).to_string(),
            Err(ollama_error) => {
                if let Some(tavily_key) = runtime.tavily_api_key.as_deref() {
                    match tavily_search_request(tavily_key, query, max_results) {
                        Ok(value) => {
                            return normalize_tavily_search_results(&value, max_results)
                                .to_string();
                        }
                        Err(tavily_error) => {
                            return tool_error(&format!(
                                "Ollama search failed: {ollama_error}; Tavily fallback failed: {tavily_error}"
                            ));
                        }
                    }
                }
                return tool_error(&ollama_error);
            }
        }
    }

    let Some(tavily_key) = runtime.tavily_api_key.as_deref() else {
        return tool_error("web search unavailable: missing Ollama and Tavily API keys");
    };
    match tavily_search_request(tavily_key, query, max_results) {
        Ok(value) => normalize_tavily_search_results(&value, max_results).to_string(),
        Err(error) => tool_error(&error),
    }
}

pub(crate) fn execute_web_fetch(runtime: &ChatRuntimeOptions, args: &Value) -> String {
    let url = args["url"].as_str().unwrap_or("").trim();
    if url.is_empty() {
        return tool_error("web_fetch requires a non-empty url");
    }

    if let Some(api_key) = runtime.ollama_api_key.as_deref() {
        let body = serde_json::json!({ "url": url });
        match ollama_web_request("web_fetch", api_key, &body) {
            Ok(value) => return normalize_ollama_fetch_result(&value, url).to_string(),
            Err(ollama_error) => {
                if let Some(tavily_key) = runtime.tavily_api_key.as_deref() {
                    match tavily_extract_request(tavily_key, url) {
                        Ok(value) => {
                            return normalize_tavily_extract_result(&value, url).to_string()
                        }
                        Err(tavily_error) => {
                            return tool_error(&format!(
                                "Ollama fetch failed: {ollama_error}; Tavily fallback failed: {tavily_error}"
                            ));
                        }
                    }
                }
                return tool_error(&ollama_error);
            }
        }
    }

    let Some(tavily_key) = runtime.tavily_api_key.as_deref() else {
        return tool_error("web fetch unavailable: missing Ollama and Tavily API keys");
    };
    match tavily_extract_request(tavily_key, url) {
        Ok(value) => normalize_tavily_extract_result(&value, url).to_string(),
        Err(error) => tool_error(&error),
    }
}

pub(crate) fn clamp_web_search_limit(requested: Option<u64>) -> u64 {
    requested
        .unwrap_or(DEFAULT_WEB_SEARCH_RESULTS)
        .clamp(1, MAX_WEB_SEARCH_RESULTS)
}

fn ollama_web_request(endpoint: &str, api_key: &str, body: &Value) -> Result<Value, String> {
    let response = reqwest::blocking::Client::new()
        .post(format!("https://ollama.com/api/{endpoint}"))
        .bearer_auth(api_key)
        .json(body)
        .timeout(Duration::from_secs(WEB_TIMEOUT_SECS))
        .send()
        .map_err(|error| error.to_string())?;
    response_json_or_error(response, "Ollama web request")
}

fn tavily_search_request(api_key: &str, query: &str, max_results: u64) -> Result<Value, String> {
    let response = reqwest::blocking::Client::new()
        .post("https://api.tavily.com/search")
        .bearer_auth(api_key.trim())
        .json(&serde_json::json!({
            "query": query,
            "search_depth": "basic",
            "max_results": max_results,
        }))
        .timeout(Duration::from_secs(WEB_TIMEOUT_SECS))
        .send()
        .map_err(|error| error.to_string())?;
    response_json_or_error(response, "Tavily search")
}

fn tavily_extract_request(api_key: &str, url: &str) -> Result<Value, String> {
    let response = reqwest::blocking::Client::new()
        .post("https://api.tavily.com/extract")
        .bearer_auth(api_key.trim())
        .json(&serde_json::json!({
            "urls": url,
            "extract_depth": "basic",
            "format": "markdown",
        }))
        .timeout(Duration::from_secs(WEB_TIMEOUT_SECS))
        .send()
        .map_err(|error| error.to_string())?;
    response_json_or_error(response, "Tavily extract")
}

fn response_json_or_error(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<Value, String> {
    let status = response.status();
    let value = response
        .json::<Value>()
        .map_err(|error| error.to_string())?;
    if !status.is_success() {
        let message = value["error"]["message"]
            .as_str()
            .or_else(|| value["error"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{label} failed with status {status}"));
        return Err(message);
    }
    Ok(value)
}

pub(crate) fn normalize_ollama_search_results(value: &Value, max_results: u64) -> Value {
    let results = value["results"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(max_results as usize)
                .map(|item| {
                    serde_json::json!({
                        "title": item["title"].as_str().unwrap_or(""),
                        "url": item["url"].as_str().unwrap_or(""),
                        "content": truncate_text(
                            item["content"]
                                .as_str()
                                .or_else(|| item["snippet"].as_str())
                                .unwrap_or(""),
                            SEARCH_CONTENT_LIMIT,
                        ),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({ "results": results })
}

pub(crate) fn normalize_tavily_search_results(value: &Value, max_results: u64) -> Value {
    let results = value["results"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(max_results as usize)
                .map(|item| {
                    serde_json::json!({
                        "title": item["title"].as_str().unwrap_or(""),
                        "url": item["url"].as_str().unwrap_or(""),
                        "content": truncate_text(item["content"].as_str().unwrap_or(""), SEARCH_CONTENT_LIMIT),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({ "results": results })
}

fn normalize_ollama_fetch_result(value: &Value, fallback_url: &str) -> Value {
    let links = value["links"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .take(20)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({
        "title": value["title"].as_str().unwrap_or(""),
        "url": value["url"].as_str().unwrap_or(fallback_url),
        "content": truncate_text(value["content"].as_str().unwrap_or(""), FETCH_CONTENT_LIMIT),
        "links": links,
    })
}

pub(crate) fn normalize_tavily_extract_result(value: &Value, fallback_url: &str) -> Value {
    let result = value["results"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or_default();
    serde_json::json!({
        "title": result["title"].as_str().unwrap_or(""),
        "url": result["url"].as_str().unwrap_or(fallback_url),
        "content": truncate_text(result["raw_content"].as_str().unwrap_or(""), FETCH_CONTENT_LIMIT),
        "links": [],
    })
}

fn truncate_text(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars().take(limit).collect::<String>() + "..."
}

fn tool_error(message: &str) -> String {
    serde_json::json!({ "error": message }).to_string()
}
