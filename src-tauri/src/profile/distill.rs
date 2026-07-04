// HalluScribe - profile distiller map step: batch sessions into an evidence
// block and extract candidate facts via a `save_profile_facts` tool call.

use super::scope::ProfileScope;
use super::types::{ProfileFact, ProfileSection};
use super::ProfileError;
use crate::archive::IndexEntry;
use serde_json::Value;
use std::borrow::Cow;
use std::fs;
use std::path::Path;

/// Max chars of a session's markdown body included per evidence block. Keeps
/// a 30-session batch well within a single completion's context budget.
const BODY_TRUNCATE_CHARS: usize = 1500;

/// Facts requested per map call; the tool schema also caps this server-side.
const MAX_FACTS_PER_BATCH: usize = 10;

// Generous headroom: session-id-heavy JSON tokenizes at ~2 chars/token, so a
// tight budget truncates the tool-call arguments mid-string (seen live at
// 2048 on the first full distill run). ctx_size dwarfs these values.
const MAP_MAX_TOKENS: u32 = 4096;

const MAP_SYSTEM_PROMPT: &str = "You are distilling a batch of archived AI coding session \
summaries into durable facts about the user (their identity/context, active projects, \
conventions and preferences, recurring problems, communication style, and timeline \
highlights). Call save_profile_facts with up to 10 facts drawn ONLY from the evidence \
provided. Every fact must include the session id(s) it is grounded in in `evidence`. Do \
not invent facts not supported by the text.";

/// Appended to `MAP_SYSTEM_PROMPT` only for the Personal scope (Phase 2c):
/// this scope's sources include personal chat exports, so the map step
/// should also capture non-work facts into `personal_context`.
const PERSONAL_MAP_EXTRA: &str = " Also extract personal facts — interests, life context, \
and non-work preferences — into the personal_context section.";

/// The map step's system prompt for `scope`. Work is byte-identical to the
/// original single-profile prompt; Personal appends one instruction sentence.
fn map_system_prompt(scope: ProfileScope) -> Cow<'static, str> {
    match scope {
        ProfileScope::Work => Cow::Borrowed(MAP_SYSTEM_PROMPT),
        ProfileScope::Personal => Cow::Owned(format!("{MAP_SYSTEM_PROMPT}{PERSONAL_MAP_EXTRA}")),
    }
}

/// Generic inference entry point the map/reduce steps call through: given a
/// system prompt, user content, and tool schema, run one completion and
/// return the parsed tool-call arguments. Production wires this to a warm
/// `gemma::ToolSession`; tests inject a canned closure.
pub type ToolCallFn<'a> =
    dyn Fn(&str, &str, &Value, u32) -> Result<Value, ProfileError> + Send + Sync + 'a;

/// Map one batch of sessions into candidate facts. Returns the facts plus
/// non-fatal warnings (facts skipped for an unknown/out-of-scope section).
pub(super) fn distill_batch(
    archive_dir: &Path,
    batch: &[&IndexEntry],
    scope: ProfileScope,
    tool_call: &ToolCallFn,
) -> Result<(Vec<ProfileFact>, Vec<String>), ProfileError> {
    let user_content = build_evidence_block(archive_dir, batch);
    let result = tool_call(
        &map_system_prompt(scope),
        &user_content,
        &save_profile_facts_tool(scope),
        MAP_MAX_TOKENS,
    )?;
    parse_facts(scope, &result)
}

fn build_evidence_block(archive_dir: &Path, batch: &[&IndexEntry]) -> String {
    batch
        .iter()
        .map(|entry| build_evidence_entry(archive_dir, entry))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

fn build_evidence_entry(archive_dir: &Path, entry: &IndexEntry) -> String {
    let body = read_body(archive_dir, entry);
    let truncated = truncate_chars(&body, BODY_TRUNCATE_CHARS);
    let tags = entry
        .error_tags
        .iter()
        .chain(entry.topic_tags.iter())
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Session id: {id}\nTitle: {title}\nDate: {date}\nProject: {project}\nTool: {tool}\n\
         Type: {session_type}\nTags: {tags}\n\nBody:\n{truncated}",
        id = entry.id,
        title = entry.title,
        date = entry.date,
        project = entry.project,
        tool = entry.tool,
        session_type = entry.session_type,
    )
}

