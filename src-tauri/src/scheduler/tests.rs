#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::super::helpers::{backend_display_name, provider_display_name, sweep_done_message};
    use super::super::types::SweepResult;
    use crate::gemma::InferenceBackend;
    use crate::readers::ChatProvider;
    use std::path::PathBuf;

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

    /// A sweep that finished cleanly, as both entry points now report it.
    fn completed_result() -> SweepResult {
        SweepResult {
            ran: true,
            processed: 12,
            skipped: 3,
            ..Default::default()
        }
    }

    #[test]
    fn done_message_reports_a_clean_sweep_as_complete() {
        assert_eq!(
            sweep_done_message(&completed_result(), &[]),
            "Sweep complete - processed: 12, skipped: 3, deferred: 0"
        );
    }

    #[test]
    fn done_message_reports_deferred_work_as_incomplete() {
        let result = SweepResult {
            deferred: 2,
            ..completed_result()
        };
        assert_eq!(
            sweep_done_message(&result, &[]),
            "Sweep incomplete - processed: 12, skipped: 3, deferred: 2"
        );
    }

    #[test]
    fn done_message_explains_low_signal_skips() {
        let result = SweepResult {
            low_signal_skipped: 9,
            ..completed_result()
        };
        assert_eq!(
            sweep_done_message(&result, &[]),
            "Sweep complete - processed: 12, skipped: 3, deferred: 0, \
             low-signal skipped: 9 (greetings or unsupported capability probes)"
        );
    }

    #[test]
    fn done_message_appends_cancelled_errors_and_marker_errors_in_order() {
        let result = SweepResult {
            cancelled: true,
            errors: vec!["one: boom".to_string(), "two: boom".to_string()],
            ..completed_result()
        };
        let markers = vec!["disk full".to_string()];
        assert_eq!(
            sweep_done_message(&result, &markers),
            "Sweep incomplete - processed: 12, skipped: 3, deferred: 0, cancelled, \
             errors: 2, settings errors: 1"
        );
    }

    #[test]
    fn done_message_reports_flagged_secrets_only_when_present() {
        let result = SweepResult {
            flagged: 4,
            ..completed_result()
        };
        assert_eq!(
            sweep_done_message(&result, &[]),
            "Sweep complete - processed: 12, skipped: 3, deferred: 0, possible secrets flagged: 4"
        );
        assert!(!sweep_done_message(&completed_result(), &[]).contains("flagged"));
    }

    #[test]
    fn done_message_reports_superseded_raws_only_when_present() {
        // The scheduled sweep is the unattended one, so this line existing in
        // exactly one shared builder is the point of the extraction.
        let result = SweepResult {
            superseded: 7,
            ..completed_result()
        };
        assert_eq!(
            sweep_done_message(&result, &[]),
            "Sweep complete - processed: 12, skipped: 3, deferred: 0, \
             earlier raw copies kept in raw/superseded: 7"
        );
        assert!(!sweep_done_message(&completed_result(), &[]).contains("superseded"));
    }
}
