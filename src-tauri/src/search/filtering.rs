use super::SearchParams;
use crate::archive::IndexEntry;

pub(super) fn matches_params(entry: &IndexEntry, params: &SearchParams) -> bool {
    matches_query(entry, params)
        && matches_date_from(entry, params)
        && matches_date_to(entry, params)
        && matches_tags(entry, params)
        && matches_project(entry, params)
        && matches_tool(entry, params)
}

fn matches_query(entry: &IndexEntry, params: &SearchParams) -> bool {
    let Some(query) = params.query.as_ref() else {
        return true;
    };

    let query_lc = query.to_lowercase();
    let in_title = entry.title.to_lowercase().contains(&query_lc);
    let in_error = entry
        .error_tags
        .iter()
        .any(|tag| tag.to_lowercase().contains(&query_lc));
    let in_topic = entry
        .topic_tags
        .iter()
        .any(|tag| tag.to_lowercase().contains(&query_lc));

    in_title || in_error || in_topic
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
