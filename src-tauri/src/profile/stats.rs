// HalluScribe - deterministic per-project activity stats for profile.md's
// auto-generated "Project Activity" section. Computed straight from an
// already scope-filtered index slice, never touched by the LLM. See
// docs/internal/PROFILE_QUALITY_PLAN.md Phase 2.

use crate::archive::IndexEntry;
use chrono::NaiveDate;
use std::collections::HashMap;

/// Heading text (without the `##` prefix) for the auto-generated section.
/// Shared with `writer.rs` (which emits it) and `parse_md.rs` (which must
/// skip it so it is never fed back into the merge prompts), mirroring the
/// `EMPTY_SECTION_PLACEHOLDER` pattern.
pub(super) const PROJECT_ACTIVITY_HEADING: &str = "Project Activity (auto-generated)";

/// A project counts as `active` when last seen within this many days of the
/// generation date.
const ACTIVE_MAX_DAYS: i64 = 60;
/// Beyond `ACTIVE_MAX_DAYS` and up to this many days, a project is `quiet`;
/// beyond it, `dormant`.
const QUIET_MAX_DAYS: i64 = 180;
/// Row cap so a many-project archive doesn't bloat the profile.
const MAX_ROWS: usize = 40;

/// Bucket label for entries whose project name is a date artifact rather
/// than a real project. Codex stores rollouts under `sessions/YYYY/MM/DD/`,
/// so `project_from_parent` records the day-of-month ("03", "13") as the
/// project for every Codex session; without folding, those buckets crowd
/// out real projects (22 of 40 rows on the live archive).
const UNLABELED_PROJECT: &str = "(unlabeled)";

/// Display name for grouping: purely numeric project names are date
/// artifacts, folded into one honest bucket; everything else is untouched.
fn display_project(name: &str) -> &str {
    if !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()) {
        UNLABELED_PROJECT
    } else {
        name
    }
}

struct ProjectRow {
    project: String,
    sessions: usize,
    first_seen: String,
    last_seen: String,
    status: &'static str,
}

/// Render the `## Project Activity (auto-generated)` markdown table from an
/// already scope-filtered entry list (the caller reuses `select::select_sources`
/// with no watermark to get the full in-scope set — see `profile/mod.rs`).
/// `generated_date` is `YYYY-MM-DD`, the same date stamped in the profile's
/// header. Empty input yields an empty string (no heading, no empty table).
pub(super) fn project_activity_markdown(entries: &[&IndexEntry], generated_date: &str) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let mut by_project: HashMap<&str, (usize, &str, &str)> = HashMap::new();
    for entry in entries {
        let stats = by_project
            .entry(display_project(&entry.project))
            .or_insert((0, entry.date.as_str(), entry.date.as_str()));
        stats.0 += 1;
        if entry.date.as_str() < stats.1 {
            stats.1 = entry.date.as_str();
        }
        if entry.date.as_str() > stats.2 {
            stats.2 = entry.date.as_str();
        }
    }

    let generation = parse_date(generated_date);
    let mut rows: Vec<ProjectRow> = by_project
        .into_iter()
        .map(|(project, (sessions, first_seen, last_seen))| ProjectRow {
            project: project.to_string(),
            sessions,
            first_seen: first_seen.to_string(),
            last_seen: last_seen.to_string(),
            status: status_for(last_seen, generation),
        })
        .collect();
    // Session count desc; tie-break project name asc for determinism.
    rows.sort_by(|a, b| {
        b.sessions
            .cmp(&a.sessions)
            .then_with(|| a.project.cmp(&b.project))
    });

    let total = rows.len();
    rows.truncate(MAX_ROWS);

    let mut out = format!("## {PROJECT_ACTIVITY_HEADING}\n\n");
    out.push_str("| project | sessions | first seen | last seen | status |\n");
    out.push_str("|---|---|---|---|---|\n");
    for row in &rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            row.project, row.sessions, row.first_seen, row.last_seen, row.status
        ));
    }
    if total > MAX_ROWS {
        out.push_str(&format!("\n(+{} more projects)\n", total - MAX_ROWS));
    }
    out
}

/// Status bucket for a project's `last_seen` date relative to `generation`.
/// An unparsable date (shouldn't happen — the index stores `YYYY-MM-DD`
/// strings) fails safe to `active` rather than panicking or misclassifying
/// as dormant.
fn status_for(last_seen: &str, generation: Option<NaiveDate>) -> &'static str {
    let (Some(generation), Some(last_seen)) = (generation, parse_date(last_seen)) else {
        return "active";
    };
    let days = (generation - last_seen).num_days();
    if days <= ACTIVE_MAX_DAYS {
        "active"
    } else if days <= QUIET_MAX_DAYS {
        "quiet"
    } else {
        "dormant"
    }
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

#[cfg(test)]
#[path = "stats_tests.rs"]
mod tests;
