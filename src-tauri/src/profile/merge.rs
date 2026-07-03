// HalluScribe - profile distiller reduce step: merge candidate facts (and the
// previous profile.md, if any) into one prose block per section via a
// `save_user_profile` tool call. Contradiction rule: recency wins, but the
// merge prompt asks the model to note the change rather than silently drop
// the old value.

use super::distill::ToolCallFn;
use super::scope::ProfileScope;
use super::types::{ProfileFact, ProfileSection, ProfileSections};
use super::ProfileError;
use serde_json::Value;

/// Above this many serialized characters of candidate facts, consolidate each
/// oversized section with intermediate calls before the final merge, so the
/// reduce call's input stays within a reasonable context budget.
const CONSOLIDATION_THRESHOLD_CHARS: usize = 60_000;

/// Each consolidate call sees at most this many chars of bullet lines. The
/// prompt forbids dropping facts, so consolidated output scales with input;
/// an unbounded input therefore overflows any fixed token budget (seen live:
/// a full-archive run truncated the consolidate tool call mid-JSON). Bounding
/// the input bounds the output: 6K chars condenses safely within
/// `CONSOLIDATE_MAX_TOKENS` even at the observed ~1.4 chars/token for
/// session-id-dense JSON.
const CONSOLIDATE_CHUNK_CHARS: usize = 6_000;

// The final profile targets 3–5K tokens across six sections; budgets below
// output size truncate the tool-call arguments mid-string.
const FINAL_MAX_TOKENS: u32 = 8192;
const CONSOLIDATE_MAX_TOKENS: u32 = 4096;

const MERGE_SYSTEM_PROMPT: &str = "You maintain a durable profile of the user distilled from \
their AI coding session archive. You are given the previous profile.md (if any) and new \
candidate facts grouped by section, newest first. Call save_user_profile with one field per \
section. Rules: recency wins on contradictions, but note the change (e.g. \"previously used \
X, switched to Y around 2026-03\"). Every non-obvious claim must carry session-id references \
like [abc-123] drawn ONLY from the evidence ids given to you - never invent an id. Keep each \
section concise (a CV, not a diary). Projects unseen for more than 12 months belong in \
Timeline, not Active Projects.";

const CONSOLIDATE_SYSTEM_PROMPT: &str = "You are compressing a long list of candidate facts for \
one profile section into a shorter set of bullet lines, preserving every session-id reference \
in brackets. Do not drop distinct facts; merge near-duplicates. Call save_section_consolidated \
with the condensed text.";

/// Merge candidate facts (and the previous profile.md, if any) into one prose
/// block per section. Non-fatal problems (a consolidate chunk falling back to
/// its raw bullets) are appended to `warnings`; only the final merge call can
/// fail the reduce.
pub fn run_reduce(
    scope: ProfileScope,
    previous_profile_md: Option<&str>,
    facts: &[ProfileFact],
    tool_call: &ToolCallFn,
    warnings: &mut Vec<String>,
) -> Result<ProfileSections, ProfileError> {
    let mut section_inputs = ProfileSections::default();
    let mut total_len = 0usize;
    let mut serialized_by_section: Vec<(ProfileSection, String)> = Vec::new();
    for section in scope.sections() {
        let serialized = serialize_section_facts(*section, facts);
        total_len += serialized.len();
        serialized_by_section.push((*section, serialized));
    }

    let needs_consolidation = total_len > CONSOLIDATION_THRESHOLD_CHARS;
    for (section, serialized) in serialized_by_section {
        if serialized.is_empty() {
            continue;
        }
        let text = if needs_consolidation {
            consolidate_section(section, &serialized, tool_call, warnings)
        } else {
            serialized
        };
        section_inputs.set(section, text);
    }

    let user_content = build_merge_user_content(scope, previous_profile_md, &section_inputs);
    let result = tool_call(
        MERGE_SYSTEM_PROMPT,
        &user_content,
        &save_user_profile_tool(scope),
        FINAL_MAX_TOKENS,
    )?;
    parse_sections(scope, &result)
}

