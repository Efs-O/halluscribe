// HalluScribe - briefing archive filtering and session selection helpers.

use super::prompt::{build_content, build_header, session_datetime};
use crate::archive::{read_sessions, IndexEntry};
use crate::search::fold_for_search;
use std::path::Path;

#[derive(Clone)]
pub struct BriefingFilters {
    pub date_from: Option<chrono::NaiveDate>,
    pub date_to: Option<chrono::NaiveDate>,
    pub fill_min: Option<f64>,
    pub fill_max: Option<f64>,
    pub keyword: Option<String>,
}

pub enum BriefingScope {
    ArchiveWide,
    FilterBased(BriefingFilters),
    SelectedSessionIds(Vec<String>),
}

pub fn collect_briefing_sessions(
    archive_dir: &Path,
    scope: &BriefingScope,
    fallback_count: usize,
) -> Result<(String, String, usize), String> {
    let mut sessions = read_sessions(archive_dir);
    if sessions.is_empty() {
        return Err("no sessions archived yet".to_string());
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(session_datetime(s)));

    match scope {
        BriefingScope::ArchiveWide => {
            let slice: Vec<_> = sessions.iter().take(fallback_count).collect();
            let n = slice.len();
            Ok((build_content(archive_dir, &slice), build_header(&slice), n))
        }
        BriefingScope::FilterBased(filters) => {
            let has_filters = filters.date_from.is_some()
                || filters.date_to.is_some()
                || filters.fill_min.is_some()
                || filters.fill_max.is_some()
                || filters.keyword.as_deref().is_some_and(|k| !k.is_empty());

            let slice: Vec<_> = sessions
                .iter()
                .filter(|s| session_matches(s, filters, archive_dir))
                .collect();
            if !slice.is_empty() && has_filters {
                let n = slice.len();
                return Ok((build_content(archive_dir, &slice), build_header(&slice), n));
            }

            let slice: Vec<_> = sessions.iter().take(fallback_count).collect();
            let n = slice.len();
            Ok((build_content(archive_dir, &slice), build_header(&slice), n))
        }
        BriefingScope::SelectedSessionIds(session_ids) => {
            let mut selected = Vec::with_capacity(session_ids.len());
            let mut missing = Vec::new();
            for session_id in session_ids {
                match sessions.iter().find(|entry| entry.id == *session_id) {
                    Some(entry) => selected.push(entry),
                    None => missing.push(session_id.clone()),
                }
            }

            if !missing.is_empty() {
                let preview = missing.into_iter().take(3).collect::<Vec<_>>().join(", ");
                return Err(format!(
                    "selected briefing scope contains missing session IDs: {preview}"
                ));
            }

            if selected.is_empty() {
                return Err("no selected sessions were provided".to_string());
            }

            selected.sort_by_key(|entry| std::cmp::Reverse(session_datetime(entry)));
            let n = selected.len();
            Ok((
                build_content(archive_dir, &selected),
                build_header(&selected),
                n,
            ))
        }
    }
}

fn session_matches(entry: &IndexEntry, filters: &BriefingFilters, archive_dir: &Path) -> bool {
    let entry_date = chrono::NaiveDate::parse_from_str(&entry.date, "%Y-%m-%d").ok();
    if let (Some(from), Some(date)) = (filters.date_from, entry_date) {
        if date < from {
            return false;
        }
    }
    if let (Some(to), Some(date)) = (filters.date_to, entry_date) {
        if date > to {
            return false;
        }
    }
    if let Some(min) = filters.fill_min {
        if entry.fill_pct < min {
            return false;
        }
    }
    if let Some(max) = filters.fill_max {
        if entry.fill_pct > max {
            return false;
        }
    }
    if let Some(keyword) = &filters.keyword {
        if !keyword.is_empty() && !keyword_matches(entry, archive_dir, keyword) {
            return false;
        }
    }
    true
}

/// Case- and Greek-accent-insensitive keyword match (D9): the keyword and the
/// index fields go through the same `fold_for_search` the body cache uses, so
/// `καλημερα` finds `Καλημέρα` in the title or the body alike.
fn keyword_matches(entry: &IndexEntry, archive_dir: &Path, keyword: &str) -> bool {
    let needle = fold_for_search(keyword);
    let index_haystack = fold_for_search(&format!(
        "{} {} {} {}",
        entry.title,
        entry.session_type,
        entry.error_tags.join(" "),
        entry.topic_tags.join(" "),
    ));
    if index_haystack.contains(&needle) {
        return true;
    }
    crate::search::body_contains(archive_dir, entry, &needle)
}

#[cfg(test)]
#[path = "filters_tests.rs"]
mod tests;
