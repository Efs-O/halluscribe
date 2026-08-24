// HalluScribe - deterministic archive analysis tools for interactive chat.

use super::tools::ChatScope;
use crate::search::{self, PeriodBreakdown};
use serde_json::Value;
use std::path::Path;

pub(crate) fn tool_schemas() -> Vec<Value> {
    vec![
        provider_counts_tool(),
        archive_facets_tool(),
        compare_periods_tool(),
        archive_health_tool(),
    ]
}

pub(crate) fn execute(
    archive_dir: &Path,
    scope: &ChatScope,
    name: &str,
    args: &Value,
) -> Option<String> {
    let allowed_ids = match scope {
        ChatScope::ArchiveWide => None,
        ChatScope::AllowedSessionIds(ids) => Some(ids),
    };
    let result = match name {
        "count_sessions_by_provider" => {
            serde_json::to_string_pretty(&search::count_sessions_by_provider_in_scope(
                archive_dir,
                args["date_from"].as_str(),
                args["date_to"].as_str(),
                allowed_ids,
            ))
            .unwrap_or_default()
        }
        "list_archive_facets" => serde_json::to_string_pretty(&search::archive_facets_in_scope(
            archive_dir,
            args["date_from"].as_str(),
            args["date_to"].as_str(),
            allowed_ids,
        ))
        .unwrap_or_default(),
        "compare_session_periods" => execute_period_comparison(archive_dir, allowed_ids, args),
        "archive_health" => {
            serde_json::to_string_pretty(&search::archive_health_in_scope(archive_dir, allowed_ids))
                .unwrap_or_default()
        }
        _ => return None,
    };
    Some(result)
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

fn archive_facets_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "list_archive_facets",
            "description": "List the valid provider, project, and session-type values in the archive, with exact session counts and the actual date span. Use BEFORE filtered searching when you do not know valid provider/project/type names, and whenever the user asks what sources or projects exist. Optional dates narrow the facets to that range. Returns {searched, total_sessions, first_date, last_date, providers, projects, session_types}.",
            "parameters": {
                "type": "object",
                "properties": {
                    "date_from": { "type": "string", "description": "Optional ISO date lower bound, YYYY-MM-DD." },
                    "date_to": { "type": "string", "description": "Optional ISO date upper bound, YYYY-MM-DD." }
                }
            }
        }
    })
}

fn compare_periods_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "compare_session_periods",
            "description": "Compare two date ranges exactly, including totals, absolute changes, percentage changes, and an exhaustive provider/project/session-type breakdown. ALWAYS use for surge, increase/decrease, trend, or 'July versus August' questions. Percentage change is null when the first period has zero sessions, rather than being invented. Do not run separate searches and calculate these figures yourself.",
            "parameters": {
                "type": "object",
                "properties": {
                    "first_date_from": { "type": "string", "description": "First-period ISO date lower bound, YYYY-MM-DD." },
                    "first_date_to": { "type": "string", "description": "First-period ISO date upper bound, YYYY-MM-DD." },
                    "second_date_from": { "type": "string", "description": "Second-period ISO date lower bound, YYYY-MM-DD." },
                    "second_date_to": { "type": "string", "description": "Second-period ISO date upper bound, YYYY-MM-DD." },
                    "group_by": { "type": "string", "enum": ["provider", "project", "session_type"], "description": "Breakdown dimension; defaults to provider." }
                },
                "required": ["first_date_from", "first_date_to", "second_date_from", "second_date_to"]
            }
        }
    })
}

fn archive_health_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "archive_health",
            "description": "Report the archive's evidence coverage: indexed session count, actual date span, summary files present/missing, and preserved raw copies present/missing. Use when the user asks whether an archive conclusion is complete or trustworthy, or why raw/summary evidence may be unavailable. This reports observed coverage only; do not infer a cause from a missing file.",
            "parameters": { "type": "object", "properties": {} }
        }
    })
}

fn execute_period_comparison(
    archive_dir: &Path,
    allowed_ids: Option<&std::collections::HashSet<String>>,
    args: &Value,
) -> String {
    let Some(first_from) = args["first_date_from"].as_str() else {
        return "compare_session_periods requires first_date_from".to_string();
    };
    let Some(first_to) = args["first_date_to"].as_str() else {
        return "compare_session_periods requires first_date_to".to_string();
    };
    let Some(second_from) = args["second_date_from"].as_str() else {
        return "compare_session_periods requires second_date_from".to_string();
    };
    let Some(second_to) = args["second_date_to"].as_str() else {
        return "compare_session_periods requires second_date_to".to_string();
    };
    let breakdown = match args["group_by"].as_str().unwrap_or("provider") {
        "provider" => PeriodBreakdown::Provider,
        "project" => PeriodBreakdown::Project,
        "session_type" => PeriodBreakdown::SessionType,
        other => {
            return format!("invalid group_by '{other}'; use provider, project, or session_type")
        }
    };
    serde_json::to_string_pretty(&search::compare_session_periods_in_scope(
        archive_dir,
        first_from,
        first_to,
        second_from,
        second_to,
        breakdown,
        allowed_ids,
    ))
    .unwrap_or_default()
}

#[cfg(test)]
#[path = "archive_analysis_tests.rs"]
mod tests;
