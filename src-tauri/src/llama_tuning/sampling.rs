// HalluScribe - request-time sampling, keyed by model architecture.
// These are JSON fields in the chat-completions body, not command-line flags,
// so they live apart from the spawn-time block even though they share the same
// architecture entry. Gemma and Qwen genuinely want different samplers - top_k
// 64 against top_k 20 - which is why this is a per-architecture setting rather
// than one constant.
//
// NOT here: temperature. HalluScribe picks that per ROLE (the sweep runs
// colder than briefing chat, deliberately), so it stays owned by the calling
// path in Rust. One number here would silently override both.
//
// llama.cpp only. The Ollama backend addresses models by name, not by file, so
// there is no GGUF header to read an architecture from.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Sampling fields added to a chat-completions request.
///
/// Every field is optional and an unset field is OMITTED from the payload
/// entirely, rather than sent as a zero or a null. That makes the default an
/// exact match for what HalluScribe sent before this existed - llama.cpp's own
/// defaults, untouched - so adding the file cannot silently change output.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SamplingTuning {
    /// `top_k`. Gemma is trained for 64; Qwen's coding recipe uses 20.
    pub top_k: Option<u32>,
    /// `top_p` (nucleus sampling).
    pub top_p: Option<f64>,
    /// `min_p`.
    pub min_p: Option<f64>,
    /// `stop`: extra stop strings beyond the model's own EOS token. Normally
    /// unnecessary - llama.cpp reads the stop token from the GGUF's chat
    /// template - and needed only for a model whose template disagrees with
    /// its family, such as a fine-tune that kept a custom turn marker.
    pub stop: Vec<String>,
    /// `reasoning_effort`: `none`, `low`, `medium` or `high`. Distinct from the
    /// spawn-time `--reasoning-budget`, which caps thinking tokens; this asks
    /// the model how hard to think in the first place.
    pub reasoning_effort: Option<String>,
    /// `presence_penalty`: flat penalty on any token that has already appeared,
    /// regardless of how often. 0 disables it. Qwen's instruct (non-thinking)
    /// recipe recommends 1.5 to suppress repetition; its thinking recipe leaves
    /// it at 0.
    pub presence_penalty: Option<f64>,
    /// `repeat_penalty`: penalty scaled by how recently a token appeared.
    ///
    /// Named for llama.cpp, which is the only backend this file drives - model
    /// cards and other runtimes call the same knob `repetition_penalty`, and
    /// sending THAT name to `/v1/chat/completions` would be ignored silently.
    /// 1.0 disables it. Gemma guidance: try 1.05-1.15 if output repeats, and
    /// never above 1.2.
    pub repeat_penalty: Option<f64>,
}

/// The values llama.cpp accepts for `reasoning_effort`. Anything else is
/// rejected here rather than sent, so a typo surfaces as a config error instead
/// of a request the server refuses mid-sweep.
const REASONING_EFFORTS: [&str; 4] = ["none", "low", "medium", "high"];

impl SamplingTuning {
    pub fn validate(&self, key: &str) -> Result<(), String> {
        if let Some(top_k) = self.top_k {
            if top_k == 0 {
                return Err(format!(
                    "llama-tuning.yaml: {key}.sampling.top_k must be above 0 (omit it to leave the sampler alone)"
                ));
            }
        }
        check_probability(self.top_p, key, "top_p")?;
        check_probability(self.min_p, key, "min_p")?;
        if let Some(effort) = &self.reasoning_effort {
            if !REASONING_EFFORTS.contains(&effort.as_str()) {
                return Err(format!(
                    "llama-tuning.yaml: {key}.sampling.reasoning_effort must be one of {} (got \"{effort}\")",
                    REASONING_EFFORTS.join(", ")
                ));
            }
        }
        if let Some(presence) = self.presence_penalty {
            if !(-2.0..=2.0).contains(&presence) {
                return Err(format!(
                    "llama-tuning.yaml: {key}.sampling.presence_penalty must be between -2 and 2 (got {presence})"
                ));
            }
        }
        if let Some(repeat) = self.repeat_penalty {
            // 1.0 is "off" and below 1.0 REWARDS repetition, which is never what
            // the setting is reached for - rejected rather than sent, so a typo
            // like 0.1 surfaces here instead of as a model that loops.
            if repeat < 1.0 {
                return Err(format!(
                    "llama-tuning.yaml: {key}.sampling.repeat_penalty must be 1.0 or above \
                     (1.0 disables it; below 1.0 rewards repetition) (got {repeat})"
                ));
            }
        }
        Ok(())
    }

    /// Merge these fields into a chat-completions payload.
    ///
    /// Only fields the user actually set are written, so the caller's own
    /// values - temperature, max_tokens, the messages themselves - are never
    /// touched, and an empty block is a genuine no-op.
    pub fn apply_to_payload(&self, payload: &mut Value) {
        let Some(object) = payload.as_object_mut() else {
            return;
        };
        if let Some(top_k) = self.top_k {
            object.insert("top_k".to_string(), Value::from(top_k));
        }
        if let Some(top_p) = self.top_p {
            object.insert("top_p".to_string(), Value::from(top_p));
        }
        if let Some(min_p) = self.min_p {
            object.insert("min_p".to_string(), Value::from(min_p));
        }
        if !self.stop.is_empty() {
            object.insert("stop".to_string(), Value::from(self.stop.clone()));
        }
        if let Some(effort) = &self.reasoning_effort {
            object.insert("reasoning_effort".to_string(), Value::from(effort.clone()));
        }
        if let Some(presence) = self.presence_penalty {
            object.insert("presence_penalty".to_string(), Value::from(presence));
        }
        if let Some(repeat) = self.repeat_penalty {
            object.insert("repeat_penalty".to_string(), Value::from(repeat));
        }
    }
}

