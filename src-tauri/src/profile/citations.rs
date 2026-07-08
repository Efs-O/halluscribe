// HalluScribe - citation integrity: shared id resolution (exact match or
// unique-prefix repair) plus a write-time sanitizer that scrubs corrupted
// bracket citation groups out of merged profile prose. See
// docs/internal/PROFILE_QUALITY_PLAN.md Phase 1.

use std::collections::HashSet;

/// Minimum candidate length eligible for prefix-repair. Guards against a
/// short, common substring spuriously "uniquely" matching one id in a small
/// batch.
const MIN_PREFIX_LEN: usize = 8;

/// Cap on ids kept per citation group when sanitizing merged prose.
const MAX_IDS_PER_GROUP: usize = 3;

/// Resolve a model-emitted id candidate against a known id set: exact match
/// wins; otherwise, if `candidate` (>= `MIN_PREFIX_LEN` chars) is a prefix of
/// exactly one known id, repair to that id; otherwise `None`.
pub(super) fn resolve_id<'a>(candidate: &str, known_ids: &HashSet<&'a str>) -> Option<&'a str> {
    if let Some(&exact) = known_ids.get(candidate) {
        return Some(exact);
    }
    if candidate.len() < MIN_PREFIX_LEN {
        return None;
    }
    let mut matches = known_ids.iter().filter(|id| id.starts_with(candidate));
    let first = *matches.next()?;
    if matches.next().is_some() {
        return None; // ambiguous prefix - matches more than one id
    }
    Some(first)
}

/// Sanitize bracket citation groups in `text` against the full known id set.
/// Returns the sanitized text plus any warnings (dropped ids / removed
/// groups) for the caller's refresh log.
pub(super) fn sanitize_citations(text: &str, known_ids: &HashSet<&str>) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut warnings = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            if let Some(close) = find_close(&chars, i) {
                let inner: String = chars[i + 1..close].iter().collect();
                // A markdown link has `(` immediately after `]` - never treat
                // as a citation group.
                let is_link = chars.get(close + 1) == Some(&'(');
                if !is_link && is_id_shaped_group(&inner) {
                    let (kept, group_warnings) = sanitize_group(&inner, known_ids);
                    warnings.extend(group_warnings);
                    if !kept.is_empty() {
                        out.push('[');
                        out.push_str(&kept.join(", "));
                        out.push(']');
                    } else {
                        warnings.push(format!("removed citation group '[{inner}]' - no valid ids"));
                        // Collapse a doubled space left behind by the removal
                        // (e.g. "text [a, b] more" -> "text  more" -> "text more").
                        if out.ends_with(' ') && chars.get(close + 1) == Some(&' ') {
                            out.pop();
                        }
                    }
                    i = close + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    (out, warnings)
}

/// Index of the matching `]` for an opening `[` at `start`, or `None` if
/// unterminated. No nesting support needed - citation groups never nest.
fn find_close(chars: &[char], start: usize) -> Option<usize> {
    chars[start + 1..]
        .iter()
        .position(|&c| c == ']')
        .map(|offset| start + 1 + offset)
}

/// A group counts as citation-shaped only when it is non-empty and every
/// comma-separated token is id-shaped (see `is_id_shaped_token`). Otherwise
/// it is left untouched (prose brackets, `[42]`, etc).
fn is_id_shaped_group(inner: &str) -> bool {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.split(',').all(|tok| is_id_shaped_token(tok.trim()))
}

/// A token is id-shaped when it is at least `MIN_PREFIX_LEN` chars, contains
/// only hex digits and dashes, and contains at least one hex LETTER (a-f).
/// That last requirement is deliberately stricter than "dash or hex letter":
/// a plain decimal number like "12345678" is excluded (no letter, no dash),
/// but so is a date bracket like "2026-06-01" (dash-shaped digits-only, as
/// found in merge fallback bullet lines `- [date] fact [evidence]`) - both
/// would otherwise false-positive as citation groups. Real session ids are
/// hash-derived hex, so requiring a hex letter is safe in practice.
fn is_id_shaped_token(tok: &str) -> bool {
    if tok.len() < MIN_PREFIX_LEN {
        return false;
    }
    if !tok.chars().all(|c| c == '-' || c.is_ascii_hexdigit()) {
        return false;
    }
    tok.chars()
        .any(|c| c.is_ascii_hexdigit() && !c.is_ascii_digit())
}

/// Resolve every token in a citation group against `known_ids`, drop
/// invalids (with a warning), and cap the survivors at `MAX_IDS_PER_GROUP`.
fn sanitize_group(inner: &str, known_ids: &HashSet<&str>) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::new();
    let mut warnings = Vec::new();
    for tok in inner.split(',').map(str::trim) {
        match resolve_id(tok, known_ids) {
            Some(canonical) => {
                if kept.len() < MAX_IDS_PER_GROUP {
                    kept.push(canonical.to_string());
                }
            }
            None => warnings.push(format!("dropped invalid citation id '{tok}'")),
        }
    }
    (kept, warnings)
}

#[cfg(test)]
#[path = "citations_tests.rs"]
mod tests;
