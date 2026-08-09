#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::super::helpers::{backend_display_name, is_sweep_due, provider_display_name};
    use super::super::types::SweepResult;
    use crate::gemma::InferenceBackend;
    use crate::readers::ChatProvider;
    use std::path::PathBuf;

    #[test]
    fn due_exactly_at_scheduled_time_when_not_run_today() {
        assert!(is_sweep_due(2, 30, "2026-06-17", "02:30", ""));
    }

    #[test]
    fn due_after_scheduled_time_catches_up() {
        // App opened mid-afternoon, having missed the 02:30 window entirely.
        assert!(is_sweep_due(14, 0, "2026-06-17", "02:30", "2026-06-16"));
    }

    #[test]
    fn not_due_before_scheduled_time() {
        assert!(!is_sweep_due(2, 29, "2026-06-17", "02:30", ""));
        assert!(!is_sweep_due(1, 0, "2026-06-17", "02:30", ""));
    }

    #[test]
    fn not_due_when_already_run_today() {
        // Past the time, but today's date is already recorded.
        assert!(!is_sweep_due(14, 0, "2026-06-17", "02:30", "2026-06-17"));
    }

    #[test]
    fn due_again_the_next_day() {
        assert!(is_sweep_due(2, 30, "2026-06-18", "02:30", "2026-06-17"));
    }

    #[test]
    fn midnight_boundary() {
        assert!(is_sweep_due(0, 0, "2026-06-17", "00:00", ""));
        assert!(is_sweep_due(23, 59, "2026-06-17", "00:00", "2026-06-16"));
    }

    #[test]
    fn not_due_for_invalid_schedule_string() {
        assert!(!is_sweep_due(14, 5, "2026-06-17", "25:99", ""));
    }

    #[test]
    fn provider_display_names_are_correct() {
        assert_eq!(
            provider_display_name(&ChatProvider::ClaudeCode),
            "Claude Code"
        );
        assert_eq!(provider_display_name(&ChatProvider::ChatGPT), "ChatGPT");
        assert_eq!(provider_display_name(&ChatProvider::Gemini), "Gemini");
    }

    #[test]
    fn backend_display_llamacpp() {
        let backend = InferenceBackend::LlamaCpp {
            bin: PathBuf::from("/usr/bin/llama-server"),
            model: PathBuf::from("/models/gemma.gguf"),
            port: 8080,
            gpu: crate::llama_gpu::GpuConfig::layers_only(-1),
        };
        assert_eq!(backend_display_name(&backend), "llama.cpp");
    }

    #[test]
    fn backend_display_ollama() {
        let backend = InferenceBackend::Ollama {
            host: "localhost".to_string(),
            port: 11434,
            model: "gemma4:26b".to_string(),
        };
        assert_eq!(backend_display_name(&backend), "Ollama");
    }

    #[test]
    fn sweep_result_default_not_ran() {
        let result = SweepResult::default();
        assert!(!result.ran);
        assert_eq!(result.processed, 0);
        assert_eq!(result.skipped, 0);
        assert_eq!(result.deferred, 0);
        assert!(result.errors.is_empty());
    }
}
