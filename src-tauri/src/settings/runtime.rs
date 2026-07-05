// HalluScribe - settings runtime conversions (backend, sweep config, seeding).
use super::{BackendKind, HalluScribeSettings};
use crate::gemma::InferenceBackend;
use crate::scheduler::SweepConfig;
use std::path::PathBuf;

impl HalluScribeSettings {
    /// Seed a NEW workspace's settings from this (the default root's) settings.
    /// Host-level fields — the machine's inference setup — are inherited so a guest
    /// doesn't re-enter model config. Archive-level fields (import paths, preserve_raw,
    /// profile_sources, schedule, first_run, last_sweep_date, …) start fresh at their
    /// defaults. See docs/internal/PERSONAL_PARITY_PLAN.md §E cautions.
    pub fn seed_workspace_settings(&self) -> HalluScribeSettings {
        HalluScribeSettings {
            backend: self.backend.clone(),
            llama_server_bin: self.llama_server_bin.clone(),
            gemma_model_path: self.gemma_model_path.clone(),
            embedding_model_path: self.embedding_model_path.clone(),
            gpu_layers: self.gpu_layers,
            llama_server_port: self.llama_server_port,
            ollama_host: self.ollama_host.clone(),
            ollama_port: self.ollama_port,
            ollama_model: self.ollama_model.clone(),
            ollama_api_key: self.ollama_api_key.clone(),
            tavily_api_key: self.tavily_api_key.clone(),
            ctx_size: self.ctx_size,
            max_tokens: self.max_tokens,
            idle_threshold_mins: self.idle_threshold_mins,
            ..HalluScribeSettings::default()
        }
    }

    pub fn generation_limits(&self) -> Result<(u32, u32), String> {
        if self.ctx_size == 0 || self.max_tokens == 0 {
            return Err(
                "Context window and Max output tokens must be set in Settings before generation can run."
                    .to_string(),
            );
        }
        Ok((self.ctx_size, self.max_tokens))
    }

    /// Build the `InferenceBackend` from current settings.
    /// Returns `None` if the backend is llama.cpp but bin or model path is empty.
    pub fn to_inference_backend(&self) -> Option<InferenceBackend> {
        match self.backend {
            BackendKind::LlamaCpp => {
                if self.llama_server_bin.is_empty() || self.gemma_model_path.is_empty() {
                    return None;
                }
                Some(InferenceBackend::LlamaCpp {
                    bin: PathBuf::from(&self.llama_server_bin),
                    model: PathBuf::from(&self.gemma_model_path),
                    port: self.llama_server_port,
                    gpu_layers: self.gpu_layers,
                })
            }
            BackendKind::Ollama => Some(InferenceBackend::Ollama {
                host: self.ollama_host.clone(),
                port: self.ollama_port,
                model: self.ollama_model.clone(),
            }),
        }
    }

    /// Build a `SweepConfig` from current settings.
    /// Returns `None` if the backend config is invalid.
    pub fn to_sweep_config(&self, archive_dir: PathBuf, force: bool) -> Option<SweepConfig> {
        if !force && !self.scheduled_processing_enabled {
            return None;
        }
        let backend = self.to_inference_backend()?;
        let (ctx_size, max_tokens) = self.generation_limits().ok()?;
        let lookback_secs = if self.first_run {
            u64::MAX
        } else {
            self.lookback_hours * 3600
        };

        Some(SweepConfig {
            archive_dir,
            backend,
            settings: self.clone(),
            ctx_size,
            max_tokens,
            min_fill_pct: self.summary_min_fill_pct,
            lookback_secs,
            force,
            schedule_time: self.schedule_time.clone(),
            last_sweep_date: self.last_sweep_date.clone(),
            import_only: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_workspace_settings_inherits_host_resets_archive() {
        let host = HalluScribeSettings {
            backend: BackendKind::Ollama,
            llama_server_bin: "/usr/bin/llama-server".to_string(),
            gemma_model_path: "/models/gemma.gguf".to_string(),
            gpu_layers: 20,
            ctx_size: 1000,
            max_tokens: 500,
            first_run: false,
            chatgpt_import_path: "X".to_string(),
            preserve_raw_transcripts: false,
            ..Default::default()
        };
        let seeded = host.seed_workspace_settings();

        // Host-level fields are inherited.
        assert_eq!(seeded.backend, BackendKind::Ollama);
        assert_eq!(seeded.llama_server_bin, "/usr/bin/llama-server");
        assert_eq!(seeded.gemma_model_path, "/models/gemma.gguf");
        assert_eq!(seeded.gpu_layers, 20);
        assert_eq!(seeded.ctx_size, 1000);
        assert_eq!(seeded.max_tokens, 500);

        // Archive-level fields are reset to defaults.
        assert!(seeded.first_run);
        assert_eq!(seeded.chatgpt_import_path, "");
        assert!(seeded.preserve_raw_transcripts);
        assert_eq!(
            seeded.profile_sources,
            HalluScribeSettings::default().profile_sources
        );
    }
}
