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

// The exact current text of the two existing prompts, captured byte-for-byte
// (at the 900-word target) so any edit to them fails loudly. These are the
// regression guard for Phase 8: the business variant must not touch them.
const CODING_PROMPT_900: &str = "You are a technical scribe. Analyse the AI coding session transcript the user provides and call save_session_summary with a detailed, developer-quality result. For the summary be specific and complete: cover Goal, What Was Done, Key Decisions, Files Changed, Open Issues, and Suggested Next Step. Aim for approximately 900 words, staying concise but complete. If the transcript contains a comparison table, benchmark result, rating, verdict, or decision matrix, copy it into verbatim_highlights exactly as written \u{2014} including its column headings and its conclusions. Do not paraphrase it and do not count it against the word budget above; verbatim_highlights is separate from the summary. For error_tags and topic_tags, only use short tags that are explicitly grounded in the transcript text itself, such as literal technologies, filenames, APIs, libraries, or error names that appear in the transcript. Do not infer broad languages, frameworks, or domains unless they are directly mentioned.";

const GENERAL_PROMPT_900: &str = "You are summarizing an AI conversation session. Analyse the normalized transcript and call save_session_summary with a clear result. For the summary be specific: cover the main topic, key questions, key answers or decisions, unresolved follow-ups, and any notable next steps. Aim for approximately 900 words, staying concise but complete. If the transcript contains a comparison table, rating, verdict, or decision matrix, copy it into verbatim_highlights exactly as written \u{2014} including its column headings and its conclusions. Do not paraphrase it and do not count it against the word budget above; verbatim_highlights is separate from the summary. Do not assume the conversation is about coding unless it clearly is. For error_tags and topic_tags, only use short tags that are explicitly grounded in the transcript text itself. Do not invent inferred tags that are not literally supported by the transcript.";

#[test]
fn coding_prompt_is_byte_identical() {
    assert_eq!(coding_system_prompt(900), CODING_PROMPT_900);
}

#[test]
fn general_prompt_is_byte_identical() {
    assert_eq!(general_system_prompt(900), GENERAL_PROMPT_900);
}

/// The word target `build_system_prompt` computes for the short transcript the
/// selection tests feed it (the minimum clamp), so the expected prompts match
/// exactly what the dispatcher produces.
const TEST_TRANSCRIPT: &str = "a transcript";

#[test]
fn the_business_prompt_is_selected_for_every_business_provider() {
    let target = summary_word_target(TEST_TRANSCRIPT);
    let expected = business_system_prompt(target);
    for provider in [
        ChatProvider::AppleMessages,
        ChatProvider::WhatsApp,
        ChatProvider::WhatsAppBusiness,
        ChatProvider::Viber,
    ] {
        assert_eq!(
            build_system_prompt(&provider, TEST_TRANSCRIPT),
            expected,
            "provider {provider:?}"
        );
    }
}

#[test]
fn the_coding_prompt_is_selected_for_every_coding_provider() {
    let target = summary_word_target(TEST_TRANSCRIPT);
    let expected = coding_system_prompt(target);
    for provider in [
        ChatProvider::ClaudeCode,
        ChatProvider::Codex,
        ChatProvider::Forge,
    ] {
        assert_eq!(
            build_system_prompt(&provider, TEST_TRANSCRIPT),
            expected,
            "provider {provider:?}"
        );
    }
}

#[test]
fn the_general_prompt_is_selected_for_every_general_provider() {
    let target = summary_word_target(TEST_TRANSCRIPT);
    let expected = general_system_prompt(target);
    for provider in [
        ChatProvider::ChatGPT,
        ChatProvider::ClaudeAI,
        ChatProvider::Gemini,
        ChatProvider::Grok,
        ChatProvider::HalluScribeAgentChat,
        ChatProvider::OllamaChat,
    ] {
        assert_eq!(
            build_system_prompt(&provider, TEST_TRANSCRIPT),
            expected,
            "provider {provider:?}"
        );
    }
}

#[test]
fn the_business_prompt_covers_the_required_fields_and_price_rule() {
    let prompt = business_system_prompt(900);
    for field in [
        "customer",
        "products or materials",
        "quantities",
        "dimensions",
        "prices",
        "dates",
        "agreed outcome",
        "open issues",
    ] {
        assert!(prompt.contains(field), "missing {field:?}");
    }
    assert!(prompt.contains("exactly as written"));
    assert!(prompt.contains("never infer"));
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
