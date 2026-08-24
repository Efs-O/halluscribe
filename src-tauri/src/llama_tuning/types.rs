// HalluScribe - llama-server tuning types.
// One block of spawn-time flags per model architecture. Deliberately holds no
// model path, no ctx size and no GPU flag: those already have a single
// authority in Settings, and a second one here would silently override the
// user's own choice.

use super::SamplingTuning;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::process::Command;

/// How the speculative decoder is run, when the model has one.
///
/// Separate from `ArchTuning` because these flags are inert - and on some
/// builds rejected - without a drafter. Emitted by `apply_speculative`, which
/// the MTP path calls only once it knows a drafter exists, so a model without
/// one can never be left with a dangling `--spec-*` flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpeculativeTuning {
    /// `--spec-type`: usually `draft-mtp`; Qwen can combine it with
    /// `ngram-mod` as `draft-mtp,ngram-mod`.
    pub spec_type: String,
    /// `--spec-draft-n-max`: tokens drafted per step.
    pub n_max: u32,
    /// `--spec-draft-ngl`: how much of the DRAFT model goes on the GPU, as
    /// `all`, `auto`, or a layer count. Empty emits no flag. Distinct from the
    /// main model's GPU layers, which Settings owns - a drafter is small enough
    /// to keep entirely on the card even when the main model cannot be.
    pub gpu_layers: String,
    /// `--spec-draft-type-k`: the draft model's own KV cache type. Empty emits
    /// no flag. Usually kept higher-precision than the main model's, since the
    /// draft cache is small and its errors cost rejected tokens.
    pub cache_type_k: String,
    /// `--spec-draft-type-v`.
    pub cache_type_v: String,
}

impl Default for SpeculativeTuning {
    fn default() -> Self {
        Self {
            spec_type: "draft-mtp".to_string(),
            // llama.cpp's own default is 3; 4 is what HalluScribe hardcoded
            // before this file existed, so it stays the built-in default.
            n_max: 4,
            gpu_layers: String::new(),
            cache_type_k: String::new(),
            cache_type_v: String::new(),
        }
    }
}

impl SpeculativeTuning {
    /// Push the draft-model flags. `drafter` is the sidecar GGUF when the model
    /// has one; `None` means the MTP head ships inside the model itself, so no
    /// `--model-draft` is passed and llama-server uses the built-in head.
    pub fn apply(&self, command: &mut Command, drafter: Option<&std::path::Path>) {
        if let Some(drafter) = drafter {
            command.args(["--model-draft", &drafter.to_string_lossy()]);
        }
        command.args(["--spec-type", &self.spec_type]);
        command.args(["--spec-draft-n-max", &self.n_max.to_string()]);
        if !self.gpu_layers.is_empty() {
            command.args(["--spec-draft-ngl", &self.gpu_layers]);
        }
        if !self.cache_type_k.is_empty() {
            command.args(["--spec-draft-type-k", &self.cache_type_k]);
        }
        if !self.cache_type_v.is_empty() {
            command.args(["--spec-draft-type-v", &self.cache_type_v]);
        }
    }
}

/// Spawn-time llama-server flags for one model architecture.
///
/// Every field's serde default reproduces the flag block HalluScribe hardcoded
/// before this file existed, so a partially written or hand-trimmed YAML can
/// never produce a half-configured server.
///
/// `Eq` is deliberately absent: `sampling` carries floats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArchTuning {
    /// `--batch-size`.
    pub batch_size: u32,
    /// `--ubatch-size`. Unset emits no flag, leaving llama.cpp's own default.
    pub ubatch_size: Option<u32>,
    /// `--cache-type-k`, e.g. `q8_0`, `q4_0`, `f16`.
    pub cache_type_k: String,
    /// `--cache-type-v`.
    pub cache_type_v: String,
    /// `--parallel`: how many slots share `--ctx-size` between them.
    pub parallel: u32,
    /// `--flash-attn on|off`.
    pub flash_attn: bool,
    /// `--threads`.
    pub threads: u32,
    /// `--threads-batch`.
    pub threads_batch: u32,
    /// `--kv-unified`: one shared KV cache rather than one per slot. Emitted
    /// only when true.
    pub kv_unified: bool,
    /// Whether the vision projector goes on the GPU. False emits
    /// `--no-mmproj-offload`, keeping it in system RAM - image encoding then
    /// costs CPU time once per image, but zero VRAM, which is what makes a
    /// large model and its draft model fit together on one card.
    pub mmproj_offload: bool,
    /// `--reasoning-budget`: cap on thinking tokens. Unset emits no flag.
    /// Independent of `--reasoning on|off`, which the briefing path sets per
    /// request; this bounds the budget when reasoning is on.
    pub reasoning_budget: Option<u32>,
    /// Draft-model flags, applied only when a drafter is actually in play.
    pub speculative: SpeculativeTuning,
    /// Request-time sampling. Not a command-line flag - merged into the
    /// chat-completions body by `SamplingTuning::apply_to_payload`.
    pub sampling: SamplingTuning,
    /// Flags with no field of their own, passed through verbatim in order.
    pub extra_args: Vec<String>,
}

