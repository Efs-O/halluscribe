// HalluScribe - durable session redaction: ledger + apply/preview operations.

use super::index::{find_session, set_secret_flags};
use super::ArchiveError;
use crate::scanner::secrets::scan_for_secrets;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const LEDGER_FILE: &str = "redactions.json";
const MIN_FIND_LEN: usize = 4;
const DEFAULT_REPLACEMENT: &str = "[REDACTED]";
const EXCERPT_CONTEXT: usize = 60;
const MAX_EXCERPTS: usize = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RedactionRule {
    pub session_id: String,
    pub find: String,
    pub replace: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Ledger {
    rules: Vec<RedactionRule>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RedactionPreview {
    pub occurrences: usize,
    pub excerpts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RedactionOutcome {
    pub replacements: usize,
    pub backup_path: String,
}

/// Load all redaction rules from the ledger. Missing file yields an empty list.
pub fn load_rules(archive_dir: &Path) -> Vec<RedactionRule> {
    load_ledger(archive_dir).rules
}

/// Return the redaction rules that apply to one session, in ledger order.
pub fn rules_for_session(archive_dir: &Path, id: &str) -> Vec<RedactionRule> {
    load_rules(archive_dir)
        .into_iter()
        .filter(|rule| rule.session_id == id)
        .collect()
}

fn load_ledger(archive_dir: &Path) -> Ledger {
    let path = archive_dir.join(LEDGER_FILE);
    if !path.exists() {
        return Ledger::default();
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_rules(archive_dir: &Path, ledger: &Ledger) -> Result<(), ArchiveError> {
    fs::create_dir_all(archive_dir)?;
    fs::write(
        archive_dir.join(LEDGER_FILE),
        serde_json::to_string_pretty(ledger)?,
    )?;
    Ok(())
}

/// Apply a list of redaction rules to `text` in order, plain substring replacement.
pub fn apply_rules(text: &str, rules: &[RedactionRule]) -> String {
    let mut out = text.to_string();
    for rule in rules {
        out = out.replace(&rule.find, &rule.replace);
    }
    out
}

fn validate_find(find: &str) -> Result<(), ArchiveError> {
    if find.trim().is_empty() {
        return Err(ArchiveError::Invalid(
            "redaction text must not be empty".to_string(),
        ));
    }
    if find.len() < MIN_FIND_LEN {
        return Err(ArchiveError::Invalid(format!(
            "redaction text must be at least {MIN_FIND_LEN} characters"
        )));
    }
    Ok(())
}

fn session_md_path(
    archive_dir: &Path,
    session_id: &str,
) -> Result<std::path::PathBuf, ArchiveError> {
    let entry = find_session(archive_dir, session_id).ok_or_else(|| {
        ArchiveError::Invalid(format!("session '{session_id}' not found in archive"))
    })?;
    Ok(archive_dir.join(&entry.archive_path))
}

fn build_excerpts(text: &str, find: &str) -> Vec<String> {
    let mut excerpts = Vec::new();
    let mut search_start = 0usize;
    while let Some(rel_pos) = text[search_start..].find(find) {
        let start = search_start + rel_pos;
        let end = start + find.len();

        let ctx_start = floor_char_boundary(text, start.saturating_sub(EXCERPT_CONTEXT));
        let ctx_end = ceil_char_boundary(text, (end + EXCERPT_CONTEXT).min(text.len()));

        let mut excerpt = String::new();
        excerpt.push_str(&text[ctx_start..start]);
        excerpt.push_str(">>>");
        excerpt.push_str(&text[start..end]);
        excerpt.push_str("<<<");
        excerpt.push_str(&text[end..ctx_end]);
        excerpts.push(excerpt);

        search_start = end;
        if excerpts.len() >= MAX_EXCERPTS {
            break;
        }
    }
    excerpts
}

fn count_occurrences(text: &str, find: &str) -> usize {
    let mut count = 0;
    let mut search_start = 0usize;
    while let Some(rel_pos) = text[search_start..].find(find) {
        count += 1;
        search_start = search_start + rel_pos + find.len();
    }
    count
}

fn floor_char_boundary(text: &str, mut idx: usize) -> usize {
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_char_boundary(text: &str, mut idx: usize) -> usize {
    while idx < text.len() && !text.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

/// Preview a redaction: count occurrences and build a handful of excerpts, without
/// modifying anything on disk.
pub fn preview_redaction(
    archive_dir: &Path,
    session_id: &str,
    find: &str,
) -> Result<RedactionPreview, ArchiveError> {
    validate_find(find)?;
    let md_path = session_md_path(archive_dir, session_id)?;
    let text = fs::read_to_string(&md_path)?;
    Ok(RedactionPreview {
        occurrences: count_occurrences(&text, find),
        excerpts: build_excerpts(&text, find),
    })
}

/// Apply a redaction: backup the session markdown, rewrite it with all occurrences of
/// `find` replaced by `replace`, and persist the rule to the ledger so it survives
/// future sweeps of this session.
pub fn apply_redaction(
    archive_dir: &Path,
    session_id: &str,
    find: &str,
    replace: &str,
) -> Result<RedactionOutcome, ArchiveError> {
    validate_find(find)?;
    let replace = if replace.is_empty() {
        DEFAULT_REPLACEMENT
    } else {
        replace
    };

    let md_path = session_md_path(archive_dir, session_id)?;
    let original = fs::read_to_string(&md_path)?;
    let occurrences = count_occurrences(&original, find);
    if occurrences == 0 {
        return Err(ArchiveError::Invalid(
            "text not found in session body".to_string(),
        ));
    }

    let timestamp = Utc::now().format("%Y%m%d%H%M%S");
    let file_name = md_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("session.md");
    let backup_path = md_path.with_file_name(format!("{file_name}.bak-{timestamp}"));
    fs::write(&backup_path, &original)?;

    let updated = original.replace(find, replace);
    fs::write(&md_path, &updated)?;

    let mut ledger = load_ledger(archive_dir);
    let rule = RedactionRule {
        session_id: session_id.to_string(),
        find: find.to_string(),
        replace: replace.to_string(),
    };
    if !ledger.rules.contains(&rule) {
        ledger.rules.push(rule);
        save_rules(archive_dir, &ledger)?;
    }

    // Redacting can remove (or, in principle, introduce) a flagged secret shape;
    // re-scan and update the index immediately so the badge reflects reality
    // without waiting for the next sweep. Runs after the ledger save so a
    // failed index write can never leave a redaction without its durable rule.
    set_secret_flags(archive_dir, session_id, scan_for_secrets(&updated))?;

    Ok(RedactionOutcome {
        replacements: occurrences,
        backup_path: backup_path.to_string_lossy().into_owned(),
    })
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;