/// Consolidate one section's bullet list chunk by chunk so each call's output
/// stays within `CONSOLIDATE_MAX_TOKENS`. A failed chunk keeps its raw
/// bullets (recorded in `warnings`) rather than aborting the whole reduce.
fn consolidate_section(
    section: ProfileSection,
    serialized: &str,
    tool_call: &ToolCallFn,
    warnings: &mut Vec<String>,
) -> String {
    chunk_lines(serialized, CONSOLIDATE_CHUNK_CHARS)
        .iter()
        .enumerate()
        .map(|(idx, chunk)| {
            consolidate_chunk(section, chunk, tool_call).unwrap_or_else(|error| {
                warnings.push(format!(
                    "consolidate {} chunk {}: {error} (kept raw facts)",
                    section.as_str(),
                    idx + 1
                ));
                chunk.clone()
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn consolidate_chunk(
    section: ProfileSection,
    chunk: &str,
    tool_call: &ToolCallFn,
) -> Result<String, ProfileError> {
    let user_content = format!(
        "Section: {}\n\nCandidate facts (newest first):\n{}",
        section.heading(),
        chunk
    );
    let result = tool_call(
        CONSOLIDATE_SYSTEM_PROMPT,
        &user_content,
        &save_section_consolidated_tool(),
        CONSOLIDATE_MAX_TOKENS,
    )?;
    result["consolidated"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ProfileError::BadToolCall("missing consolidated string".to_string()))
}

/// Split `text` on line boundaries into pieces of at most `max_chars` each
/// (a single line longer than `max_chars` becomes its own piece).
fn chunk_lines(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if !current.is_empty() && current.len() + line.len() + 1 > max_chars {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn serialize_section_facts(section: ProfileSection, facts: &[ProfileFact]) -> String {
    let mut matching: Vec<&ProfileFact> = facts.iter().filter(|f| f.section == section).collect();
    matching.sort_by(|a, b| b.date.cmp(&a.date));
    matching
        .iter()
        .map(|f| {
            format!(
                "- [{date}] {fact} [{evidence}]",
                date = f.date,
                fact = f.fact,
                evidence = f.evidence.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_merge_user_content(
    scope: ProfileScope,
    previous_profile_md: Option<&str>,
    sections: &ProfileSections,
) -> String {
    let mut parts = Vec::new();
    if let Some(previous) = previous_profile_md {
        if !previous.is_empty() {
            parts.push(format!("Previous profile.md:\n{previous}"));
        }
    }
    for section in scope.sections() {
        let text = sections.get(*section);
        if !text.is_empty() {
            parts.push(format!(
                "New candidate facts for {}:\n{}",
                section.heading(),
                text
            ));
        }
    }
    if parts.is_empty() {
        "No new facts and no previous profile. Produce an empty profile.".to_string()
    } else {
        parts.join("\n\n---\n\n")
    }
}

/// The `save_user_profile` tool schema for `scope`: one required string
/// property per section in the scope's skeleton.
fn save_user_profile_tool(scope: ProfileScope) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for section in scope.sections() {
        properties.insert(
            section.as_str().to_string(),
            serde_json::json!({"type": "string"}),
        );
        required.push(section.as_str());
    }
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "save_user_profile",
            "description": "Save the merged user profile, one field per fixed section.",
            "parameters": {
                "type": "object",
                "properties": Value::Object(properties),
                "required": required
            }
        }
    })
}

fn save_section_consolidated_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "save_section_consolidated",
            "description": "Save the condensed bullet list for one profile section.",
            "parameters": {
                "type": "object",
                "properties": {
                    "consolidated": { "type": "string" }
                },
                "required": ["consolidated"]
            }
        }
    })
}

fn parse_sections(scope: ProfileScope, value: &Value) -> Result<ProfileSections, ProfileError> {
    let mut sections = ProfileSections::default();
    for section in scope.sections() {
        let text = value[section.as_str()].as_str().ok_or_else(|| {
            ProfileError::BadToolCall(format!("missing section field: {}", section.as_str()))
        })?;
        sections.set(*section, text.to_string());
    }
    Ok(sections)
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;