impl Default for ArchTuning {
    fn default() -> Self {
        Self {
            batch_size: 512,
            ubatch_size: None,
            cache_type_k: "q8_0".to_string(),
            cache_type_v: "q8_0".to_string(),
            parallel: 1,
            flash_attn: true,
            threads: 6,
            threads_batch: 6,
            kv_unified: false,
            mmproj_offload: true,
            reasoning_budget: None,
            speculative: SpeculativeTuning::default(),
            sampling: SamplingTuning::default(),
            extra_args: Vec::new(),
        }
    }
}

impl ArchTuning {
    /// Reject values llama-server would only reject after spawning, when its
    /// stderr is going to a log nobody is reading.
    pub fn validate(&self, key: &str) -> Result<(), String> {
        if self.batch_size == 0 {
            return Err(format!(
                "llama-tuning.yaml: {key}.batch_size must be above 0"
            ));
        }
        if self.parallel == 0 {
            return Err(format!("llama-tuning.yaml: {key}.parallel must be above 0"));
        }
        if self.threads == 0 || self.threads_batch == 0 {
            return Err(format!(
                "llama-tuning.yaml: {key}.threads and {key}.threads_batch must be above 0"
            ));
        }
        if self.cache_type_k.trim().is_empty() || self.cache_type_v.trim().is_empty() {
            return Err(format!(
                "llama-tuning.yaml: {key}.cache_type_k and {key}.cache_type_v must name a type"
            ));
        }
        if self.speculative.spec_type.trim().is_empty() {
            return Err(format!(
                "llama-tuning.yaml: {key}.speculative.spec_type must not be empty"
            ));
        }
        self.sampling.validate(key)
    }

    /// Push this block's flags onto a llama-server command line. Call after
    /// `apply_serve_subcommand`, like every other argument group.
    pub fn apply(&self, command: &mut Command) {
        command.args([
            "--batch-size",
            &self.batch_size.to_string(),
            "--cache-type-k",
            &self.cache_type_k,
            "--cache-type-v",
            &self.cache_type_v,
            "--parallel",
            &self.parallel.to_string(),
            "--flash-attn",
            if self.flash_attn { "on" } else { "off" },
            "--threads",
            &self.threads.to_string(),
            "--threads-batch",
            &self.threads_batch.to_string(),
        ]);
        if let Some(ubatch) = self.ubatch_size {
            command.args(["--ubatch-size", &ubatch.to_string()]);
        }
        if self.kv_unified {
            command.arg("--kv-unified");
        }
        if !self.mmproj_offload {
            command.arg("--no-mmproj-offload");
        }
        if let Some(budget) = self.reasoning_budget {
            command.args(["--reasoning-budget", &budget.to_string()]);
        }
        command.args(&self.extra_args);
    }

    /// Push the draft-model flags for a model that has a drafter. Kept off
    /// `apply` so the ordinary spawn path cannot emit `--spec-*` for a model
    /// with nothing to draft with.
    pub fn apply_speculative(&self, command: &mut Command, drafter: Option<&std::path::Path>) {
        self.speculative.apply(command, drafter);
    }
}

/// The whole `llama-tuning.yaml`.
///
/// Resolution SELECTS one block, it does not merge several. An architecture
/// either has its own entry or it uses `default`, so the flags a server runs
/// with are always exactly one readable block of this file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TuningFile {
    /// Used by any architecture with no entry of its own.
    pub default: ArchTuning,
    /// Keyed by the GGUF header's `general.architecture` - `gemma4`, `qwen35`.
    pub architectures: BTreeMap<String, ArchTuning>,
}

impl TuningFile {
    pub fn validate(&self) -> Result<(), String> {
        self.default.validate("default")?;
        for (key, tuning) in &self.architectures {
            tuning.validate(&format!("architectures.{key}"))?;
        }
        Ok(())
    }

    /// The block governing `architecture`, plus the key it was found under so
    /// callers and logs can say which entry actually applied.
    pub fn select(&self, architecture: &str) -> (&str, &ArchTuning) {
        match self.architectures.get_key_value(architecture) {
            Some((key, tuning)) => (key.as_str(), tuning),
            None => ("default", &self.default),
        }
    }
}
