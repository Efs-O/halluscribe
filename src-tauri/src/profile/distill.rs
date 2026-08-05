// HalluScribe - profile distiller map step: batch sessions into an evidence
// block and extract candidate facts via a `save_profile_facts` tool call.
// Evidence-block construction and the batch-local session labels the model
// cites live in `evidence.rs`.

use super::citations::resolve_id;
use super::evidence::{build_evidence_block, expand_labels, label_map, split_evidence_ids};
use super::scope::ProfileScope;
use super::types::{ProfileFact, ProfileSection};
use super::ProfileError;
use crate::archive::IndexEntry;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

/// Facts requested per map call; the tool schema also caps this server-side.
const MAX_FACTS_PER_BATCH: usize = 10;

// Generous headroom: session-id-heavy JSON tokenizes at ~2 chars/token, so a
// tight budget truncates the tool-call arguments mid-string (seen live at
// 2048 on the first full distill run). ctx_size dwarfs these values.
const MAP_MAX_TOKENS: u32 = 4096;

/// Operational clauses shared by both scopes' map prompts: the evidence-label
/// contract and the always-call-the-tool rule. The label contract replaced
/// "cite the session id" on 2026-08-04 — see `evidence.rs` for why the model
/// is never shown a real session id.
const MAP_PROMPT_RULES: &str = " Call save_profile_facts with up to 10 facts drawn ONLY from the \
evidence provided. Every fact must cite in `evidence` the short session label(s) it is grounded \
in, exactly as they appear after `Session id:` (e.g. \"S1\", \"S4\") - never any other identifier, \
and never one that is not in this batch. Do not invent facts not supported by the text. If the \
batch contains no qualifying facts, still call save_profile_facts with an empty facts array - \
never reply in plain text.";

/// Work scope: the original coding-oriented framing and section list.
const WORK_MAP_INTRO: &str = "You are distilling a batch of archived AI coding session \
summaries into durable facts about the user (their identity/context, active projects, \
conventions and preferences, recurring problems, communication style, and timeline \
highlights).";

/// Work scope only: `recurring_problems` is not part of the Personal skeleton,
/// so this clause must never reach the Personal prompt (it previously did,
/// and every fact the model filed there was then discarded by `parse_facts`).
const WORK_MAP_EXTRA: &str = " Any task the user repeatedly re-solves is a recurring problem \
regardless of domain - business workflows (e.g. catalog ingestion, network administration, \
tax/ERP reconciliation) belong in recurring_problems just as much as dev/AI issues.";

/// Personal scope (Phase 0 refocus): a life/character skeleton, so the intro
/// names only the four sections that skeleton actually accepts.
const PERSONAL_MAP_INTRO: &str = "You are distilling a batch of archived AI session summaries \
into durable facts about the user as a person (their identity/context, interests and life \
context, communication style, and timeline highlights).";

const PERSONAL_MAP_EXTRA: &str = " Extract personal, life, and character facts — interests, \
relationships, communication style, and life timeline — into the personal_context section; ignore \
purely technical or coding detail such as codebase internals, coding style rules, or bug/error \
specifics. When the evidence supports it, also extract household and relationship structure — who lives \
with or is cared for by the user, approximate ages, and roles — into personal_context, using \
explicit hedging language such as \"likely\" or \"appears to\" when this is inferred rather than \
stated outright.";

/// The map step's system prompt for `scope`. Each scope names only the
/// sections its own skeleton accepts, so the model is never steered toward a
/// section that `parse_facts` will then reject.
fn map_system_prompt(scope: ProfileScope) -> String {
    match scope {
        ProfileScope::Work => format!("{WORK_MAP_INTRO}{MAP_PROMPT_RULES}{WORK_MAP_EXTRA}"),
        ProfileScope::Personal => {
            format!("{PERSONAL_MAP_INTRO}{MAP_PROMPT_RULES}{PERSONAL_MAP_EXTRA}")
        }
    }
}

/// Generic inference entry point the map/reduce steps call through: given a
/// system prompt, user content, and tool schema, run one completion and
/// return the parsed tool-call arguments. Production wires this to a warm
/// `gemma::ToolSession`; tests inject a canned closure.
pub type ToolCallFn<'a> =
    dyn Fn(&str, &str, &Value, u32) -> Result<Value, ProfileError> + Send + Sync + 'a;

/// Map one batch of sessions into candidate facts. Returns the facts plus
/// non-fatal warnings (facts skipped for an unknown/out-of-scope section, or
/// dropped for unresolvable evidence).
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
    // Labels back to real session ids first, so everything downstream (and the
    // validation below) only ever sees real ids.
    let facts = expand_labels(facts, &label_map(batch));
    let known_ids: HashSet<&str> = batch.iter().map(|entry| entry.id.as_str()).collect();
    let (facts, evidence_warnings) = validate_evidence(facts, &known_ids);
    warnings.extend(evidence_warnings);
    Ok((facts, warnings))
}

/// Validate each fact's `evidence` ids against the batch's exact id set
/// (parse-time defence against hallucinated/truncated citations — see
/// docs/internal/PROFILE_QUALITY_PLAN.md Phase 1a). Runs after label
/// expansion, so a well-behaved model's evidence is already exact. An id that
/// exactly matches a batch id is kept; an id that is a unique prefix
/// (>= 8 chars) of exactly one batch id is repaired to that full id; anything
/// else is dropped. A fact whose evidence becomes empty is dropped entirely
/// (never fails the batch) with a warning.
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
        // One malformed item must not discard the whole batch. Truncated JSON
        // cannot reach here — it fails serde parsing back in the tool-call
        // extractor, with an explicit max_tokens hint — so an item arriving
        // without a string `section`/`fact` means the model shaped that ONE
        // entry wrongly while the rest of the batch is fine. Failing the batch
        // threw away every good fact alongside it; skip and warn instead, the
        // same way an out-of-scope section is already handled.
        let Some(section_str) = item["section"].as_str() else {
            warnings.push("skipped fact with missing or non-string section".to_string());
            continue;
        };
        let Some(fact) = item["fact"].as_str().map(str::to_string) else {
            warnings.push(format!(
                "skipped fact in section '{section_str}' with missing or non-string fact text"
            ));
            continue;
        };
        let section = match resolve_section(section_str) {
            Some(section) if scope.sections().contains(&section) => section,
            _ => {
                warnings.push(format!("skipped fact with unknown section '{section_str}'"));
                continue;
            }
        };
        let evidence = split_evidence_ids(
            item["evidence"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        );
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
