// HalluScribe - profile distiller map step: batch sessions into an evidence
// block and extract candidate facts via a `save_profile_facts` tool call.

use super::types::{ProfileFact, ProfileSection};
use super::ProfileError;
use crate::archive::IndexEntry;
use serde_json::Value;
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

/// Generic inference entry point the map/reduce steps call through: given a
/// system prompt, user content, and tool schema, run one completion and
/// return the parsed tool-call arguments. Production wires this to a warm
/// `gemma::ToolSession`; tests inject a canned closure.
pub type ToolCallFn<'a> =
    dyn Fn(&str, &str, &Value, u32) -> Result<Value, ProfileError> + Send + Sync + 'a;

pub(super) fn distill_batch(
    archive_dir: &Path,
    batch: &[&IndexEntry],
    tool_call: &ToolCallFn,
) -> Result<Vec<ProfileFact>, ProfileError> {
    let user_content = build_evidence_block(archive_dir, batch);
    let result = tool_call(
        MAP_SYSTEM_PROMPT,
        &user_content,
        &save_profile_facts_tool(),
        MAP_MAX_TOKENS,
    )?;
    parse_facts(&result)
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

fn save_profile_facts_tool() -> Value {
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
                                    "enum": [
                                        "identity", "projects", "conventions",
                                        "recurring_problems", "communication_style", "timeline"
                                    ]
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

fn parse_facts(value: &Value) -> Result<Vec<ProfileFact>, ProfileError> {
    let items = value["facts"]
        .as_array()
        .ok_or_else(|| ProfileError::BadToolCall("missing facts array".to_string()))?;
    let mut facts = Vec::new();
    for item in items.iter().take(MAX_FACTS_PER_BATCH) {
        let section_str = item["section"]
            .as_str()
            .ok_or_else(|| ProfileError::BadToolCall("fact missing string section".to_string()))?;
        let section = ProfileSection::from_key(section_str)
            .ok_or_else(|| ProfileError::BadToolCall(format!("unknown section: {section_str}")))?;
        let fact = item["fact"]
            .as_str()
            .ok_or_else(|| ProfileError::BadToolCall("fact missing string fact".to_string()))?
            .to_string();
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
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn entry(id: &str, archive_path: &str) -> IndexEntry {
        IndexEntry {
            id: id.to_string(),
            project: "proj".to_string(),
            date: "2026-06-01".to_string(),
            title: format!("Session {id}"),
            tool: "Claude Code".to_string(),
            fill_pct: 90.0,
            session_timestamp: "2026-06-01T00:00:00+00:00".to_string(),
            updated_at: String::new(),
            session_type: "building".to_string(),
            error_tags: vec!["ECONNRESET".to_string()],
            topic_tags: vec!["rust".to_string()],
            archive_path: archive_path.to_string(),
            source_jsonl: String::new(),
            source_size_bytes: 0,
            provider: "claude_code".to_string(),
            fill_estimated: false,
            transcript_hash: String::new(),
        }
    }

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("halluscribe_profile_distill_{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn ok_facts_response(section: &str) -> Value {
        serde_json::json!({
            "facts": [
                {
                    "section": section,
                    "fact": "Uses Rust and Tauri.",
                    "evidence": ["s1"],
                    "date": "2026-06-01"
                }
            ]
        })
    }

    #[test]
    fn distill_batch_parses_facts_from_fake_tool_call() {
        let dir = tmp_dir("basic");
        fs::write(dir.join("s1.md"), "session body").unwrap();
        let e = entry("s1", "s1.md");
        let batch = vec![&e];
        let tool_call: &ToolCallFn =
            &|_sys, _user, _tool, _max| Ok(ok_facts_response("conventions"));
        let facts = distill_batch(&dir, &batch, tool_call).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].section, ProfileSection::Conventions);
        assert_eq!(facts[0].evidence, vec!["s1".to_string()]);
    }

    #[test]
    fn distill_batch_surfaces_malformed_tool_call_as_error() {
        let dir = tmp_dir("bad_json");
        fs::write(dir.join("s1.md"), "body one").unwrap();
        let e = entry("s1", "s1.md");
        let batch = vec![&e];
        let tool_call: &ToolCallFn =
            &|_sys, _user, _tool, _max| Ok(serde_json::json!({"not_facts": true}));
        let error = distill_batch(&dir, &batch, tool_call).unwrap_err();
        assert!(error.to_string().contains("missing facts array"));
    }

    #[test]
    fn parse_facts_rejects_unknown_section() {
        let value = ok_facts_response("not_a_real_section");
        let error = parse_facts(&value).unwrap_err();
        assert!(matches!(error, ProfileError::BadToolCall(_)));
    }

    #[test]
    fn truncate_chars_is_multibyte_safe() {
        let text = "café".repeat(1000); // multi-byte 'é' repeated well past the limit
        let truncated = truncate_chars(&text, 10);
        assert_eq!(truncated.chars().count(), 10);
        // Must still be valid UTF-8 (guaranteed by String) and start correctly.
        assert!(truncated.starts_with("café"));
    }

    #[test]
    fn build_evidence_entry_truncates_long_bodies() {
        let dir = tmp_dir("truncate");
        let long_body = "x".repeat(5000);
        fs::write(dir.join("s1.md"), &long_body).unwrap();
        let e = entry("s1", "s1.md");
        let block = build_evidence_entry(&dir, &e);
        // Body section should contain at most BODY_TRUNCATE_CHARS x's, not 5000.
        let body_start = block.find("Body:\n").unwrap() + "Body:\n".len();
        assert_eq!(block[body_start..].chars().count(), BODY_TRUNCATE_CHARS);
    }

    #[test]
    fn build_evidence_entry_includes_metadata() {
        let dir = tmp_dir("metadata");
        fs::write(dir.join("s1.md"), "hello").unwrap();
        let e = entry("s1", "s1.md");
        let block = build_evidence_entry(&dir, &e);
        assert!(block.contains("Session id: s1"));
        assert!(block.contains("Project: proj"));
        assert!(block.contains("ECONNRESET"));
        assert!(block.contains("rust"));
    }
}
