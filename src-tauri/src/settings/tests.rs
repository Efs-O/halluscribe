// HalluScribe - settings persistence and runtime conversion tests.
#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::super::{load_settings, save_settings, BackendKind, HalluScribeSettings};
    use crate::gemma::InferenceBackend;
    use std::{fs, path::PathBuf};
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn defaults_are_sane() {
        let settings = HalluScribeSettings::default();
        assert!(!settings.always_on_top);
        assert_eq!(settings.backend, BackendKind::LlamaCpp);
        assert_eq!(settings.ollama_model, "gemma4:26b");
        assert!(settings.ollama_api_key.is_empty());
        assert!(settings.tavily_api_key.is_empty());
        assert_eq!(settings.summary_min_fill_pct, 50.0);
        // Auto-sweep is opt-in: off by default so no unattended sweep runs
        // against a workspace (esp. a new guest) without the user enabling it.
        assert!(!settings.scheduled_processing_enabled);
        assert_eq!(settings.schedule_time, "02:00");
        assert_eq!(settings.lookback_hours, 24);
        assert_eq!(settings.gpu_layers, -1);
        assert_eq!(settings.ctx_size, 0);
        assert_eq!(settings.max_tokens, 0);
    }

    #[test]
    fn load_returns_defaults_when_file_absent() {
        let dir = tmp();
        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(settings.schedule_time, "02:00");
    }

    #[test]
    fn load_partial_json_merges_defaults() {
        let dir = tmp();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"schedule_time": "03:15", "ollama_model": "gemma4:12b"}"#,
        )
        .unwrap();
        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(settings.schedule_time, "03:15");
        assert_eq!(settings.ollama_model, "gemma4:12b");
        assert_eq!(settings.summary_min_fill_pct, 50.0);
        assert_eq!(settings.ctx_size, 0);
        assert_eq!(settings.max_tokens, 0);
    }

    #[test]
    fn load_malformed_json_returns_an_error() {
        let dir = tmp();
        fs::write(dir.path().join("settings.json"), "not json at all").unwrap();
        let error = load_settings(dir.path()).unwrap_err();
        assert!(error.to_string().contains("settings JSON error"));
    }

    #[test]
    fn save_refuses_to_replace_malformed_settings() {
        let dir = tmp();
        let path = dir.path().join("settings.json");
        fs::write(&path, "not json at all").unwrap();

        assert!(save_settings(dir.path(), &HalluScribeSettings::default()).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "not json at all");
    }

    #[test]
    fn save_and_reload_round_trips() {
        let dir = tmp();
        let settings = HalluScribeSettings {
            schedule_time: "05:30".to_string(),
            always_on_top: true,
            ollama_model: "gemma4:12b".to_string(),
            ollama_api_key: "ollama-key".to_string(),
            tavily_api_key: "tavily-key".to_string(),
            ..Default::default()
        };
        save_settings(dir.path(), &settings).unwrap();
        let reloaded = load_settings(dir.path()).unwrap();
        assert_eq!(reloaded.schedule_time, "05:30");
        assert!(reloaded.always_on_top);
        assert_eq!(reloaded.ollama_model, "gemma4:12b");
        assert_eq!(reloaded.ollama_api_key, "ollama-key");
        assert_eq!(reloaded.tavily_api_key, "tavily-key");
    }

    #[test]
    fn load_legacy_schedule_hour_migrates_to_top_of_hour() {
        let dir = tmp();
        fs::write(dir.path().join("settings.json"), r#"{"schedule_time": 4}"#).unwrap();
        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(settings.schedule_time, "04:00");
    }

    #[test]
    fn save_creates_dir_if_absent() {
        let dir = tmp();
        let nested = dir.path().join("deep").join("nested");
        let settings = HalluScribeSettings::default();
        save_settings(&nested, &settings).unwrap();
        assert!(nested.join("settings.json").exists());
    }

    #[test]
    fn backend_kind_serializes_lowercase() {
        let a = serde_json::to_string(&BackendKind::LlamaCpp).unwrap();
        let b = serde_json::to_string(&BackendKind::Ollama).unwrap();
        assert_eq!(a, "\"llamacpp\"");
        assert_eq!(b, "\"ollama\"");
    }

    #[test]
    fn backend_kind_deserializes_from_lowercase() {
        let a: BackendKind = serde_json::from_str("\"llamacpp\"").unwrap();
        let b: BackendKind = serde_json::from_str("\"ollama\"").unwrap();
        assert_eq!(a, BackendKind::LlamaCpp);
        assert_eq!(b, BackendKind::Ollama);
    }

    #[test]
    fn llamacpp_returns_none_when_bin_empty() {
        let settings = HalluScribeSettings {
            backend: BackendKind::LlamaCpp,
            llama_server_bin: String::new(),
            gemma_model_path: "/models/gemma.gguf".to_string(),
            ..Default::default()
        };
        assert!(settings.to_inference_backend().is_none());
    }

    #[test]
    fn llamacpp_returns_none_when_model_empty() {
        let settings = HalluScribeSettings {
            backend: BackendKind::LlamaCpp,
            llama_server_bin: "/usr/bin/llama-server".to_string(),
            gemma_model_path: String::new(),
            ..Default::default()
        };
        assert!(settings.to_inference_backend().is_none());
    }

    #[test]
    fn llamacpp_returns_some_when_both_set() {
        let settings = HalluScribeSettings {
            backend: BackendKind::LlamaCpp,
            llama_server_bin: "/usr/bin/llama-server".to_string(),
            gemma_model_path: "/models/gemma.gguf".to_string(),
            gpu_layers: 35,
            gpu_devices: "CUDA1".to_string(),
            gpu_split_mode: "layer".to_string(),
            gpu_tensor_split: "0.8,0.2".to_string(),
            gpu_main_index: 1,
            llama_server_port: 8080,
            ..Default::default()
        };
        let backend = settings.to_inference_backend().unwrap();
        let InferenceBackend::LlamaCpp { port, gpu, .. } = backend else {
            panic!("expected a llama.cpp backend");
        };
        assert_eq!(port, 8080);
        // Every GPU placement setting has to reach the backend, or the sweep
        // would silently spawn on whatever card enumerated first.
        assert_eq!(gpu.layers, 35);
        assert_eq!(gpu.devices, "CUDA1");
        assert_eq!(gpu.split_mode, "layer");
        assert_eq!(gpu.tensor_split, "0.8,0.2");
        assert_eq!(gpu.main_gpu, 1);
    }

    #[test]
    fn gpu_placement_defaults_to_fully_unset() {
        // A fresh install must behave exactly as bare llama.cpp does.
        let gpu = HalluScribeSettings::default().gpu_config();
        assert_eq!(gpu.layers, -1);
        assert!(gpu.devices.is_empty());
        assert!(gpu.split_mode.is_empty());
        assert!(gpu.tensor_split.is_empty());
        assert_eq!(gpu.main_gpu, -1);
        assert!(gpu.validate().is_ok());
    }

    #[test]
    fn ollama_always_returns_some() {
        let settings = HalluScribeSettings {
            backend: BackendKind::Ollama,
            ..Default::default()
        };
        let backend = settings.to_inference_backend().unwrap();
        assert!(matches!(backend, InferenceBackend::Ollama { .. }));
    }

    #[test]
    fn sweep_config_none_when_llamacpp_bin_empty() {
        let settings = HalluScribeSettings {
            backend: BackendKind::LlamaCpp,
            llama_server_bin: String::new(),
            ..Default::default()
        };
        assert!(settings
            .to_sweep_config(PathBuf::from("/tmp/hs"), false)
            .is_none());
    }

    #[test]
    fn sweep_config_none_when_scheduled_processing_disabled() {
        let settings = HalluScribeSettings {
            backend: BackendKind::Ollama,
            scheduled_processing_enabled: false,
            ctx_size: 65_536,
            max_tokens: 32_768,
            ..Default::default()
        };
        assert!(settings
            .to_sweep_config(PathBuf::from("/tmp/hs"), false)
            .is_none());
    }

    #[test]
    fn force_sweep_ignores_scheduled_processing_toggle() {
        let settings = HalluScribeSettings {
            backend: BackendKind::Ollama,
            scheduled_processing_enabled: false,
            ctx_size: 65_536,
            max_tokens: 32_768,
            ..Default::default()
        };
        assert!(settings
            .to_sweep_config(PathBuf::from("/tmp/hs"), true)
            .is_some());
    }

    #[test]
    fn first_run_defaults_to_true() {
        assert!(HalluScribeSettings::default().first_run);
    }

    #[test]
    fn first_run_true_overrides_lookback_to_unlimited() {
        let settings = HalluScribeSettings {
            backend: BackendKind::Ollama,
            first_run: true,
            lookback_hours: 24,
            ctx_size: 65_536,
            max_tokens: 32_768,
            scheduled_processing_enabled: true,
            ..Default::default()
        };
        let cfg = settings
            .to_sweep_config(PathBuf::from("/tmp/hs"), false)
            .unwrap();
        assert_eq!(cfg.lookback_secs, u64::MAX);
    }

    #[test]
    fn first_run_false_uses_lookback_hours() {
        let settings = HalluScribeSettings {
            backend: BackendKind::Ollama,
            first_run: false,
            lookback_hours: 48,
            ctx_size: 65_536,
            max_tokens: 32_768,
            scheduled_processing_enabled: true,
            ..Default::default()
        };
        let cfg = settings
            .to_sweep_config(PathBuf::from("/tmp/hs"), false)
            .unwrap();
        assert_eq!(cfg.lookback_secs, 48 * 3600);
    }

    #[test]
    fn phase8_defaults_require_runtime_limits_to_be_set() {
        let settings = HalluScribeSettings::default();
        assert_eq!(settings.ctx_size, 0);
        assert_eq!(settings.max_tokens, 0);
        assert_eq!(settings.briefing_window_hours, 2);
    }

    #[test]
    fn phase8_fields_round_trip() {
        let dir = tmp();
        let settings = HalluScribeSettings {
            ctx_size: 65_536,
            max_tokens: 24_576,
            briefing_window_hours: 12,
            ..Default::default()
        };
        save_settings(dir.path(), &settings).unwrap();
        let reloaded = load_settings(dir.path()).unwrap();
        assert_eq!(reloaded.ctx_size, 65_536);
        assert_eq!(reloaded.max_tokens, 24_576);
        assert_eq!(reloaded.briefing_window_hours, 12);
    }

    #[test]
    fn phase8_fields_default_when_absent_from_json() {
        let dir = tmp();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"schedule_time": "04:00"}"#,
        )
        .unwrap();
        let settings = load_settings(dir.path()).unwrap();
        assert!(!settings.always_on_top);
        assert_eq!(settings.ctx_size, 0);
        assert_eq!(settings.max_tokens, 0);
        assert_eq!(settings.briefing_window_hours, 2);
        assert!(settings.ollama_api_key.is_empty());
        assert!(settings.tavily_api_key.is_empty());
    }

    #[test]
    fn sweep_config_propagates_force_and_schedule_time() {
        let settings = HalluScribeSettings {
            backend: BackendKind::Ollama,
            first_run: false,
            schedule_time: "03:45".to_string(),
            summary_min_fill_pct: 60.0,
            lookback_hours: 48,
            ctx_size: 65_536,
            max_tokens: 32_768,
            ..Default::default()
        };
        let cfg = settings
            .to_sweep_config(PathBuf::from("/tmp/hs"), true)
            .unwrap();
        assert!(cfg.force);
        assert_eq!(cfg.schedule_time, "03:45");
        assert_eq!(cfg.min_fill_pct, 60.0);
        assert_eq!(cfg.lookback_secs, 48 * 3600);
    }

    #[test]
    fn default_profile_sources_include_coding_tools_and_exclude_personal_chats() {
        let settings = HalluScribeSettings::default();
        for key in [
            "claude_code",
            "codex",
            "forge",
            "halluscribe_agent_chat",
            "ollama_chat",
        ] {
            assert!(
                settings.profile_sources.iter().any(|s| s == key),
                "missing default profile source: {key}"
            );
        }
        for excluded in ["chatgpt", "claude_ai", "gemini"] {
            assert!(
                !settings.profile_sources.iter().any(|s| s == excluded),
                "personal chat provider must be excluded by default: {excluded}"
            );
        }
    }

    #[test]
    fn old_settings_json_without_business_default_country_code_loads_blank() {
        // A settings.json written before the field existed must still load, with
        // the new field defaulting to blank (D8: no guessed country code).
        let dir = tmp();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"schedule_time": "04:00", "ollama_model": "gemma4:12b"}"#,
        )
        .unwrap();
        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(settings.ollama_model, "gemma4:12b");
        assert!(settings.business_default_country_code.is_empty());
    }

    #[test]
    fn business_default_country_code_round_trips() {
        let dir = tmp();
        let settings = HalluScribeSettings {
            business_default_country_code: "30".to_string(),
            ..Default::default()
        };
        save_settings(dir.path(), &settings).unwrap();
        let reloaded = load_settings(dir.path()).unwrap();
        assert_eq!(reloaded.business_default_country_code, "30");
    }

    #[test]
    fn old_settings_json_without_profile_sources_gets_default() {
        let dir = tmp();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"schedule_time": "04:00", "ollama_model": "gemma4:12b"}"#,
        )
        .unwrap();
        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(settings.ollama_model, "gemma4:12b");
        assert_eq!(
            settings.profile_sources,
            HalluScribeSettings::default().profile_sources
        );
    }

    #[test]
    fn profile_sources_round_trip() {
        let dir = tmp();
        let settings = HalluScribeSettings {
            profile_sources: vec!["claude_code".to_string()],
            ..Default::default()
        };
        save_settings(dir.path(), &settings).unwrap();
        let reloaded = load_settings(dir.path()).unwrap();
        assert_eq!(reloaded.profile_sources, vec!["claude_code".to_string()]);
    }

    #[test]
    fn stale_preserve_raw_transcripts_field_is_ignored_not_a_parse_error() {
        // L3: the toggle was removed entirely (raw preservation is now
        // unconditional), but a settings.json written before the removal
        // still has the field on disk. Serde ignores unknown fields by
        // default, so this must still load successfully rather than error.
        let dir = tmp();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"schedule_time": "04:00", "preserve_raw_transcripts": true}"#,
        )
        .unwrap();
        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(settings.schedule_time, "04:00");
    }

    #[test]
    fn generation_limits_require_non_zero_values() {
        let settings = HalluScribeSettings::default();
        assert!(settings.generation_limits().is_err());

        let configured = HalluScribeSettings {
            ctx_size: 65_536,
            max_tokens: 32_768,
            ..Default::default()
        };
        assert_eq!(configured.generation_limits().unwrap(), (65_536, 32_768));
    }
}
