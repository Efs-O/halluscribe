// HalluScribe - profile distiller map step: batch sessions into an evidence
// block and extract candidate facts via a `save_profile_facts` tool call.

use super::citations::resolve_id;
use super::scope::ProfileScope;
use super::types::{ProfileFact, ProfileSection};
use super::ProfileError;
use crate::archive::IndexEntry;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Chars of a session's markdown body kept from the START (goal/narrative).
const HEAD_CHARS: usize = 1200;

/// Chars of a session's markdown body kept from the END. The sweep template
/// puts Key Decisions / Files Changed / Open Issues / Suggested Next Step at
/// the END of every summary (verified 2026-07-07: 83% of 1,442 archived .md
/// files exceed the old 1,500-char flat truncation, so that data was
/// systematically never seen by the distiller). See
/// docs/internal/PROFILE_QUALITY_PLAN.md Phase 2b.
const TAIL_CHARS: usize = 1800;

/// Marker joining head and tail when a body is truncated.
const SNIP_MARKER: &str = "\n[...snip...]\n";

/// Budget math (re-verified 2026-07-07 against the real constants):
/// - `select::BATCH_SIZE` was 30 sessions/map-call under the old flat
///   1,500-char window: 30 x 1,500 = 45,000 evidence chars/call.
/// - HEAD_CHARS + TAIL_CHARS raises the per-session cap to ~3,000 chars, plus
///   ~150-250 chars of metadata (id/title/date/project/tool/type/tags) per
///   entry: 30 x (3,000 + ~200) ~= 96,000 chars/call - well past the ~60K
///   char target (~15-30K tokens at a worst-case 2-4 chars/token for
///   Greek-heavy content).
/// - The deployed ctx_size (`~/.halluscribe/settings.json`) is 102,400
///   tokens, so 96K chars (~24-48K tokens) would still technically fit
///   alongside MAP_MAX_TOKENS=4096 output - but the settings UI allows
///   ctx_size as low as 8,192 tokens
///   (`src/components/settings/SettingsForm.svelte`), and the plan's budget
///   target is meant to hold at the low end too, not just the one deployment
///   observed live. `select::BATCH_SIZE` was therefore reduced from 30 to 18:
///   18 x (3,000 + ~200) ~= 57,600 chars/call, under the ~60K target with
///   margin for metadata variance (long titles/tags).
const BODY_TRUNCATE_CHARS: usize = HEAD_CHARS + TAIL_CHARS;

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
not invent facts not supported by the text. Any task the user repeatedly re-solves is a \
recurring problem regardless of domain - business workflows (e.g. catalog ingestion, \
network administration, tax/ERP reconciliation) belong in recurring_problems just as much \
as dev/AI issues. If the batch contains no qualifying facts, still call save_profile_facts \
with an empty facts array - never reply in plain text.";

/// Appended to `MAP_SYSTEM_PROMPT` only for the Personal scope (Phase 0
/// refocus): Personal is now a life/character skeleton, so the map step
/// should bias toward personal facts into `personal_context` and ignore
/// coding detail that has no home in that skeleton.
const PERSONAL_MAP_EXTRA: &str = " Also extract personal, life, and character facts — interests, \
relationships, communication style, and life timeline — into the personal_context section; ignore \
purely technical or coding detail such as project internals, conventions, or bug/error specifics. \
When the evidence supports it, also extract household and relationship structure — who lives \
with or is cared for by the user, approximate ages, and roles — into personal_context, using \
explicit hedging language such as \"likely\" or \"appears to\" when this is inferred rather than \
stated outright.";

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
    let (facts, mut warnings) = parse_facts(scope, &result)?;
    let known_ids: HashSet<&str> = batch.iter().map(|entry| entry.id.as_str()).collect();
    let (facts, evidence_warnings) = validate_evidence(facts, &known_ids);
    warnings.extend(evidence_warnings);
    Ok((facts, warnings))
}

/// Validate each fact's `evidence` ids against the batch's exact id set
/// (parse-time defence against hallucinated/truncated citations — see
/// docs/internal/PROFILE_QUALITY_PLAN.md Phase 1a). An id that exactly
/// matches a batch id is kept; an id that is a unique prefix (>= 8 chars) of
/// exactly one batch id is repaired to that full id; anything else is
/// dropped. A fact whose evidence becomes empty is dropped entirely (never
/// fails the batch) with a warning.
fn validate_evidence(
    facts: Vec<ProfileFact>,
    known_ids: &HashSet<&str>,
) -> (Vec<ProfileFact>, Vec<String>) {
    let mut kept = Vec::new();
    let mut warnings = Vec::new();
    for mut fact in facts {
        let original = fact.evidence.clone();
        let mut resolved = Vec::new();
        for id in &original {
            match resolve_id(id, known_ids) {
                Some(canonical) => resolved.push(canonical.to_string()),
                None => warnings.push(format!(
                    "dropped invalid evidence id '{id}' from fact '{}'",
                    fact.fact
                )),
            }
        }
        if resolved.is_empty() {
            warnings.push(format!(
                "dropped fact '{}' with no valid evidence ids",
                fact.fact
            ));
            continue;
        }
        fact.evidence = resolved;
        kept.push(fact);
    }
    (kept, warnings)
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
    let truncated = head_tail_window(&body);
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

/// Head+tail evidence window (Phase 2b): bodies within the combined budget
/// pass through verbatim; longer bodies keep the first `HEAD_CHARS` (the
/// goal/narrative opening) and the last `TAIL_CHARS` (Key Decisions / Files
/// Changed / Open Issues / Suggested Next Step, which the sweep template
/// always places at the end), joined by `SNIP_MARKER`. Char-based throughout
/// (`.chars()`, never byte slicing) so multi-byte content (Greek, etc.) is
/// never cut mid-codepoint - see the v0.3.4 byte-slice panic this repo
/// already hit once.
fn head_tail_window(text: &str) -> String {
    let total_chars = text.chars().count();
    if total_chars <= BODY_TRUNCATE_CHARS {
        return text.to_string();
    }
    let head = truncate_chars(text, HEAD_CHARS);
    let tail_start = total_chars.saturating_sub(TAIL_CHARS);
    let tail: String = text.chars().skip(tail_start).collect();
    format!("{head}{SNIP_MARKER}{tail}")
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
