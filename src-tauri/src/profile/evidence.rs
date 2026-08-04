// HalluScribe - map-step evidence block: the per-session windowed body handed
// to the model, plus the batch-local `S<n>` session labels it cites instead of
// raw session UUIDs. Split out of distill.rs to keep that file under the
// repo's 350-LOC cap.
//
// Why labels: the map model used to be shown the full 36-char session UUID and
// asked to copy it back verbatim into `evidence`. Small models mangle or
// confabulate those strings (seen live 2026-08-04: a 374-session Personal run
// emitted `6a0ec0a-e492-9654-83eb-b820-b8af29eb857e` — six dash groups instead
// of five — which no prefix repair can recover), and a fact whose evidence all
// fails to resolve is dropped entirely. A batch is at most `BATCH_SIZE`
// sessions, so `S1`..`S18` is unambiguous, trivially copyable, and expands back
// to the real id here — every downstream consumer still sees real ids.

use super::types::ProfileFact;
use crate::archive::IndexEntry;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Chars of a session's markdown body kept from the START (goal/narrative).
pub(super) const HEAD_CHARS: usize = 1200;

/// Chars of a session's markdown body kept from the END. The sweep template
/// puts Key Decisions / Files Changed / Open Issues / Suggested Next Step at
/// the END of every summary (verified 2026-07-07: 83% of 1,442 archived .md
/// files exceed the old 1,500-char flat truncation, so that data was
/// systematically never seen by the distiller). See
/// docs/internal/PROFILE_QUALITY_PLAN.md Phase 2b.
pub(super) const TAIL_CHARS: usize = 1800;

/// Marker joining head and tail when a body is truncated.
pub(super) const SNIP_MARKER: &str = "\n[...snip...]\n";

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

/// The label shown to the model for the `idx`-th (0-based) session of a batch.
pub(super) fn label_for(idx: usize) -> String {
    format!("S{}", idx + 1)
}

/// Map every batch label (`S1`, `S2`, ...) to that session's real id.
pub(super) fn label_map<'a>(batch: &[&'a IndexEntry]) -> HashMap<String, &'a str> {
    batch
        .iter()
        .enumerate()
        .map(|(idx, entry)| (label_for(idx), entry.id.as_str()))
        .collect()
}

/// Rewrite label-shaped evidence entries into the real session ids they stand
/// for. Anything that is not a recognised label is left untouched, so the
/// caller's existing exact/unique-prefix validation still gets its chance at
/// a model that emitted a real id anyway — this can only resolve more
/// evidence than before, never less.
pub(super) fn expand_labels(
    facts: Vec<ProfileFact>,
    labels: &HashMap<String, &str>,
) -> Vec<ProfileFact> {
    facts
        .into_iter()
        .map(|mut fact| {
            fact.evidence = fact
                .evidence
                .into_iter()
                .map(|candidate| match canonical_label(&candidate) {
                    Some(label) => match labels.get(&label) {
                        Some(id) => (*id).to_string(),
                        // A label outside the batch range is a hallucination:
                        // leave it as-is so validation drops it with a warning.
                        None => candidate,
                    },
                    None => candidate,
                })
                .collect();
            fact
        })
        .collect()
}

/// Normalise a model-emitted evidence token to its canonical `S<n>` label, or
/// `None` when it is not label-shaped. Tolerates the wrappers small models add
/// in practice: surrounding brackets/quotes, a `Session`/`Session id:` prefix,
/// lower case, and the bare ordinal (`4` for `S4`). A real session id can never
/// reach `Some` here: ids contain dashes, so they fail the all-digits check.
fn canonical_label(candidate: &str) -> Option<String> {
    let lowered = candidate.to_ascii_lowercase();
    let trimmed = lowered.trim_matches(|c: char| {
        c.is_whitespace() || matches!(c, '[' | ']' | '"' | '\'' | '`' | ',')
    });
    let without_prefix = trimmed
        .strip_prefix("session id:")
        .or_else(|| trimmed.strip_prefix("session id"))
        .or_else(|| trimmed.strip_prefix("session"))
        .unwrap_or(trimmed)
        .trim();
    let digits = without_prefix
        .strip_prefix('s')
        .unwrap_or(without_prefix)
        .trim();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let ordinal: usize = digits.parse().ok()?;
    if ordinal == 0 {
        return None;
    }
    Some(format!("S{ordinal}"))
}

/// The full evidence block for a batch: one labelled entry per session.
pub(super) fn build_evidence_block(archive_dir: &Path, batch: &[&IndexEntry]) -> String {
    batch
        .iter()
        .enumerate()
        .map(|(idx, entry)| build_evidence_entry(archive_dir, entry, &label_for(idx)))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

/// One session's evidence entry. `label` is what the model sees as the session
/// id — the real id is deliberately never shown, so it cannot be miscopied.
pub(super) fn build_evidence_entry(archive_dir: &Path, entry: &IndexEntry, label: &str) -> String {
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
        "Session id: {label}\nTitle: {title}\nDate: {date}\nProject: {project}\nTool: {tool}\n\
         Type: {session_type}\nTags: {tags}\n\nBody:\n{truncated}",
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
pub(super) fn truncate_chars(text: &str, max_chars: usize) -> String {
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
pub(super) fn head_tail_window(text: &str) -> String {
    let total_chars = text.chars().count();
    if total_chars <= BODY_TRUNCATE_CHARS {
        return text.to_string();
    }
    let head = truncate_chars(text, HEAD_CHARS);
    let tail_start = total_chars.saturating_sub(TAIL_CHARS);
    let tail: String = text.chars().skip(tail_start).collect();
    format!("{head}{SNIP_MARKER}{tail}")
}

#[cfg(test)]
#[path = "evidence_tests.rs"]
mod tests;
