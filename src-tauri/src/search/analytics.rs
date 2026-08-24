// HalluScribe - exact archive analytics for the chat tool surface.

use super::{matches_params, SearchParams};
use crate::archive::{read_sessions, IndexEntry};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ProviderSessionCount {
    pub provider: String,
    pub session_count: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ProviderSessionCounts {
    pub searched: usize,
    pub total_sessions: usize,
    pub providers: Vec<ProviderSessionCount>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FacetCount {
    pub value: String,
    pub session_count: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ArchiveFacets {
    pub searched: usize,
    pub total_sessions: usize,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
    pub providers: Vec<FacetCount>,
    pub projects: Vec<FacetCount>,
    pub session_types: Vec<FacetCount>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeriodBreakdown {
    Provider,
    Project,
    SessionType,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct PeriodSummary {
    pub date_from: String,
    pub date_to: String,
    pub total_sessions: usize,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct PeriodGroupChange {
    pub value: String,
    pub first_period_sessions: usize,
    pub second_period_sessions: usize,
    pub absolute_change: i64,
    pub percentage_change: Option<f64>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct PeriodComparison {
    pub searched: usize,
    pub group_by: String,
    pub first_period: PeriodSummary,
    pub second_period: PeriodSummary,
    pub absolute_change: i64,
    pub percentage_change: Option<f64>,
    pub groups: Vec<PeriodGroupChange>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ArchiveHealth {
    pub indexed_sessions: usize,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
    pub summary_files_present: usize,
    pub summary_files_missing: usize,
    pub raw_copies_present: usize,
    pub raw_copies_missing: usize,
}

pub fn count_sessions_by_provider_in_scope(
    archive_dir: &Path,
    date_from: Option<&str>,
    date_to: Option<&str>,
    allowed_ids: Option<&HashSet<String>>,
) -> ProviderSessionCounts {
    let (searched, entries) = date_filtered_entries(archive_dir, date_from, date_to, allowed_ids);
    let providers = facet_counts(&entries, |entry| entry.tool.clone())
        .into_iter()
        .map(|entry| ProviderSessionCount {
            provider: entry.value,
            session_count: entry.session_count,
        })
        .collect();
    ProviderSessionCounts {
        searched,
        total_sessions: entries.len(),
        providers,
    }
}

pub fn archive_facets_in_scope(
    archive_dir: &Path,
    date_from: Option<&str>,
    date_to: Option<&str>,
    allowed_ids: Option<&HashSet<String>>,
) -> ArchiveFacets {
    let (searched, entries) = date_filtered_entries(archive_dir, date_from, date_to, allowed_ids);
    let (first_date, last_date) = date_bounds(&entries);
    ArchiveFacets {
        searched,
        total_sessions: entries.len(),
        first_date,
        last_date,
        providers: facet_counts(&entries, |entry| entry.tool.clone()),
        projects: facet_counts(&entries, |entry| entry.project.clone()),
        session_types: facet_counts(&entries, |entry| entry.session_type.clone()),
    }
}

pub fn compare_session_periods_in_scope(
    archive_dir: &Path,
    first_from: &str,
    first_to: &str,
    second_from: &str,
    second_to: &str,
    breakdown: PeriodBreakdown,
    allowed_ids: Option<&HashSet<String>>,
) -> PeriodComparison {
    let (searched, first_entries) =
        date_filtered_entries(archive_dir, Some(first_from), Some(first_to), allowed_ids);
    let (_, second_entries) =
        date_filtered_entries(archive_dir, Some(second_from), Some(second_to), allowed_ids);
    let first_counts = grouped_counts(&first_entries, breakdown);
    let second_counts = grouped_counts(&second_entries, breakdown);
    let keys: BTreeSet<String> = first_counts
        .keys()
        .chain(second_counts.keys())
        .cloned()
        .collect();
    let groups = keys
        .into_iter()
        .map(|value| {
            let first_period_sessions = first_counts.get(&value).copied().unwrap_or(0);
            let second_period_sessions = second_counts.get(&value).copied().unwrap_or(0);
            PeriodGroupChange {
                value,
                first_period_sessions,
                second_period_sessions,
                absolute_change: second_period_sessions as i64 - first_period_sessions as i64,
                percentage_change: percentage_change(first_period_sessions, second_period_sessions),
            }
        })
        .collect();
    let first_total = first_entries.len();
    let second_total = second_entries.len();

    PeriodComparison {
        searched,
        group_by: breakdown_label(breakdown).to_string(),
        first_period: PeriodSummary {
            date_from: first_from.to_string(),
            date_to: first_to.to_string(),
            total_sessions: first_total,
        },
        second_period: PeriodSummary {
            date_from: second_from.to_string(),
            date_to: second_to.to_string(),
            total_sessions: second_total,
        },
        absolute_change: second_total as i64 - first_total as i64,
        percentage_change: percentage_change(first_total, second_total),
        groups,
    }
}

pub fn archive_health_in_scope(
    archive_dir: &Path,
    allowed_ids: Option<&HashSet<String>>,
) -> ArchiveHealth {
    let entries: Vec<IndexEntry> = read_sessions(archive_dir)
        .into_iter()
        .filter(|entry| in_scope(entry, allowed_ids))
        .collect();
    let summary_files_present = entries
        .iter()
        .filter(|entry| archive_dir.join(&entry.archive_path).is_file())
        .count();
    let raw_copies_present = entries
        .iter()
        .filter(|entry| !entry.raw_path.is_empty() && archive_dir.join(&entry.raw_path).is_file())
        .count();
    let (first_date, last_date) = date_bounds(&entries);
    let indexed_sessions = entries.len();

    ArchiveHealth {
        indexed_sessions,
        first_date,
        last_date,
        summary_files_present,
        summary_files_missing: indexed_sessions - summary_files_present,
        raw_copies_present,
        raw_copies_missing: indexed_sessions - raw_copies_present,
    }
}

fn date_filtered_entries(
    archive_dir: &Path,
    date_from: Option<&str>,
    date_to: Option<&str>,
    allowed_ids: Option<&HashSet<String>>,
) -> (usize, Vec<IndexEntry>) {
    let params = SearchParams {
        date_from: date_from.map(str::to_string),
        date_to: date_to.map(str::to_string),
        ..SearchParams::default()
    };
    let sessions = read_sessions(archive_dir);
    let searched = sessions
        .iter()
        .filter(|entry| in_scope(entry, allowed_ids))
        .count();
    let entries = sessions
        .into_iter()
        .filter(|entry| in_scope(entry, allowed_ids))
        .filter(|entry| matches_params(archive_dir, entry, &params))
        .collect();
    (searched, entries)
}

fn in_scope(entry: &IndexEntry, allowed_ids: Option<&HashSet<String>>) -> bool {
    allowed_ids
        .map(|ids| ids.contains(&entry.id))
        .unwrap_or(true)
}

fn facet_counts(entries: &[IndexEntry], label: impl Fn(&IndexEntry) -> String) -> Vec<FacetCount> {
    let mut counts = HashMap::<String, usize>::new();
    for entry in entries {
        *counts.entry(label(entry)).or_default() += 1;
    }
    let mut facets: Vec<FacetCount> = counts
        .into_iter()
        .map(|(value, session_count)| FacetCount {
            value,
            session_count,
        })
        .collect();
    facets.sort_by(|left, right| {
        right
            .session_count
            .cmp(&left.session_count)
            .then_with(|| left.value.cmp(&right.value))
    });
    facets
}

fn grouped_counts(entries: &[IndexEntry], breakdown: PeriodBreakdown) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for entry in entries {
        *counts.entry(breakdown_value(entry, breakdown)).or_default() += 1;
    }
    counts
}

fn breakdown_value(entry: &IndexEntry, breakdown: PeriodBreakdown) -> String {
    match breakdown {
        PeriodBreakdown::Provider => entry.tool.clone(),
        PeriodBreakdown::Project => entry.project.clone(),
        PeriodBreakdown::SessionType => entry.session_type.clone(),
    }
}

fn breakdown_label(breakdown: PeriodBreakdown) -> &'static str {
    match breakdown {
        PeriodBreakdown::Provider => "provider",
        PeriodBreakdown::Project => "project",
        PeriodBreakdown::SessionType => "session_type",
    }
}

fn percentage_change(first: usize, second: usize) -> Option<f64> {
    (first > 0).then(|| (second as f64 - first as f64) * 100.0 / first as f64)
}

fn date_bounds(entries: &[IndexEntry]) -> (Option<String>, Option<String>) {
    let first_date = entries.iter().map(|entry| &entry.date).min().cloned();
    let last_date = entries.iter().map(|entry| &entry.date).max().cloned();
    (first_date, last_date)
}
