// HalluScribe - unit tests for the citation integrity module (citations.rs):
// id resolution (citations::resolve_id) and the write-time bracket-group
// sanitizer (citations::sanitize_citations).

use super::*;

fn ids<'a>(list: &[&'a str]) -> HashSet<&'a str> {
    list.iter().copied().collect()
}

#[test]
fn resolve_id_exact_match() {
    let known = ids(&["abc12345", "def67890"]);
    assert_eq!(resolve_id("abc12345", &known), Some("abc12345"));
}

#[test]
fn resolve_id_unique_prefix_repair() {
    let known = ids(&["d087b575-1234-4abc-9def-3bddaffa6ae1", "other-id-99999999"]);
    assert_eq!(
        resolve_id("d087b575-1234", &known),
        Some("d087b575-1234-4abc-9def-3bddaffa6ae1")
    );
}

#[test]
fn resolve_id_ambiguous_prefix_returns_none() {
    let known = ids(&["abcd1234-aaaa", "abcd1234-bbbb"]);
    assert_eq!(resolve_id("abcd1234", &known), None);
}

#[test]
fn resolve_id_too_short_for_prefix_repair_returns_none() {
    let known = ids(&["abcd1234-aaaa"]);
    assert_eq!(resolve_id("abcd", &known), None);
}

#[test]
fn sanitize_citations_leaves_valid_group_untouched() {
    let known = ids(&["abc12345", "def67890"]);
    let (text, warnings) = sanitize_citations("Uses Rust [abc12345, def67890].", &known);
    assert_eq!(text, "Uses Rust [abc12345, def67890].");
    assert!(warnings.is_empty());
}

#[test]
fn sanitize_citations_repairs_corrupted_id_by_prefix() {
    let known = ids(&["d087b575-1234-4abc-9def-3bddaffa6ae1"]);
    // 11-char truncated final group, as seen in the probe's corrupted output.
    let (text, warnings) = sanitize_citations("Ships CUDA fix [d087b575-12].", &known);
    assert_eq!(
        text,
        "Ships CUDA fix [d087b575-1234-4abc-9def-3bddaffa6ae1]."
    );
    assert!(warnings.is_empty());
}

#[test]
fn sanitize_citations_caps_group_at_three() {
    // Ids are hex-hash-derived in production (e.g. "c47cc0fa"); use the same
    // shape here so the scanner recognizes the group as citation-like.
    let full_ids: Vec<String> = (0..30).map(|i| format!("aaaa{i:04x}-deadbeef")).collect();
    let known: HashSet<&str> = full_ids.iter().map(String::as_str).collect();
    let group = full_ids.join(", ");
    let text = format!("Big claim [{group}].");
    let (sanitized, warnings) = sanitize_citations(&text, &known);
    assert_eq!(
        sanitized,
        format!(
            "Big claim [{}, {}, {}].",
            full_ids[0], full_ids[1], full_ids[2]
        )
    );
    assert!(warnings.is_empty());
}

#[test]
fn sanitize_citations_leaves_markdown_links_untouched() {
    let known = ids(&["abc12345"]);
    let text = "See [the docs](https://example.com/abc12345-page) for details.";
    let (sanitized, warnings) = sanitize_citations(text, &known);
    assert_eq!(sanitized, text);
    assert!(warnings.is_empty());
}

#[test]
fn sanitize_citations_leaves_plain_number_brackets_untouched() {
    let known = ids(&["abc12345"]);
    let (text, warnings) = sanitize_citations("Answer is [42].", &known);
    assert_eq!(text, "Answer is [42].");
    assert!(warnings.is_empty());
}

#[test]
fn sanitize_citations_removes_group_with_all_invalid_ids_cleanly() {
    let known = ids(&["abc12345"]);
    // Hex/dash-shaped so the scanner treats it as a citation group, but
    // absent from the known id set so it must be dropped entirely.
    let (text, warnings) = sanitize_citations("Claim [deadbeef01, cafebabe02] here.", &known);
    assert_eq!(text, "Claim here.");
    assert!(!warnings.is_empty());
}

#[test]
fn sanitize_citations_repairs_realistic_uuid_vs_invalid() {
    let real_id = "d087b575-1234-4abc-9def-3bddaffa6ae9";
    let known = ids(&[real_id]);
    // The 11-char final group is a valid unique prefix and gets repaired;
    // a second, unrelated invalid id is dropped from the same group.
    let text = "Decision [d087b575-1234, ce29999999].";
    let (sanitized, warnings) = sanitize_citations(text, &known);
    assert_eq!(sanitized, format!("Decision [{real_id}]."));
    assert!(warnings.iter().any(|w| w.contains("ce29999999")));
}

#[test]
fn sanitize_citations_no_brackets_is_noop() {
    let known = ids(&["abc12345"]);
    let (text, warnings) = sanitize_citations("Plain prose with no brackets at all.", &known);
    assert_eq!(text, "Plain prose with no brackets at all.");
    assert!(warnings.is_empty());
}
