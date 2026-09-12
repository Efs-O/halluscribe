// HalluScribe - scanner tests.

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::super::claude::{claude_fill_pct, parse_claude_usage_line};
    use super::super::codex::{codex_fill_pct, parse_codex_token_count};
    use super::super::forge::scan_forge_from_root;
    use super::super::shared::{collect_jsonl, mtime_secs, path_to_session_id, SCAN_DEPTH_LIMIT};
    use super::super::{scan_chat_imports, scan_sessions};
    use crate::readers::ChatProvider;
    use crate::settings::HalluScribeSettings;
    use std::{
        fs,
        path::PathBuf,
        time::{Duration, UNIX_EPOCH},
    };
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

        let targets = scan_forge_from_root(&root, u64::MAX, 0.0);
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
    fn collect_jsonl_stops_at_the_depth_boundary() {
        let dir = tempdir().unwrap();
        let mut at_limit = dir.path().to_path_buf();
        for index in 0..SCAN_DEPTH_LIMIT {
            at_limit = at_limit.join(format!("level-{index}"));
        }
        fs::create_dir_all(&at_limit).unwrap();
        fs::write(at_limit.join("included.jsonl"), "{}").unwrap();
        let beyond_limit = at_limit.join("one-more");
        fs::create_dir_all(&beyond_limit).unwrap();
        fs::write(beyond_limit.join("excluded.jsonl"), "{}").unwrap();

        let found = collect_jsonl(dir.path());
        assert!(found
            .iter()
            .any(|(_, path)| path.ends_with("included.jsonl")));
        assert!(!found
            .iter()
            .any(|(_, path)| path.ends_with("excluded.jsonl")));
    }

    #[test]
    fn mtime_secs_preserves_pre_epoch_ordering() {
        assert_eq!(mtime_secs(UNIX_EPOCH - Duration::from_secs(42)), -42);
        assert_eq!(mtime_secs(UNIX_EPOCH + Duration::from_secs(42)), 42);
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
    fn scan_chat_imports_finds_grok_export_nested_as_downloaded() {
        // A Grok export unpacks as ttl/30d/export_data/<user_id>/prod-grok-backend.json.
        // <user_id> is a per-account UUID, so it cannot be a literal candidate -
        // the folder must still be found when dropped in exactly as downloaded.
        let dir = tempdir().unwrap();
        let nested = dir
            .path()
            .join("ttl")
            .join("30d")
            .join("export_data")
            .join("5134baa3-7e58-487e-a6bf-68bcd8d71450");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("prod-grok-backend.json"), "{}").unwrap();
        // Sibling exports in the same folder must not be mistaken for it.
        fs::write(nested.join("prod-mc-billing.json"), "{}").unwrap();

        let settings = HalluScribeSettings {
            grok_import_path: dir.path().display().to_string(),
            ..HalluScribeSettings::default()
        };

        let targets = scan_chat_imports(&settings, u64::MAX);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, nested.join("prod-grok-backend.json"));
        assert!(matches!(
            targets[0].kind,
            super::super::ScanTargetKind::Import(ChatProvider::Grok)
        ));
    }

    #[test]
    fn grok_export_is_also_found_unpacked_flat() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("prod-grok-backend.json"), "{}").unwrap();

        let settings = HalluScribeSettings {
            grok_import_path: dir.path().display().to_string(),
            ..HalluScribeSettings::default()
        };

        let targets = scan_chat_imports(&settings, u64::MAX);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, dir.path().join("prod-grok-backend.json"));
    }

    /// The sweep and the raw backfill must locate the SAME files: a layout
    /// honoured by one authority and missed by the other is the exact bug
    /// IMPORT_PATHS_PLAN was written to prevent.
    #[test]
    fn both_import_authorities_agree_on_the_nested_grok_layout() {
        let dir = tempdir().unwrap();
        let nested = dir
            .path()
            .join("ttl")
            .join("30d")
            .join("export_data")
            .join("some-uuid");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("prod-grok-backend.json"), "{}").unwrap();

        let settings = HalluScribeSettings {
            grok_import_path: dir.path().display().to_string(),
            ..HalluScribeSettings::default()
        };

        let swept: Vec<PathBuf> = scan_chat_imports(&settings, u64::MAX)
            .into_iter()
            .map(|target| target.path)
            .collect();
        let backfilled = super::super::chat_import_sources(&settings, "grok");
        assert_eq!(swept, backfilled);
        assert_eq!(backfilled.len(), 1);
    }

    #[test]
    fn scan_chat_imports_discovers_split_chatgpt_export() {
        // Large ChatGPT exports arrive chunked as conversations-000.json, -001.json, …
        // (no plain conversations.json). Every chunk must be picked up.
        let dir = tempdir().unwrap();
        for suffix in ["000", "001", "002"] {
            fs::write(
                dir.path().join(format!("conversations-{suffix}.json")),
                "[]",
            )
            .unwrap();
        }
        // An unrelated json file must NOT be treated as an export chunk.
        fs::write(dir.path().join("settings.json"), "{}").unwrap();

        let settings = HalluScribeSettings {
            chatgpt_import_path: dir.path().display().to_string(),
            ..HalluScribeSettings::default()
        };

        let targets = scan_chat_imports(&settings, u64::MAX);
        assert_eq!(targets.len(), 3);
        for suffix in ["000", "001", "002"] {
            assert!(targets.iter().any(|target| target
                .path
                .ends_with(format!("conversations-{suffix}.json"))));
        }
        assert!(targets.iter().all(|target| matches!(
            target.kind,
            super::super::ScanTargetKind::Import(ChatProvider::ChatGPT)
        )));
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

        let targets = scan_sessions(
            &archive_dir,
            &HalluScribeSettings::default(),
            u64::MAX,
            0.0,
            false,
        );
        assert!(targets.iter().any(|target| {
            target.path.ends_with("10-00-00-000-abc-gemma4-chat.json")
                && matches!(
                    target.kind,
                    super::super::ScanTargetKind::Import(ChatProvider::HalluScribeAgentChat)
                )
        }));
    }

    #[test]
    fn import_only_skips_local_tools_keeps_recorded_chats() {
        // Local-tool source: a Forge override root with one minimal .jsonl fixture.
        let local = tempdir().unwrap();
        let forge_root = local.path().join("forge_sessions");
        fs::create_dir_all(&forge_root).unwrap();
        fs::write(forge_root.join("guest-1.jsonl"), "{\"role\":\"user\"}\n").unwrap();

        // Archive dir with one recorded in-app gemma chat.
        let archive = tempdir().unwrap();
        let archive_dir = archive.path().join(".halluscribe");
        let recorded_dir = archive_dir
            .join("recorded_sessions")
            .join("HalluScribe")
            .join("gemma4")
            .join("2026-04-23");
        fs::create_dir_all(&recorded_dir).unwrap();
        fs::write(recorded_dir.join("10-00-00-000-abc-gemma4-chat.json"), "{}").unwrap();

        let settings = HalluScribeSettings {
            forge_sessions_path: forge_root.display().to_string(),
            ..HalluScribeSettings::default()
        };

        let has_forge = |targets: &[super::super::ScanTarget]| {
            targets.iter().any(|target| {
                matches!(
                    target.kind,
                    super::super::ScanTargetKind::Coding(super::super::ToolSource::Forge)
                )
            })
        };
        let has_recorded = |targets: &[super::super::ScanTarget]| {
            targets.iter().any(|target| {
                target.path.ends_with("10-00-00-000-abc-gemma4-chat.json")
                    && matches!(
                        target.kind,
                        super::super::ScanTargetKind::Import(ChatProvider::HalluScribeAgentChat)
                    )
            })
        };

        // Host workspace (import_only = false): the local Forge fixture is present.
        let host = scan_sessions(&archive_dir, &settings, u64::MAX, 0.0, false);
        assert!(has_forge(&host));
        assert!(has_recorded(&host));

        // Guest workspace (import_only = true): the local Forge fixture is gated
        // out, while the workspace's own recorded chat is still ingested.
        let guest = scan_sessions(&archive_dir, &settings, u64::MAX, 0.0, true);
        assert!(!has_forge(&guest));
        assert!(has_recorded(&guest));
    }
}
