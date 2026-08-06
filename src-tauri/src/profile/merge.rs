// HalluScribe - profile distiller reduce step: merge candidate facts (and the
// previous profile.md, if any) into prose, one bounded `save_profile_section`
// tool call PER section — never one giant all-sections call, whose output
// exceeded any fixed token budget on large archives. Contradiction rule:
// recency wins, but the merge prompt asks the model to note the change rather
// than silently drop the old value.

use super::distill::ToolCallFn;
use super::parse_md::parse_profile_sections;
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

/// One section's merged prose fits this budget. History: the original
/// all-sections call truncated mid-JSON at 8192 tokens on a full-archive run,
/// so the merge became per-section at 4096 — which a fact-dense section
/// ("conventions", Personal scope, 1377 sessions) then also overflowed at
/// ~8.9K chars. 8192 for a single section's output leaves ample headroom.
const SECTION_MAX_TOKENS: u32 = 8192;
const CONSOLIDATE_MAX_TOKENS: u32 = 4096;

const SECTION_MERGE_SYSTEM_PROMPT: &str = "You maintain one section of a durable profile of \
the user distilled from their AI session archive. You are given the section name, the \
previous content of that section (if any), and new candidate facts, newest first. Call \
save_profile_section with the merged prose for THIS section only. Rules: recency wins on \
contradictions, but note the change (e.g. \"previously used X, switched to Y around \
2026-03\"). Every non-obvious claim must carry session-id references like [abc-123] drawn \
ONLY from the evidence ids given to you - never invent an id. Keep the section concise (a \
CV, not a diary).";

/// Appended to the user content for the Projects section only.
const PROJECTS_SECTION_NOTE: &str = "Note: projects unseen for more than 12 months belong in \
the Timeline section, not here — omit them.";

const CONSOLIDATE_SYSTEM_PROMPT: &str = "You are compressing a long list of candidate facts for \
one profile section into a shorter set of bullet lines, preserving every session-id reference \
in brackets. Do not drop distinct facts; merge near-duplicates. Call save_section_consolidated \
with the condensed text.";

/// Merge candidate facts (and the previous profile.md, if any) into one prose
/// block per section, with one bounded model call per section that has new
/// facts. Sections without new facts keep their previous text verbatim, with
/// no model call. A failed section call falls back to previous text + raw
/// bullets (recorded in `warnings`), so the reduce as a whole is effectively
/// infallible; the `Result` return type is kept for signature compatibility.
///
/// `on_progress(current, total)` is called once before each model call. On a
/// full archive the reduce is ~45 sequential calls taking the better part of an
/// hour, so reporting it as a single tick made the UI look frozen for the whole
/// stage; the call plan is therefore computed up front to give a real total.
pub fn run_reduce(
    scope: ProfileScope,
    previous_profile_md: Option<&str>,
    facts: &[ProfileFact],
    tool_call: &ToolCallFn,
    warnings: &mut Vec<String>,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<ProfileSections, ProfileError> {
    let previous = parse_profile_sections(previous_profile_md.unwrap_or(""));

    let mut total_len = 0usize;
    let mut serialized_by_section: Vec<(ProfileSection, String)> = Vec::new();
    for section in scope.sections() {
        let serialized = serialize_section_facts(*section, facts);
        total_len += serialized.len();
        serialized_by_section.push((*section, serialized));
    }
    let needs_consolidation = total_len > CONSOLIDATION_THRESHOLD_CHARS;

    let mut merged = ProfileSections::default();
    // Plan every model call before making any: one merge call per section that
    // has facts, plus one consolidate call per chunk when the archive is large
    // enough to need the consolidation pass. An empty `chunks` means the
    // section skips consolidation and merges its raw bullets directly.
    let mut plan: Vec<(ProfileSection, String, Vec<String>)> = Vec::new();
    for (section, serialized) in serialized_by_section {
        if serialized.is_empty() {
            // No new candidate facts: keep the previous section text verbatim
            // (empty string if none) without calling the model.
            merged.set(section, previous.get(section).to_string());
            continue;
        }
        let chunks = if needs_consolidation {
            chunk_lines(&serialized, CONSOLIDATE_CHUNK_CHARS)
        } else {
            Vec::new()
        };
        plan.push((section, serialized, chunks));
    }
    let total_steps: usize = plan.iter().map(|(_, _, chunks)| chunks.len() + 1).sum();

    let mut step = 0usize;
    for (section, serialized, chunks) in plan {
        let prev = previous.get(section);
        let chunk_count = chunks.len();
        let bullets = if chunks.is_empty() {
            serialized
        } else {
            let base = step;
            consolidate_section(section, &chunks, tool_call, warnings, &mut |done| {
                on_progress(base + done, total_steps);
            })
        };
        step += chunk_count + 1;
        on_progress(step, total_steps);
        let text = match merge_section(section, prev, &bullets, tool_call) {
            Ok(text) => text,
            Err(error) => {
                warnings.push(format!(
                    "merge section {}: {error} (kept raw facts)",
                    section.as_str()
                ));
                if prev.is_empty() {
                    bullets
                } else {
                    format!("{prev}\n{bullets}")
                }
            }
        };
        merged.set(section, text);
    }
    Ok(merged)
}

/// One bounded merge call for one section: previous content + new bullets in,
/// merged prose out via `save_profile_section`.
fn merge_section(
    section: ProfileSection,
    previous: &str,
    bullets: &str,
    tool_call: &ToolCallFn,
) -> Result<String, ProfileError> {
    let mut user_content = format!(
        "Section: {heading}\n\nPrevious content:\n{prev}\n\nNew candidate facts (newest first):\n{bullets}",
        heading = section.heading(),
        prev = if previous.is_empty() { "(none)" } else { previous },
    );
    if section == ProfileSection::Projects {
        user_content.push_str("\n\n");
        user_content.push_str(PROJECTS_SECTION_NOTE);
    }
    let result = tool_call(
        SECTION_MERGE_SYSTEM_PROMPT,
        &user_content,
        &save_profile_section_tool(),
        SECTION_MAX_TOKENS,
    )?;
    result["content"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ProfileError::BadToolCall("missing content string".to_string()))
}

/// Consolidate one section's pre-chunked bullet list so each call's output
/// stays within `CONSOLIDATE_MAX_TOKENS`. A failed chunk keeps its raw
/// bullets (recorded in `warnings`) rather than aborting the whole reduce.
/// `on_chunk` receives the 1-based index of the chunk about to be consolidated.
fn consolidate_section(
    section: ProfileSection,
    chunks: &[String],
    tool_call: &ToolCallFn,
    warnings: &mut Vec<String>,
    on_chunk: &mut dyn FnMut(usize),
) -> String {
    chunks
        .iter()
        .enumerate()
        .map(|(idx, chunk)| {
            on_chunk(idx + 1);
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

/// The `save_profile_section` tool schema: one required string property with
/// the merged prose for the current section.
fn save_profile_section_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "save_profile_section",
            "description": "Save the merged prose for the one profile section being maintained.",
            "parameters": {
                "type": "object",
                "properties": {
                    "content": { "type": "string" }
                },
                "required": ["content"]
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

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "merge_progress_tests.rs"]
mod progress_tests;
