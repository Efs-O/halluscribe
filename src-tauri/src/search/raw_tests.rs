// HalluScribe - unit tests for raw transcript pure matching and excerpts.

use super::{json_escaped, scan_text};

#[test]
fn finds_tool_call_args_and_user_message() {
    let text = concat!(
        r#"{"type":"tool_use","input":{"command":"netsh winsock reset"}}"#,
        "\n",
        r#"{"type":"user","message":{"content":"Please run netsh winsock reset now"}}"#,
    );

    let outcome = scan_text(text, "netsh winsock reset", 20);
    assert_eq!(outcome.total_hits, 2);
    assert_eq!(outcome.excerpts.len(), 2);
    assert_eq!(outcome.excerpts[0].line_no, 1);
    assert_eq!(outcome.excerpts[1].line_no, 2);
}

#[test]
fn finds_json_escaped_path() {
    let text = r#"{"type":"tool_use","input":{"command":"cargo run","cwd":"C:\\work\\app"}}"#;
    let outcome = scan_text(text, r"C:\work\app", 20);

    assert_eq!(json_escaped(r"C:\work\app"), r"C:\\work\\app");
    assert_eq!(outcome.total_hits, 1);
    assert_eq!(outcome.excerpts.len(), 1);
}

#[test]
fn finds_json_escaped_double_quote() {
    let text = r#"{"type":"user","message":{"content":"please say \"hello\" clearly"}}"#;
    let outcome = scan_text(text, r#"say "hello""#, 20);

    assert_eq!(json_escaped(r#"say "hello""#), r#"say \"hello\""#);
    assert_eq!(outcome.total_hits, 1);
}

#[test]
fn absent_needle_has_empty_untruncated_outcome() {
    let text = r#"{"type":"tool_use","input":{"command":"cargo test"}}"#;
    let outcome = scan_text(text, "netsh", 20);

    assert_eq!(outcome.total_hits, 0);
    assert!(outcome.excerpts.is_empty());
    assert!(!outcome.excerpts_truncated);
}

#[test]
fn empty_and_whitespace_needles_never_match_everything() {
    let text = r#"{"type":"user","message":{"content":"some content"}}"#;

    for needle in ["", "   ", "\t\r\n"] {
        let outcome = scan_text(text, needle, 20);
        assert_eq!(outcome.total_hits, 0);
        assert!(outcome.excerpts.is_empty());
        assert!(!outcome.excerpts_truncated);
    }
}

#[test]
fn giant_single_line_is_trimmed_around_hit() {
    let padding = "x".repeat(600_000);
    let text = format!(r#"{{"type":"tool_result","content":"{padding}UNIQUE_NEEDLE{padding}"}}"#);
    assert!(text.len() > 1_000_000);

    let outcome = scan_text(&text, "unique_needle", 20);
    assert_eq!(outcome.total_hits, 1);
    assert_eq!(outcome.excerpts[0].excerpt.chars().count(), 200);
    assert!(outcome.excerpts[0].excerpt.contains("unique_needle"));
    let before = outcome.excerpts[0]
        .excerpt
        .split("unique_needle")
        .next()
        .unwrap()
        .chars()
        .count();
    assert!((90..=100).contains(&before));
}

#[test]
fn greek_context_and_greek_needle_are_char_safe() {
    let padding = "αλφάβητο ".repeat(30);
    let text =
        format!(r#"{{"type":"user","message":{{"content":"{padding}ΔΟΚΙΜΗ στόχος{padding}"}}}}"#);
    let outcome = scan_text(&text, "δοκιμη", 20);

    assert_eq!(outcome.total_hits, 1);
    assert_eq!(outcome.excerpts[0].excerpt.chars().count(), 200);
    assert!(outcome.excerpts[0].excerpt.contains("δοκιμη"));
}

#[test]
fn matching_is_case_insensitive() {
    let text = r#"{"type":"tool_use","input":{"command":"netsh winsock reset"}}"#;
    assert_eq!(scan_text(text, "NETSH", 20).total_hits, 1);
}

#[test]
fn excerpt_cap_does_not_stop_truthful_counting() {
    let text = (0..30)
        .map(|index| format!(r#"{{"type":"tool_use","input":{{"command":"netsh run {index}"}}}}"#))
        .collect::<Vec<_>>()
        .join("\n");

    let outcome = scan_text(&text, "netsh", 20);
    assert_eq!(outcome.total_hits, 30);
    assert_eq!(outcome.excerpts.len(), 20);
    assert!(outcome.excerpts_truncated);
}

#[test]
fn multiple_hits_on_one_line_keep_one_excerpt() {
    let text = r#"{"type":"user","message":{"content":"netsh then netsh then netsh"}}"#;
    let outcome = scan_text(text, "netsh", 20);

    assert_eq!(outcome.total_hits, 3);
    assert_eq!(outcome.excerpts.len(), 1);
    assert!(!outcome.excerpts_truncated);
}

#[test]
fn line_numbers_are_one_based() {
    let text = concat!(
        r#"{"type":"system","message":"ready"}"#,
        "\n",
        r#"{"type":"assistant","message":"checking"}"#,
        "\n",
        r#"{"type":"tool_use","input":{"command":"netsh winsock reset"}}"#,
    );
    let outcome = scan_text(text, "netsh", 20);

    assert_eq!(outcome.excerpts[0].line_no, 3);
}

#[test]
fn overlapping_dual_needle_hits_at_same_position_count_once() {
    let text = r#"{"type":"user","message":{"content":"aaaa"}}"#;
    let outcome = scan_text(text, "aa", 20);

    assert_eq!(outcome.total_hits, 3);
    assert_eq!(outcome.excerpts.len(), 1);
}
