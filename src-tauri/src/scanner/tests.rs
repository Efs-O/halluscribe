// HalluScribe - scanner tests.

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::super::claude::{claude_fill_pct, parse_claude_usage_line};
    use super::super::codex::{codex_fill_pct, parse_codex_token_count};
    use super::super::continue_scan::{parse_continue_token_line, scan_continue_from_root};
    use super::super::forge::scan_forge_from_root;
    use super::super::shared::path_to_session_id;
    use super::super::{scan_chat_imports, scan_sessions};
    use crate::readers::ChatProvider;
    use crate::settings::HalluScribeSettings;
    use std::{fs, path::PathBuf};
    use tempfile::tempdir;

    #[test]
    fn claude_usage_valid_line() {
        let line = r#"{"message":{"model":"claude-sonnet-4-5","usage":{"input_tokens":50000,"cache_read_input_tokens":10000,"cache_creation_input_tokens":5000,"output_tokens":2000}}}"#;
        let (model, pct) = parse_claude_usage_line(line).unwrap();
        assert_eq!(model, "claude-sonnet-4-5");
        assert!(pct > 0.0 && pct <= 100.0);
    }

    #[test]
    fn claude_usage_empty_model_is_none() {
        let line = r#"{"message":{"model":"","usage":{"input_tokens":1000}}}"#;
        assert!(parse_claude_usage_line(line).is_none());
    }

    #[test]
    fn claude_usage_non_message_line_is_none() {
        let line = r#"{"type":"summary","content":"hello"}"#;
        assert!(parse_claude_usage_line(line).is_none());
    }

    #[test]
    fn claude_fill_pct_returns_last_line() {
        let low = r#"{"message":{"model":"claude-sonnet-4-5","usage":{"input_tokens":10000,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":500}}}"#;
        let high = r#"{"message":{"model":"claude-sonnet-4-5","usage":{"input_tokens":100000,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":5000}}}"#;
        let content = format!("{low}\n{high}");
        let (_, low_pct) = parse_claude_usage_line(low).unwrap();
        let (_, high_pct) = parse_claude_usage_line(high).unwrap();
        let result = claude_fill_pct(&content).unwrap();
        assert!((result - high_pct).abs() < 0.001);
        assert!(result > low_pct);
    }

    #[test]
    fn claude_reserve_is_counted() {
        let line = r#"{"message":{"model":"claude-sonnet-4-5","usage":{"input_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":0}}}"#;
        let (_, pct) = parse_claude_usage_line(line).unwrap();
        assert!(pct > 0.0);
    }

    #[test]
    fn codex_token_count_valid() {
        let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":80000},"model_context_window":128000}}}"#;
        let pct = parse_codex_token_count(line).unwrap();
        assert!((pct - 62.5).abs() < 0.01);
    }

    #[test]
    fn codex_wrong_event_type_is_none() {
        let line = r#"{"type":"session_meta","payload":{}}"#;
        assert!(parse_codex_token_count(line).is_none());
    }

    #[test]
    fn codex_null_info_is_none() {
        let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}"#;
        assert!(parse_codex_token_count(line).is_none());
    }

    #[test]
    fn codex_fill_pct_last_event_wins() {
        let first = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10000},"model_context_window":128000}}}"#;
        let last = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":90000},"model_context_window":128000}}}"#;
        let content = format!("{first}\n{last}");
        let result = codex_fill_pct(&content).unwrap();
        assert!((result - 90000.0 / 128000.0 * 100.0).abs() < 0.01);
    }

    #[test]
    fn continue_token_line_valid() {
        let line = r#"{"model":"claude-3-5-sonnet","promptTokens":75000,"timestamp":"2025-01-01T00:00:00Z"}"#;
        let (model, tokens) = parse_continue_token_line(line).unwrap();
        assert_eq!(model, "claude-3-5-sonnet");
        assert_eq!(tokens, 75000);
    }

    #[test]
    fn continue_token_line_zero_tokens_is_none() {
        let line = r#"{"model":"claude-3-5-sonnet","promptTokens":0}"#;
        assert!(parse_continue_token_line(line).is_none());
    }

    #[test]
    fn continue_scan_returns_one_target_per_recent_session() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let event_dir = root.join("dev_data").join("0.2.0");
        let sessions_dir = root.join("sessions");
        fs::create_dir_all(&event_dir).unwrap();
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::write(
            root.join("config.yaml"),
            "models:\n  - model: claude-3-5-sonnet\n    contextLength: 100000\n",
        )
        .unwrap();
        fs::write(
            event_dir.join("chatInteraction.jsonl"),
            concat!(
                "{\"sessionId\":\"first\",\"modelName\":\"claude-3-5-sonnet\",\"timestamp\":\"2030-01-01T00:00:00Z\"}\n",
                "{\"sessionId\":\"second\",\"modelName\":\"claude-3-5-sonnet\",\"timestamp\":\"2030-01-01T00:01:00Z\"}\n"
            ),
        )
        .unwrap();
        fs::write(
            event_dir.join("tokensGenerated.jsonl"),
            concat!(
                "{\"model\":\"claude-3-5-sonnet\",\"promptTokens\":45000,\"timestamp\":\"2030-01-01T00:00:30Z\"}\n",
                "{\"model\":\"claude-3-5-sonnet\",\"promptTokens\":85000,\"timestamp\":\"2030-01-01T00:01:30Z\"}\n"
            ),
        )
        .unwrap();
        fs::write(sessions_dir.join("first.json"), r#"{"history":[]}"#).unwrap();
        fs::write(sessions_dir.join("second.json"), r#"{"history":[]}"#).unwrap();

        let targets = scan_continue_from_root(root, u64::MAX, 40.0);
        assert_eq!(targets.len(), 2);
        assert!(targets
            .iter()
            .any(|target| target.path.ends_with("first.json")));
        assert!(targets
            .iter()
            .any(|target| target.path.ends_with("second.json")));
    }

    #[test]
    fn forge_scan_returns_agent_transcript_files() {
        let dir = tempdir().unwrap();
        let root = dir.path().join(".cursor").join("projects");
        let transcript_dir = root
            .join("workspace")
            .join("agent-transcripts")
            .join("session-1");
        fs::create_dir_all(&transcript_dir).unwrap();
        let transcript = transcript_dir.join("session-1.jsonl");
        fs::write(&transcript, "{\"role\":\"user\"}\n").unwrap();

        let targets = scan_forge_from_root(&root, u64::MAX);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, transcript);
        assert!(matches!(
            targets[0].kind,
            super::super::ScanTargetKind::Coding(super::super::ToolSource::Forge)
        ));
    }

    #[test]
    fn path_to_session_id_deterministic() {
        let path = PathBuf::from("/some/test/path.jsonl");
        assert_eq!(path_to_session_id(&path), path_to_session_id(&path));
    }

    #[test]
    fn path_to_session_id_different_paths_differ() {
        let a = PathBuf::from("/path/a.jsonl");
        let b = PathBuf::from("/path/b.jsonl");
        assert_ne!(path_to_session_id(&a), path_to_session_id(&b));
    }

    #[test]
    fn scan_chat_imports_trims_configured_paths() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("conversations.json"), "[]").unwrap();

        let settings = HalluScribeSettings {
            chatgpt_import_path: format!("{} ", dir.path().display()),
            ..HalluScribeSettings::default()
        };

        let targets = scan_chat_imports(&settings, u64::MAX);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, dir.path().join("conversations.json"));
        assert!(matches!(
            targets[0].kind,
            super::super::ScanTargetKind::Import(ChatProvider::ChatGPT)
        ));
    }

    #[test]
    fn scan_sessions_includes_recorded_gemma_chats() {
        let dir = tempdir().unwrap();
        let archive_dir = dir.path().join(".halluscribe");
        let recorded_dir = archive_dir
            .join("recorded_sessions")
            .join("HalluScribe")
            .join("gemma4")
            .join("2026-04-23");
        fs::create_dir_all(&recorded_dir).unwrap();
        fs::write(recorded_dir.join("10-00-00-000-abc-gemma4-chat.json"), "{}").unwrap();

        let targets = scan_sessions(&archive_dir, &HalluScribeSettings::default(), u64::MAX, 0.0);
        assert!(targets.iter().any(|target| {
            target.path.ends_with("10-00-00-000-abc-gemma4-chat.json")
                && matches!(
                    target.kind,
                    super::super::ScanTargetKind::Import(ChatProvider::HalluScribeGemmaChat)
                )
        }));
    }
}
