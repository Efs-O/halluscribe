// HalluScribe - tests for chat_prompt.rs's system prompt builder.
use super::*;

fn base_context() -> ChatPromptContext {
    ChatPromptContext {
        web_search_available: false,
        has_images: false,
        search_mode: SearchModePrompt::Archive,
        scope_size: None,
        profile: None,
        profile_scope: ProfileScope::Work,
    }
}

#[test]
fn archive_only_prompt_allows_general_conversation_without_web_evidence() {
    let prompt = build_chat_system_prompt(&base_context());
    assert!(prompt
        .contains("answer general questions and have normal conversation using general knowledge"));
    assert!(prompt.contains("natural, helpful conversation about general topics"));
    assert!(!prompt.contains("two evidence sources"));
}

#[test]
fn archive_prompt_warns_against_false_completeness() {
    let prompt = build_chat_system_prompt(&base_context());
    assert!(prompt.contains("total_matches"));
    assert!(prompt.contains("never state or imply you searched every session"));
}

#[test]
fn archive_prompt_routes_deterministic_analysis_questions_to_their_tools() {
    let prompt = build_chat_system_prompt(&base_context());

    assert!(prompt.contains("count_sessions_by_provider"));
    assert!(prompt.contains("list_archive_facets"));
    assert!(prompt.contains("compare_session_periods"));
    assert!(prompt.contains("archive_health"));
}

#[test]
fn semantic_prompt_preserves_precision_rule() {
    let prompt = build_chat_system_prompt(&ChatPromptContext {
        web_search_available: true,
        has_images: false,
        search_mode: SearchModePrompt::Semantic,
        scope_size: Some(5),
        profile: None,
        profile_scope: ProfileScope::Work,
    });
    assert!(prompt.contains("do not rewrite that as a full fix"));
    assert!(prompt.contains("semantically relevant sessions"));
    assert!(prompt.contains("exactly 5 session(s)"));
    assert!(prompt.contains("Archive Evidence:"));
    assert!(prompt.contains("Current Web Evidence:"));
}

#[test]
fn profile_present_includes_markers_and_content() {
    let prompt = build_chat_system_prompt(&ChatPromptContext {
        profile: Some("# User Profile\n\nWorks mostly in Rust and Svelte.".to_string()),
        ..base_context()
    });
    assert!(prompt.contains("--- BEGIN USER PROFILE ---"));
    assert!(prompt.contains("--- END USER PROFILE ---"));
    assert!(prompt.contains("Works mostly in Rust and Svelte."));
}

#[test]
fn profile_absent_states_unavailable_without_markers() {
    let prompt = build_chat_system_prompt(&base_context());
    assert!(
        prompt.contains("No distilled user profile is available for the selected scope (work).")
    );
    assert!(!prompt.contains("--- BEGIN USER PROFILE ---"));
}

#[test]
fn work_profile_is_labelled_as_work() {
    let prompt = build_chat_system_prompt(&ChatPromptContext {
        profile: Some("Works mostly in Rust.".to_string()),
        profile_scope: ProfileScope::Work,
        ..base_context()
    });
    assert!(prompt.contains("the user's work profile"));
}

#[test]
fn personal_profile_is_labelled_as_personal() {
    let prompt = build_chat_system_prompt(&ChatPromptContext {
        profile: Some("Enjoys hiking.".to_string()),
        profile_scope: ProfileScope::Personal,
        ..base_context()
    });
    assert!(prompt.contains("the user's personal profile"));
}

#[test]
fn absent_personal_profile_names_the_scope() {
    let prompt = build_chat_system_prompt(&ChatPromptContext {
        profile_scope: ProfileScope::Personal,
        ..base_context()
    });
    assert!(prompt
        .contains("No distilled user profile is available for the selected scope (personal)."));
    assert!(!prompt.contains("--- BEGIN USER PROFILE ---"));
}

#[test]
fn oversized_profile_is_truncated_with_marker() {
    let oversized = "a".repeat(PROFILE_PROMPT_MAX_CHARS + 500);
    let prompt = build_chat_system_prompt(&ChatPromptContext {
        profile: Some(oversized),
        ..base_context()
    });
    assert!(prompt.contains("[profile truncated for context budget]"));
    // The embedded content must not exceed the cap plus the marker text.
    assert!(!prompt.contains(&"a".repeat(PROFILE_PROMPT_MAX_CHARS + 1)));
}

#[test]
fn semantic_mode_with_profile_states_scope_not_widened() {
    let prompt = build_chat_system_prompt(&ChatPromptContext {
        search_mode: SearchModePrompt::Semantic,
        profile: Some("Prefers concise commit messages.".to_string()),
        ..base_context()
    });
    assert!(prompt.contains("does not widen the allowed tool-search scope"));
}
