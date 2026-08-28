// HalluScribe - output-token extraction tests, per coding tool.

use super::{claude, codex, forge};

#[test]
fn claude_sums_output_tokens_across_assistant_turns() {
    // Shape taken from a live Claude Code transcript: usage rides the assistant
    // message and already contains the thinking tokens.
    let content = concat!(
        r#"{"message":{"role":"assistant","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":2,"cache_read_input_tokens":50650,"output_tokens":884,"output_tokens_details":{"thinking_tokens":572}}}}"#,
        "\n",
        r#"{"message":{"role":"user","content":[{"type":"text","text":"go on"}]}}"#,
        "\n",
        r#"{"message":{"role":"assistant","content":[{"type":"text","text":"ok"}],"usage":{"output_tokens":116}}}"#,
        "\n",
    );
    let count = claude::token_count(content);
    assert_eq!(count.output, 1000);
    assert!(!count.estimated);
}

#[test]
fn claude_falls_back_to_assistant_characters_without_usage() {
    let content = format!(
        r#"{{"message":{{"role":"assistant","content":[{{"type":"text","text":"{}"}}]}}}}"#,
        "x".repeat(400)
    );
    let count = claude::token_count(&content);
    assert_eq!(count.output, 100);
    assert!(count.estimated);
}

#[test]
fn claude_estimate_ignores_user_turns() {
    // The whole point of the per-role split: a pasted file is not model output.
    let content = format!(
        r#"{{"message":{{"role":"user","content":[{{"type":"text","text":"{}"}}]}}}}"#,
        "x".repeat(40_000)
    );
    assert_eq!(claude::token_count(&content).output, 0);
}

#[test]
fn claude_estimate_counts_tool_arguments_as_generated() {
    // A coding agent emits far more file content through tool calls than it
    // ever says in prose; leaving these out understated a real session ~28x.
    let content = format!(
        r#"{{"message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t1","name":"Write","input":{{"body":"{}"}}}}]}}}}"#,
        "x".repeat(392)
    );
    let count = claude::token_count(&content);
    assert!(count.estimated);
    assert!(count.output >= 98, "got {}", count.output);
}

#[test]
fn codex_prefers_the_cumulative_total() {
    let content = concat!(
        r#"{"type":"event_msg","payload":{"type":"token_count","info":{"model_context_window":272000,"last_token_usage":{"input_tokens":90,"output_tokens":40},"total_token_usage":{"input_tokens":900,"output_tokens":300}}}}"#,
        "\n",
        r#"{"type":"event_msg","payload":{"type":"token_count","info":{"model_context_window":272000,"last_token_usage":{"input_tokens":95,"output_tokens":55},"total_token_usage":{"input_tokens":1800,"output_tokens":700}}}}"#,
        "\n",
    );
    let count = codex::token_count(content);
    assert_eq!(count.output, 700);
    assert!(!count.estimated);
}

#[test]
fn codex_sums_per_turn_usage_when_no_total_is_reported() {
    let content = concat!(
        r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"output_tokens":40}}}}"#,
        "\n",
        r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"output_tokens":55}}}}"#,
        "\n",
    );
    let count = codex::token_count(content);
    assert_eq!(count.output, 95);
    assert!(!count.estimated);
}

#[test]
fn codex_ignores_a_null_info_block() {
    let content = r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}"#;
    assert_eq!(codex::token_count(content).output, 0);
    assert!(codex::token_count(content).estimated);
}

#[test]
fn forge_reads_the_usage_line() {
    let content = concat!(
        r#"{"type":"session_start","session_id":"a","title":"t","model":"qwen","timestamp_ms":1}"#,
        "\n",
        r#"{"role":"assistant","content":"hello","timestamp_ms":2,"model":"qwen"}"#,
        "\n",
        r#"{"type":"usage","input_tokens":12000,"output_tokens":3400,"model_request_count":4,"timestamp_ms":3,"model":"qwen"}"#,
        "\n",
    );
    let count = forge::token_count(content);
    assert_eq!(count.output, 3400);
    assert!(!count.estimated);
}

#[test]
fn forge_takes_the_largest_cumulative_total() {
    // The totals only climb in practice, but a reader should never report a
    // smaller number than it has already seen if a counter is ever reset.
    let content = concat!(
        r#"{"type":"usage","output_tokens":500}"#,
        "\n",
        r#"{"type":"usage","output_tokens":1800}"#,
        "\n",
        r#"{"type":"usage","output_tokens":40}"#,
        "\n",
    );
    assert_eq!(forge::token_count(content).output, 1800);
}

#[test]
fn forge_estimates_from_content_and_reasoning_when_older() {
    // Pre-0.13.17 sessions have no usage line at all. Reasoning counts: on a
    // thinking model it is the bulk of what was generated, and a tool-call turn
    // carries reasoning with no content.
    let content = format!(
        concat!(
            r#"{{"role":"user","content":"{}","timestamp_ms":1}}"#,
            "\n",
            r#"{{"role":"assistant","content":"{}","timestamp_ms":2}}"#,
            "\n",
            r#"{{"role":"assistant","content":null,"reasoning":"{}","tool_calls":[],"timestamp_ms":3}}"#,
            "\n"
        ),
        "u".repeat(8000),
        "a".repeat(200),
        "r".repeat(200)
    );
    let count = forge::token_count(&content);
    assert_eq!(count.output, 100);
    assert!(count.estimated);
}

#[test]
fn forge_estimate_counts_tool_arguments() {
    let content = format!(
        r#"{{"role":"assistant","content":null,"tool_calls":[{{"name":"edit_file","input":{{"body":"{}"}}}}],"timestamp_ms":1}}"#,
        "x".repeat(392)
    );
    let count = forge::token_count(&content);
    assert!(count.estimated);
    assert!(count.output >= 98, "got {}", count.output);
}

#[test]
fn forge_usage_lines_do_not_reach_the_transcript() {
    // They carry no `role`, which is the key the transcript parser filters on.
    let content = concat!(
        r#"{"role":"assistant","content":"hello","timestamp_ms":2}"#,
        "\n",
        r#"{"type":"usage","output_tokens":3400,"timestamp_ms":3}"#,
        "\n",
    );
    let rendered = forge::preprocess(content);
    assert!(rendered.contains("hello"));
    assert!(!rendered.contains("3400"));
    assert!(!rendered.contains("usage"));
}
