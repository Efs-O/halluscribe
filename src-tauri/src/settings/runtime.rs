use super::{BackendKind, HalluScribeSettings};
use crate::gemma::InferenceBackend;
use crate::scheduler::SweepConfig;
use std::path::PathBuf;

impl HalluScribeSettings {
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
        })
    }
}
