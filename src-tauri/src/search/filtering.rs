// HalluScribe - search predicate helpers: match an IndexEntry against SearchParams.

use super::tokenize::{parse_query, ParsedQuery};
use super::SearchParams;
use crate::archive::IndexEntry;
use std::path::Path;

// Per-token field weights for ranking (best field only, see `metadata_best_weight`
// / `phrase_score`). Field set is title/error_tags/topic_tags/body - `project`
// is deliberately NOT part of this matcher: SearchParams has a dedicated
// `project` filter for this path, while `content::matches_fulltext` (the
// session-list box, no structured filters) does include project in its query.
const WEIGHT_TITLE: u32 = 8;
const WEIGHT_TOPIC_TAG: u32 = 4;
const WEIGHT_ERROR_TAG: u32 = 3;
const WEIGHT_BODY: u32 = 1;

pub(super) fn matches_params(
    archive_dir: &Path,
    entry: &IndexEntry,
    params: &SearchParams,
) -> bool {
    match_score(archive_dir, entry, params).is_some()
}

/// Score-returning match: `None` means the entry fails a structured filter
/// (date/tags/project/tool) or the query itself; `Some(score)` means it
/// passes, with `score` reflecting only the query's field-weighted relevance
/// (0 when no query was given - there is nothing to rank on).
pub(super) fn match_score(
    archive_dir: &Path,
    entry: &IndexEntry,
    params: &SearchParams,
) -> Option<u32> {
    if !matches_date_from(entry, params)
        || !matches_date_to(entry, params)
        || !matches_tags(entry, params)
        || !matches_project(entry, params)
        || !matches_tool(entry, params)
    {
        return None;
    }
    query_score(archive_dir, entry, params)
}

fn query_score(archive_dir: &Path, entry: &IndexEntry, params: &SearchParams) -> Option<u32> {
    let Some(query) = params.query.as_ref() else {
        return Some(0);
    };

    match parse_query(query) {
        ParsedQuery::Phrase(phrase) => phrase_score(archive_dir, entry, &phrase),
        ParsedQuery::Tokens(tokens) => tokens_score(archive_dir, entry, &tokens),
    }
}

/// Exact-substring mode (today's behaviour): best single field weight, or
/// `None` if the phrase appears nowhere.
fn phrase_score(archive_dir: &Path, entry: &IndexEntry, needle: &str) -> Option<u32> {
    match metadata_best_weight(entry, needle) {
        Some(weight) => Some(weight),
        None => super::body_contains(archive_dir, entry, needle).then_some(WEIGHT_BODY),
    }
}

/// AND-of-tokens mode: every token must match somewhere. Cheap metadata
/// checks first; the `.md` body is read at most once, only for tokens still
/// unmatched after the metadata pass (per-token short-circuit, plan §3.1(4)).
fn tokens_score(archive_dir: &Path, entry: &IndexEntry, tokens: &[String]) -> Option<u32> {
    let mut total = 0u32;
    let mut unmatched: Vec<&str> = Vec::new();
    for token in tokens {
        match metadata_best_weight(entry, token) {
            Some(weight) => total += weight,
            None => unmatched.push(token.as_str()),
        }
    }
    if unmatched.is_empty() {
        return Some(total);
    }

    let hits = super::body_find(archive_dir, entry, &unmatched);
    if hits.iter().all(|hit| *hit) {
        total += WEIGHT_BODY * unmatched.len() as u32;
        Some(total)
    } else {
        None
    }
}

/// Best (highest-weight) metadata field containing `needle`, if any. No disk read.
fn metadata_best_weight(entry: &IndexEntry, needle: &str) -> Option<u32> {
    if entry.title.to_lowercase().contains(needle) {
        return Some(WEIGHT_TITLE);
    }
    if entry
        .topic_tags
        .iter()
        .any(|tag| tag.to_lowercase().contains(needle))
    {
        return Some(WEIGHT_TOPIC_TAG);
    }
    if entry
        .error_tags
        .iter()
        .any(|tag| tag.to_lowercase().contains(needle))
    {
        return Some(WEIGHT_ERROR_TAG);
    }
    None
}

fn matches_date_from(entry: &IndexEntry, params: &SearchParams) -> bool {
    let Some(from) = params.date_from.as_ref() else {
        return true;
    };

    // TODO(locale): date display and input use YYYY-MM-DD (ISO) throughout. Future: read system
    // locale so US users see MM/DD/YYYY and Greek users see DD/MM/YYYY HH:MM (24h). For now,
    // all date params must be YYYY-MM-DD - the tool schema enforces this for Gemma tool calls.
    matches_date_bound(
        &entry.date,
        from,
        |entry_date, bound| entry_date >= bound,
        |entry_date, bound| entry_date >= bound,
    )
}

fn matches_date_to(entry: &IndexEntry, params: &SearchParams) -> bool {
    let Some(to) = params.date_to.as_ref() else {
        return true;
    };

    matches_date_bound(
        &entry.date,
        to,
        |entry_date, bound| entry_date <= bound,
        |entry_date, bound| entry_date <= bound,
    )
}

fn matches_date_bound<F, G>(entry_date: &str, bound: &str, compare: F, fallback: G) -> bool
where
    F: FnOnce(chrono::NaiveDate, chrono::NaiveDate) -> bool,
    G: FnOnce(&str, &str) -> bool,
{
    let entry = chrono::NaiveDate::parse_from_str(entry_date, "%Y-%m-%d");
    let bound_date = chrono::NaiveDate::parse_from_str(bound, "%Y-%m-%d");
    match (entry, bound_date) {
        (Ok(entry_date), Ok(bound_date)) => compare(entry_date, bound_date),
        _ => fallback(entry_date, bound),
    }
}

fn matches_tags(entry: &IndexEntry, params: &SearchParams) -> bool {
    let Some(tags) = params.tags.as_ref() else {
        return true;
    };

    let all_tags: Vec<&str> = entry
        .error_tags
        .iter()
        .chain(entry.topic_tags.iter())
        .map(String::as_str)
        .collect();
    tags.iter().all(|tag| all_tags.contains(&tag.as_str()))
}

fn matches_project(entry: &IndexEntry, params: &SearchParams) -> bool {
    let Some(project) = params.project.as_ref() else {
        return true;
    };

    entry
        .project
        .to_lowercase()
        .contains(&project.to_lowercase())
}

fn matches_tool(entry: &IndexEntry, params: &SearchParams) -> bool {
    let Some(tool) = params.tool.as_ref() else {
        return true;
    };

    let tool_lc = tool.to_lowercase();
    match tool_lc.as_str() {
        "claude_code" => entry.tool.to_lowercase().contains("claude"),
        "codex" => entry.tool.to_lowercase().contains("codex"),
        "continue" => entry.tool.to_lowercase().contains("continue"),
        "forge" => entry.tool.to_lowercase().contains("forge"),
        other => entry.tool.to_lowercase().contains(other),
    }
}