fn read_body(archive_dir: &Path, entry: &IndexEntry) -> String {
    fs::read_to_string(archive_dir.join(&entry.archive_path)).unwrap_or_default()
}

/// Truncate to at most `max_chars` *characters* (not bytes), so multi-byte
/// UTF-8 content is never cut mid-codepoint.
fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn save_profile_facts_tool(scope: ProfileScope) -> Value {
    let section_keys: Vec<&'static str> = scope
        .sections()
        .iter()
        .map(ProfileSection::as_str)
        .collect();
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "save_profile_facts",
            "description": "Save up to 10 distilled facts about the user extracted from this batch of sessions.",
            "parameters": {
                "type": "object",
                "properties": {
                    "facts": {
                        "type": "array",
                        "maxItems": MAX_FACTS_PER_BATCH,
                        "items": {
                            "type": "object",
                            "properties": {
                                "section": {
                                    "type": "string",
                                    "enum": section_keys
                                },
                                "fact": { "type": "string" },
                                "evidence": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                },
                                "date": { "type": "string" }
                            },
                            "required": ["section", "fact", "evidence", "date"]
                        }
                    }
                },
                "required": ["facts"]
            }
        }
    })
}

/// Resolve a model-emitted section key: exact key first, then a fixed alias
/// table for the near-miss keys small models emit in practice.
fn resolve_section(key: &str) -> Option<ProfileSection> {
    ProfileSection::from_key(key).or_else(|| match key.to_lowercase().as_str() {
        "preferences" | "convention" | "conventions_preferences" => {
            Some(ProfileSection::Conventions)
        }
        "personal" | "interests" | "life_context" => Some(ProfileSection::PersonalContext),
        "problems" | "recurring" => Some(ProfileSection::RecurringProblems),
        "communication" | "style" => Some(ProfileSection::CommunicationStyle),
        "project" | "active_projects" => Some(ProfileSection::Projects),
        "context" => Some(ProfileSection::Identity),
        "highlights" => Some(ProfileSection::Timeline),
        _ => None,
    })
}

/// Parse the map tool-call output tolerantly: a fact with an unknown or
/// out-of-scope section is skipped with a warning (never fails the batch);
/// a missing/invalid `fact` string still fails the batch, since that means
/// the JSON itself was truncated or malformed.
fn parse_facts(
    scope: ProfileScope,
    value: &Value,
) -> Result<(Vec<ProfileFact>, Vec<String>), ProfileError> {
    let items = value["facts"]
        .as_array()
        .ok_or_else(|| ProfileError::BadToolCall("missing facts array".to_string()))?;
    let mut facts = Vec::new();
    let mut warnings = Vec::new();
    for item in items.iter().take(MAX_FACTS_PER_BATCH) {
        let section_str = item["section"]
            .as_str()
            .ok_or_else(|| ProfileError::BadToolCall("fact missing string section".to_string()))?;
        let fact = item["fact"]
            .as_str()
            .ok_or_else(|| ProfileError::BadToolCall("fact missing string fact".to_string()))?
            .to_string();
        let section = match resolve_section(section_str) {
            Some(section) if scope.sections().contains(&section) => section,
            _ => {
                warnings.push(format!("skipped fact with unknown section '{section_str}'"));
                continue;
            }
        };
        let evidence = item["evidence"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let date = item["date"].as_str().unwrap_or("").to_string();
        facts.push(ProfileFact {
            section,
            fact,
            evidence,
            date,
        });
    }
    Ok((facts, warnings))
}

#[cfg(test)]
#[path = "distill_tests.rs"]
mod tests;
