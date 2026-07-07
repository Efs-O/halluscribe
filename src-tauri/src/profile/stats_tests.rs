// HalluScribe - unit tests for the project-activity stats module (stats.rs):
// per-project counts/date-ranges, status thresholds, sort/cap, empty input.

use super::*;

fn entry(id: &str, project: &str, date: &str) -> IndexEntry {
    IndexEntry {
        id: id.to_string(),
        project: project.to_string(),
        date: date.to_string(),
        title: format!("Session {id}"),
        tool: "Claude Code".to_string(),
        fill_pct: 90.0,
        session_timestamp: format!("{date}T00:00:00+00:00"),
        updated_at: String::new(),
        session_type: "building".to_string(),
        error_tags: Vec::new(),
        topic_tags: Vec::new(),
        archive_path: format!("sessions/{project}/{id}.md"),
        source_jsonl: String::new(),
        source_size_bytes: 0,
        provider: "claude_code".to_string(),
        fill_estimated: false,
        transcript_hash: String::new(),
        secret_flags: Vec::new(),
        raw_path: String::new(),
    }
}

#[test]
fn empty_index_yields_empty_string() {
    let entries: Vec<&IndexEntry> = Vec::new();
    assert_eq!(project_activity_markdown(&entries, "2026-07-07"), "");
}

#[test]
fn counts_first_and_last_seen_per_project() {
    let entries = [
        entry("a", "proj-x", "2026-05-01"),
        entry("b", "proj-x", "2026-06-15"),
        entry("c", "proj-x", "2026-04-10"),
        entry("d", "proj-y", "2026-06-01"),
    ];
    let refs: Vec<&IndexEntry> = entries.iter().collect();
    let md = project_activity_markdown(&refs, "2026-07-07");
    assert!(md.contains(&format!("## {PROJECT_ACTIVITY_HEADING}")));
    assert!(md.contains("| proj-x | 3 | 2026-04-10 | 2026-06-15 | active |"));
    assert!(md.contains("| proj-y | 1 | 2026-06-01 | 2026-06-01 | active |"));
}

#[test]
fn status_active_at_exactly_60_days() {
    let entries = [entry("a", "proj", "2026-05-08")]; // 60 days before 2026-07-07
    let refs: Vec<&IndexEntry> = entries.iter().collect();
    let md = project_activity_markdown(&refs, "2026-07-07");
    assert!(md.contains("| proj | 1 | 2026-05-08 | 2026-05-08 | active |"));
}

#[test]
fn status_quiet_at_exactly_61_days() {
    let entries = [entry("a", "proj", "2026-05-07")]; // 61 days before 2026-07-07
    let refs: Vec<&IndexEntry> = entries.iter().collect();
    let md = project_activity_markdown(&refs, "2026-07-07");
    assert!(md.contains("| proj | 1 | 2026-05-07 | 2026-05-07 | quiet |"));
}

#[test]
fn status_quiet_at_exactly_180_days() {
    let entries = [entry("a", "proj", "2026-01-08")]; // 180 days before 2026-07-07
    let refs: Vec<&IndexEntry> = entries.iter().collect();
    let md = project_activity_markdown(&refs, "2026-07-07");
    assert!(md.contains("| proj | 1 | 2026-01-08 | 2026-01-08 | quiet |"));
}

#[test]
fn status_dormant_at_exactly_181_days() {
    let entries = [entry("a", "proj", "2026-01-07")]; // 181 days before 2026-07-07
    let refs: Vec<&IndexEntry> = entries.iter().collect();
    let md = project_activity_markdown(&refs, "2026-07-07");
    assert!(md.contains("| proj | 1 | 2026-01-07 | 2026-01-07 | dormant |"));
}

#[test]
fn sort_order_is_session_count_desc_then_project_name_asc() {
    let entries = [
        entry("a", "zebra", "2026-07-01"),
        entry("b", "zebra", "2026-07-01"),
        entry("c", "apple", "2026-07-01"),
        entry("d", "apple", "2026-07-01"),
        entry("e", "mango", "2026-07-01"),
    ];
    let refs: Vec<&IndexEntry> = entries.iter().collect();
    let md = project_activity_markdown(&refs, "2026-07-07");
    let apple_pos = md.find("| apple |").unwrap();
    let zebra_pos = md.find("| zebra |").unwrap();
    let mango_pos = md.find("| mango |").unwrap();
    // apple and zebra both have 2 sessions - tie-broken alphabetically, so
    // apple (2) comes before zebra (2), and both come before mango (1).
    assert!(apple_pos < zebra_pos);
    assert!(zebra_pos < mango_pos);
}

#[test]
fn caps_at_forty_rows_with_trailing_more_line() {
    let entries: Vec<IndexEntry> = (0..45)
        .map(|i| entry(&format!("s{i}"), &format!("proj-{i:02}"), "2026-07-01"))
        .collect();
    let refs: Vec<&IndexEntry> = entries.iter().collect();
    let md = project_activity_markdown(&refs, "2026-07-07");
    let row_count = md
        .lines()
        .filter(|line| line.starts_with("| proj-"))
        .count();
    assert_eq!(row_count, 40);
    assert!(md.contains("(+5 more projects)"));
}

#[test]
fn no_more_line_when_under_cap() {
    let entries = [entry("a", "proj", "2026-07-01")];
    let refs: Vec<&IndexEntry> = entries.iter().collect();
    let md = project_activity_markdown(&refs, "2026-07-07");
    assert!(!md.contains("more projects"));
}

#[test]
fn unparsable_generation_date_fails_safe_to_active() {
    let entries = [entry("a", "proj", "2026-01-01")];
    let refs: Vec<&IndexEntry> = entries.iter().collect();
    let md = project_activity_markdown(&refs, "not-a-date");
    assert!(md.contains("| proj | 1 | 2026-01-01 | 2026-01-01 | active |"));
}
