// HalluScribe - per-architecture llama-server tuning.
// One home for the spawn-time flags the sweep, the briefing chat and the
// embedding runtime all need, keyed by what the model's own GGUF header says it
// is. Replaces three copies of the same hardcoded argument block, and lets a
// Gemma and a Qwen get the flags each actually wants without a rebuild.
//
// Design and staging: docs/internal/LLAMA_TUNING_PLAN.md

mod gguf;
mod host;
mod persistence;
mod sampling;
mod types;

pub use gguf::{read_identity, ModelIdentity};
pub use host::{resolve_host_tuning, set_host_root};
pub use persistence::{load_tuning, tuning_path, write_template};
pub use sampling::SamplingTuning;
pub use types::{ArchTuning, SpeculativeTuning, TuningFile};

use std::path::Path;

/// The tuning that governs one model, with the evidence for why.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTuning {
    /// What the model's header declared.
    pub identity: ModelIdentity,
    /// Which key of `llama-tuning.yaml` supplied the flags - an architecture
    /// name, or `default`. Reported so a surprising flag set can be traced back
    /// to the block that produced it.
    pub matched_key: String,
    pub tuning: ArchTuning,
}

/// Resolve the flags for `model`, reading both the tuning file and the model's
/// GGUF header.
///
/// `default_archive_dir` must be the DEFAULT archive root, not the active
/// workspace's: tuning is host-level.
pub fn resolve_for_model(
    default_archive_dir: &Path,
    model: &Path,
) -> Result<ResolvedTuning, String> {
    let file = load_tuning(default_archive_dir)?;
    let identity = read_identity(model)?;
    let (matched_key, tuning) = file.select(&identity.architecture);
    Ok(ResolvedTuning {
        identity,
        matched_key: matched_key.to_string(),
        tuning: tuning.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    fn args_of(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn default_tuning_reproduces_the_previously_hardcoded_flag_block() {
        // This is the contract that lets the tuning file be introduced without
        // changing how any existing install runs. The list is the block that
        // gemma/llamacpp.rs and briefing/llamacpp.rs spelled out by hand.
        let mut command = Command::new("llama-server");
        ArchTuning::default().apply(&mut command);
        assert_eq!(
            args_of(&command),
            vec![
                "--batch-size",
                "512",
                "--cache-type-k",
                "q8_0",
                "--cache-type-v",
                "q8_0",
                "--parallel",
                "1",
                "--flash-attn",
                "on",
                "--threads",
                "6",
                "--threads-batch",
                "6",
            ]
        );
    }

    #[test]
    fn extra_args_are_appended_verbatim_and_in_order() {
        let tuning = ArchTuning {
            extra_args: vec!["--no-mmproj-offload".into(), "--kv-unified".into()],
            ..ArchTuning::default()
        };
        let mut command = Command::new("llama-server");
        tuning.apply(&mut command);
        let args = args_of(&command);
        let tail = &args[args.len() - 2..];
        assert_eq!(tail, ["--no-mmproj-offload", "--kv-unified"]);
    }

    #[test]
    fn an_unset_ubatch_size_emits_no_flag() {
        let mut command = Command::new("llama-server");
        ArchTuning::default().apply(&mut command);
        assert!(!args_of(&command).contains(&"--ubatch-size".to_string()));
    }

    #[test]
    fn flash_attn_false_emits_off_rather_than_omitting_the_flag() {
        // llama.cpp's own default has changed between builds, so "off" must be
        // stated rather than implied by absence.
        let tuning = ArchTuning {
            flash_attn: false,
            ..ArchTuning::default()
        };
        let mut command = Command::new("llama-server");
        tuning.apply(&mut command);
        let args = args_of(&command);
        let index = args.iter().position(|a| a == "--flash-attn").unwrap();
        assert_eq!(args[index + 1], "off");
    }

    /// Print the flag line each real GGUF in `HALLUSCRIBE_TEST_GGUF` (a
    /// `;`-separated list) would now be spawned with, resolved against a fresh
    /// copy of the shipped template - which is exactly what a first run writes.
    /// Silently passes when the variable is unset, so CI stays green on a
    /// machine with no models. Run with:
    ///   cargo test --lib llama_tuning::tests::prints_the -- --nocapture
    #[test]
    fn prints_the_resolved_flag_line_for_a_real_gguf_when_one_is_pointed_at() {
        let Ok(list) = std::env::var("HALLUSCRIBE_TEST_GGUF") else {
            return;
        };
        let root =
            std::env::temp_dir().join(format!("halluscribe-tuning-preview-{}", std::process::id()));
        for entry in list.split(';').filter(|entry| !entry.trim().is_empty()) {
            let model = Path::new(entry.trim());
            let resolved = resolve_for_model(&root, model).expect("resolve");
            let mut command = Command::new("llama-server");
            resolved.tuning.apply(&mut command);
            crate::llama_mtp::apply_mtp_flags(&mut command, model, &resolved).expect("mtp");
            println!(
                "\n{}\n  block={} arch={} builtin_mtp={}\n  {}",
                model.display(),
                resolved.matched_key,
                resolved.identity.architecture,
                resolved.identity.has_builtin_mtp,
                args_of(&command).join(" ")
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_unknown_architecture_selects_the_default_block_by_name() {
        let file = TuningFile::default();
        let (key, _) = file.select("some-future-model");
        assert_eq!(key, "default");
    }

    #[test]
    fn a_known_architecture_selects_its_own_block() {
        let mut file = TuningFile::default();
        file.architectures.insert(
            "qwen35".to_string(),
            ArchTuning {
                cache_type_k: "q4_0".to_string(),
                speculative: SpeculativeTuning {
                    n_max: 2,
                    ..SpeculativeTuning::default()
                },
                ..ArchTuning::default()
            },
        );
        let (key, tuning) = file.select("qwen35");
        assert_eq!(key, "qwen35");
        assert_eq!(tuning.cache_type_k, "q4_0");
        assert_eq!(tuning.speculative.n_max, 2);
    }

    #[test]
    fn the_optional_placement_flags_are_absent_unless_asked_for() {
        // Each of these must be opt-in: emitting --kv-unified or
        // --no-mmproj-offload by default would change VRAM layout for every
        // existing install without anyone asking.
        let mut command = Command::new("llama-server");
        ArchTuning::default().apply(&mut command);
        let args = args_of(&command);
        for flag in ["--kv-unified", "--no-mmproj-offload", "--reasoning-budget"] {
            assert!(!args.contains(&flag.to_string()), "{flag} should be absent");
        }
    }

    #[test]
    fn placement_flags_are_emitted_when_set() {
        let tuning = ArchTuning {
            kv_unified: true,
            mmproj_offload: false,
            reasoning_budget: Some(4096),
            ..ArchTuning::default()
        };
        let mut command = Command::new("llama-server");
        tuning.apply(&mut command);
        let args = args_of(&command);
        assert!(args.contains(&"--kv-unified".to_string()));
        assert!(args.contains(&"--no-mmproj-offload".to_string()));
        let index = args.iter().position(|a| a == "--reasoning-budget").unwrap();
        assert_eq!(args[index + 1], "4096");
    }

    #[test]
    fn a_sidecar_drafter_gets_model_draft_and_its_own_cache_types() {
        let tuning = ArchTuning {
            speculative: SpeculativeTuning {
                n_max: 1,
                gpu_layers: "all".to_string(),
                cache_type_k: "q8_0".to_string(),
                cache_type_v: "q8_0".to_string(),
            },
            // The main cache runs q4_0 while the draft cache runs q8_0 - the
            // two must not be conflated.
            cache_type_k: "q4_0".to_string(),
            ..ArchTuning::default()
        };
        let mut command = Command::new("llama-server");
        tuning.apply_speculative(&mut command, Some(Path::new("/models/mtp-gemma-4.gguf")));
        let args = args_of(&command);
        assert_eq!(
            args,
            vec![
                "--model-draft",
                "/models/mtp-gemma-4.gguf",
                "--spec-type",
                "draft-mtp",
                "--spec-draft-n-max",
                "1",
                "--spec-draft-ngl",
                "all",
                "--spec-draft-type-k",
                "q8_0",
                "--spec-draft-type-v",
                "q8_0",
            ]
        );
    }

    #[test]
    fn a_builtin_mtp_head_gets_no_model_draft_flag() {
        // Qwen 3.8's drafter is inside the model GGUF. Passing --model-draft
        // with nothing to point it at is what kills llama-server on load.
        let tuning = ArchTuning {
            speculative: SpeculativeTuning {
                n_max: 2,
                ..SpeculativeTuning::default()
            },
            ..ArchTuning::default()
        };
        let mut command = Command::new("llama-server");
        tuning.apply_speculative(&mut command, None);
        assert_eq!(
            args_of(&command),
            vec!["--spec-type", "draft-mtp", "--spec-draft-n-max", "2"]
        );
    }

    #[test]
    fn ordinary_spawn_never_emits_speculative_flags() {
        // apply() and apply_speculative() are separate precisely so a model
        // with no drafter cannot end up with a dangling --spec-* flag.
        let mut command = Command::new("llama-server");
        ArchTuning::default().apply(&mut command);
        assert!(!args_of(&command).iter().any(|a| a.starts_with("--spec")));
    }
}
