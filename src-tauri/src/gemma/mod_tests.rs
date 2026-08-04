// HalluScribe - unit tests for the gemma module: transcript grounding of tags
// and verbatim highlights, and the summary word-target clamp.

use super::*;

#[test]
fn retain_grounded_tags_drops_inferred_single_word_tags() {
    let tags = vec!["Python".to_string(), "JWT".to_string(), "auth".to_string()];
    let kept = retain_grounded_tags(&tags, "[User]\nFix JWT auth bug in middleware");
    assert_eq!(kept, vec!["JWT".to_string(), "auth".to_string()]);
}

#[test]
fn retain_grounded_tags_keeps_grounded_multiword_tags() {
    let tags = vec!["invalid request".to_string(), "media type".to_string()];
    let kept = retain_grounded_tags(
        &tags,
        "[Assistant]\nError: invalid request because media type is missing",
    );
    assert_eq!(
        kept,
        vec!["invalid request".to_string(), "media type".to_string()]
    );
}

#[test]
fn summary_word_target_uses_minimum_for_small_transcripts() {
    assert_eq!(summary_word_target("a short transcript"), MIN_SUMMARY_WORDS);
}

#[test]
fn summary_word_target_scales_for_medium_transcripts() {
    let transcript = "a".repeat(20_000);
    assert_eq!(summary_word_target(&transcript), 900);
}

#[test]
fn summary_word_target_caps_large_transcripts() {
    let transcript = "a".repeat(100_000);
    assert_eq!(summary_word_target(&transcript), MAX_SUMMARY_WORDS);
}

const BOARD_TRANSCRIPT: &str = "[Assistant]\nHere is the comparison:\n\n\
        | task | who wins | confidence |\n| ocr | gemma | high |\n\n\
        On the counting task the local model beats me outright.";

#[test]
fn retain_grounded_highlights_keeps_verbatim_table() {
    let highlights = vec!["| task | who wins | confidence |\n| ocr | gemma | high |".to_string()];
    let kept = retain_grounded_highlights(&highlights, BOARD_TRANSCRIPT);
    assert_eq!(kept, highlights);
}

#[test]
fn retain_grounded_highlights_drops_invented_content() {
    // The verdict sounds plausible and is the kind of thing a small model
    // will confidently synthesise, but these words are not in the
    // transcript. A fabricated "verbatim" quote is worse than none.
    let highlights = vec!["| task | who wins |\n| ocr | claude wins every round |".to_string()];
    assert!(retain_grounded_highlights(&highlights, BOARD_TRANSCRIPT).is_empty());
}

#[test]
fn retain_grounded_highlights_ignores_table_punctuation_differences() {
    // Normalisation collapses pipes/newlines/padding, so a re-flowed copy of
    // a real table still counts as grounded.
    let highlights = vec!["task / who wins / confidence".to_string()];
    let kept = retain_grounded_highlights(&highlights, BOARD_TRANSCRIPT);
    assert_eq!(kept.len(), 1);
}

#[test]
fn retain_grounded_highlights_dedupes_and_drops_empty() {
    let highlights = vec![
        "the local model beats me outright".to_string(),
        "   ".to_string(),
        "The local model beats me outright".to_string(),
    ];
    let kept = retain_grounded_highlights(&highlights, BOARD_TRANSCRIPT);
    assert_eq!(kept, vec!["the local model beats me outright".to_string()]);
}

#[test]
fn retain_grounded_highlights_enforces_caps() {
    let long = "x".repeat(MAX_HIGHLIGHT_CHARS + 1);
    let transcript = format!("[Assistant]\n{long}\nrepeated verdict line");
    assert!(retain_grounded_highlights(&[long], &transcript).is_empty());

    // More candidates than MAX_HIGHLIGHTS: the list is truncated, not grown.
    let many: Vec<String> = (0..MAX_HIGHLIGHTS + 5)
        .map(|i| format!("verdict {i}"))
        .collect();
    let all = many.join(" ");
    assert_eq!(
        retain_grounded_highlights(&many, &all).len(),
        MAX_HIGHLIGHTS
    );
}

#[test]
fn coding_prompt_asks_for_verbatim_highlights_outside_the_budget() {
    let prompt = coding_system_prompt(900);
    assert!(prompt.contains("verbatim_highlights"));
    assert!(prompt.contains("exactly as written"));
}

#[test]
fn general_prompt_asks_for_verbatim_highlights_outside_the_budget() {
    let prompt = general_system_prompt(900);
    assert!(prompt.contains("verbatim_highlights"));
    assert!(prompt.contains("exactly as written"));
}
