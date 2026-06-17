// HalluScribe - briefing prompt and archive-content builders.

use crate::archive::IndexEntry;
use std::fs;
use std::path::Path;

pub fn briefing_system_prompt(min_word_limit: usize, max_word_limit: usize) -> String {
    format!(
        "You are reviewing a set of archived AI conversation sessions. \
         Write a concise narrative briefing: how many sessions, what was accomplished or discussed \
         in each, recurring themes, unresolved questions, and notable blockers or insights. \
         When sessions contain coding work, mention file names, implementation details, and error patterns. \
         Be specific and use session titles when possible. \
         For each session include its date and context fill percentage. \
         Aim for 200 to 300 words per session. \
         Do NOT group, merge, or combine sessions - every session must have its own paragraph. \
         Total target range: {min_word_limit} to {max_word_limit} words."
    )
}

pub fn build_content(archive_dir: &Path, entries: &[&IndexEntry]) -> String {
    let mut parts = Vec::new();
    for entry in entries {
        let path = archive_dir.join(&entry.archive_path);
        if let Ok(markdown) = fs::read_to_string(&path) {
            parts.push(format!("---\n# {}\n{markdown}", entry.title));
        } else {
            parts.push(format!(
                "---\n# {} ({})\nDate: {}  Tool: {}  Fill: {:.0}%\nTags: {}\n",
                entry.title,
                entry.id,
                entry.date,
                entry.tool,
                entry.fill_pct,
                entry.topic_tags.join(", ")
            ));
        }
    }
    parts.join("\n\n")
}

pub fn session_datetime(entry: &IndexEntry) -> chrono::DateTime<chrono::Utc> {
    if let Some(updated_at) = parse_rfc3339(&entry.updated_at) {
        return updated_at;
    }
    if let Some(session_timestamp) = parse_rfc3339(&entry.session_timestamp) {
        return session_timestamp;
    }

    let filename = entry.archive_path.split('/').next_back().unwrap_or("");
    let parts: Vec<&str> = filename.split('-').collect();
    let (hour, minute, second) = if parts.len() >= 3 {
        (
            parts[0].parse().unwrap_or(0u32),
            parts[1].parse().unwrap_or(0u32),
            parts[2].parse().unwrap_or(0u32),
        )
    } else {
        (0, 0, 0)
    };
    let date_parts: Vec<&str> = entry.date.split('-').collect();
    if date_parts.len() == 3 {
        let (year, month, day) = (
            date_parts[0].parse().unwrap_or(2026i32),
            date_parts[1].parse().unwrap_or(1u32),
            date_parts[2].parse().unwrap_or(1u32),
        );
        chrono::NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, minute, second))
            .map(|naive| naive.and_utc())
            .unwrap_or_else(chrono::Utc::now)
    } else {
        chrono::Utc::now()
    }
}

fn parse_rfc3339(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if value.trim().is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.with_timezone(&chrono::Utc))
}

pub fn build_header(entries: &[&IndexEntry]) -> String {
    let count = entries.len();
    let session_word = if count == 1 { "session" } else { "sessions" };
    let mut dates: Vec<&str> = entries.iter().map(|entry| entry.date.as_str()).collect();
    dates.sort();
    dates.dedup();
    let date_str = match dates.as_slice() {
        [] => String::new(),
        [date] => format_ymd(date),
        [first, .., last] => {
            if first.len() >= 7 && last.len() >= 7 && first[..7] == last[..7] {
                let month = month_abbr(&first[5..7]);
                let first_day: u32 = first[8..10].parse().unwrap_or(0);
                let last_day: u32 = last[8..10].parse().unwrap_or(0);
                format!("{month} {first_day}-{last_day}")
            } else {
                format!("{}-{}", format_ymd(first), format_ymd(last))
            }
        }
    };
    format!("{count} {session_word} - {date_str}")
}

fn format_ymd(date: &str) -> String {
    if date.len() < 10 {
        return date.to_string();
    }
    let month = month_abbr(&date[5..7]);
    let day: u32 = date[8..10].parse().unwrap_or(0);
    format!("{month} {day}")
}

fn month_abbr(month: &str) -> &'static str {
    match month {
        "01" => "Jan",
        "02" => "Feb",
        "03" => "Mar",
        "04" => "Apr",
        "05" => "May",
        "06" => "Jun",
        "07" => "Jul",
        "08" => "Aug",
        "09" => "Sep",
        "10" => "Oct",
        "11" => "Nov",
        "12" => "Dec",
        _ => "???",
    }
}
