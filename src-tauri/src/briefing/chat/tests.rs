// HalluScribe - tests for chat tool errors and web-answer compliance.

use super::*;

#[test]
fn tool_error_message_reads_only_string_errors() {
    assert_eq!(
        tool_error_message(r#"{"error":"request failed"}"#),
        Some("request failed".to_string())
    );
    assert_eq!(tool_error_message(r#"{"result":"ok"}"#), None);
    assert_eq!(tool_error_message("not json"), None);
}

#[test]
fn sources_detection_accepts_headings_decorations_and_urls() {
    assert!(contains_sources_block("## Sources:\n- https://example.com"));
    assert!(contains_sources_block(
        "**Sources:**\n- https://example.com"
    ));
    assert!(contains_source_urls("Evidence: http://example.com"));
    assert!(!contains_sources_block("No citations were supplied."));
}

#[test]
fn web_answer_issue_only_requires_sources_after_web_use() {
    assert!(web_answer_issue(false, None).is_none());
    assert!(matches!(
        web_answer_issue(true, None),
        Some(WebAnswerIssue::Empty)
    ));
    assert!(matches!(
        web_answer_issue(true, Some("Answer without citations")),
        Some(WebAnswerIssue::MissingSources)
    ));
    assert!(web_answer_issue(true, Some("Sources:\nhttps://example.com")).is_none());
}

#[test]
fn compliance_retry_prompts_require_sources() {
    assert!(retry_prompt_for_issue(WebAnswerIssue::Empty).contains("Sources:"));
    assert!(retry_prompt_for_issue(WebAnswerIssue::MissingSources).contains("exact URLs"));
}