fn check_probability(value: Option<f64>, key: &str, field: &str) -> Result<(), String> {
    match value {
        Some(value) if !(0.0..=1.0).contains(&value) => Err(format!(
            "llama-tuning.yaml: {key}.sampling.{field} must be between 0 and 1 (got {value})"
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_payload() -> Value {
        serde_json::json!({ "model": "m", "temperature": 0.2, "messages": [] })
    }

    #[test]
    fn an_empty_block_leaves_the_payload_untouched() {
        // The contract that lets sampling config be introduced without changing
        // a single existing request.
        let mut payload = base_payload();
        SamplingTuning::default().apply_to_payload(&mut payload);
        assert_eq!(payload, base_payload());
    }

    #[test]
    fn set_fields_are_added_without_disturbing_the_callers_own() {
        let sampling = SamplingTuning {
            top_k: Some(64),
            top_p: Some(0.95),
            min_p: Some(0.0),
            stop: vec!["<end_of_turn>".to_string()],
            reasoning_effort: Some("medium".to_string()),
            presence_penalty: Some(1.5),
            repeat_penalty: Some(1.1),
        };
        let mut payload = base_payload();
        sampling.apply_to_payload(&mut payload);
        assert_eq!(payload["top_k"], 64);
        assert_eq!(payload["top_p"], 0.95);
        assert_eq!(payload["min_p"], 0.0);
        assert_eq!(payload["stop"], serde_json::json!(["<end_of_turn>"]));
        assert_eq!(payload["reasoning_effort"], "medium");
        assert_eq!(payload["presence_penalty"], 1.5);
        assert_eq!(payload["repeat_penalty"], 1.1);
        // The role's own temperature survives - it is not ours to overwrite.
        assert_eq!(payload["temperature"], 0.2);
    }

    #[test]
    fn the_penalty_key_sent_is_llama_cpps_name_not_the_model_cards() {
        // Model cards say `repetition_penalty`; llama.cpp's server parses
        // `repeat_penalty` and ignores anything else without complaining, so
        // the wrong name here would be a silent no-op.
        let sampling = SamplingTuning {
            repeat_penalty: Some(1.15),
            ..SamplingTuning::default()
        };
        let mut payload = base_payload();
        sampling.apply_to_payload(&mut payload);
        assert!(payload.get("repeat_penalty").is_some());
        assert!(payload.get("repetition_penalty").is_none());
    }

    #[test]
    fn a_repeat_penalty_below_one_is_rejected() {
        let sampling = SamplingTuning {
            repeat_penalty: Some(0.1),
            ..SamplingTuning::default()
        };
        let error = sampling.validate("architectures.gemma4").unwrap_err();
        assert!(error.contains("1.0 or above"), "got: {error}");
    }

    #[test]
    fn an_out_of_range_presence_penalty_is_rejected() {
        let sampling = SamplingTuning {
            presence_penalty: Some(3.0),
            ..SamplingTuning::default()
        };
        let error = sampling.validate("architectures.qwen35").unwrap_err();
        assert!(error.contains("between -2 and 2"), "got: {error}");
    }

    #[test]
    fn min_p_of_zero_is_sent_rather_than_treated_as_unset() {
        // 0 is a meaningful min_p (disable the filter), so Option must carry the
        // difference between "set to 0" and "not configured".
        let sampling = SamplingTuning {
            min_p: Some(0.0),
            ..SamplingTuning::default()
        };
        let mut payload = base_payload();
        sampling.apply_to_payload(&mut payload);
        assert!(payload.get("min_p").is_some());
    }

    #[test]
    fn an_out_of_range_probability_is_rejected() {
        let sampling = SamplingTuning {
            top_p: Some(1.5),
            ..SamplingTuning::default()
        };
        let error = sampling.validate("architectures.gemma4").unwrap_err();
        assert!(error.contains("between 0 and 1"), "got: {error}");
    }

    #[test]
    fn an_unknown_reasoning_effort_is_rejected_with_the_valid_set_named() {
        let sampling = SamplingTuning {
            reasoning_effort: Some("maximum".to_string()),
            ..SamplingTuning::default()
        };
        let error = sampling.validate("architectures.qwen35").unwrap_err();
        assert!(error.contains("none, low, medium, high"), "got: {error}");
    }

    #[test]
    fn a_zero_top_k_is_rejected_rather_than_silently_disabling_the_sampler() {
        let sampling = SamplingTuning {
            top_k: Some(0),
            ..SamplingTuning::default()
        };
        assert!(sampling.validate("architectures.gemma4").is_err());
    }
}
