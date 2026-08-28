// HalluScribe - llama-tuning.yaml load and first-run write.
// Mirrors settings persistence: a missing file is a fresh install, a malformed
// one is an explicit error. Corruption is never read as the user choosing
// defaults, because the difference is a server spawned with the wrong flags.

use super::TuningFile;
use std::fs;
use std::path::{Path, PathBuf};

/// The commented starter file, written on first use so there is something to
/// edit rather than a blank page. Embedded at build time - it is documentation
/// as much as data, and must ship with the binary.
const TEMPLATE: &str = include_str!("template.yaml");

/// Host-level, not per-workspace: these flags describe the machine's cards and
/// cores, not an archive's contents. Always pass the DEFAULT archive root, so
/// every workspace on this machine reads the same tuning.
pub fn tuning_path(default_archive_dir: &Path) -> PathBuf {
    default_archive_dir.join("llama-tuning.yaml")
}

/// Read the tuning file, creating it from the template when absent.
///
/// A parse or validation failure is returned verbatim rather than swallowed:
/// the flags decide whether a model fits in VRAM, so silently falling back to
/// built-in defaults would turn a typo into an unexplained slowdown.
pub fn load_tuning(default_archive_dir: &Path) -> Result<TuningFile, String> {
    let path = tuning_path(default_archive_dir);
    if !path.exists() {
        write_template(default_archive_dir)?;
    }
    let contents =
        fs::read_to_string(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let tuning: TuningFile = serde_yaml::from_str(&contents)
        .map_err(|e| format!("{} is not valid YAML: {e}", path.display()))?;
    tuning.validate()?;
    Ok(tuning)
}

/// Write the commented template, creating the archive directory if needed.
/// Never overwrites an existing file - a user's edits are not ours to discard.
pub fn write_template(default_archive_dir: &Path) -> Result<(), String> {
    let path = tuning_path(default_archive_dir);
    if path.exists() {
        return Ok(());
    }
    fs::create_dir_all(default_archive_dir)
        .map_err(|e| format!("failed to create {}: {e}", default_archive_dir.display()))?;
    crate::atomic_file::write_atomic(&path, TEMPLATE)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::super::SamplingTuning;
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("halluscribe-tuning-{tag}-{unique}"))
    }

    #[test]
    fn the_shipped_template_parses_and_validates() {
        // The template is documentation the user edits by hand; if it ever stops
        // being loadable, every fresh install fails on first spawn.
        let tuning: TuningFile = serde_yaml::from_str(TEMPLATE).expect("template must parse");
        tuning.validate().expect("template must validate");
        assert!(tuning.architectures.contains_key("gemma4"));
        assert!(tuning.architectures.contains_key("qwen35"));
    }

    #[test]
    fn embedding_block_accepts_a_full_context_in_one_physical_batch() {
        // The embedding runtime never shared the generation flag block - it runs
        // a 4x batch and an unquantised cache. Falling to `default` here would
        // quietly quarter its indexing batch, so it needs an entry of its own.
        let tuning: TuningFile = serde_yaml::from_str(TEMPLATE).expect("template must parse");
        let embedding = &tuning.architectures["gemma-embedding"];
        assert_eq!(embedding.batch_size, 4096);
        assert_eq!(embedding.ubatch_size, Some(4096));
        assert_eq!(embedding.cache_type_k, "f16");
        assert_eq!(embedding.cache_type_v, "f16");
        assert!(embedding.flash_attn);
        assert_eq!(embedding.parallel, 1);
        assert_eq!(embedding.threads, 6);
        assert_eq!(embedding.threads_batch, 6);
        // An embedding model never drafts, never samples and has no projector.
        assert_eq!(embedding.sampling, SamplingTuning::default());
        assert!(embedding.reasoning_budget.is_none());
    }

    #[test]
    fn first_load_writes_the_template_and_reads_it_back() {
        let root = temp_root("first");
        let tuning = load_tuning(&root).unwrap();
        assert!(tuning_path(&root).exists(), "template should be written");
        let qwen = &tuning.architectures["qwen35"];
        assert_eq!(qwen.cache_type_k, "q4_0");
        assert_eq!(qwen.speculative.n_max, 2);
        assert!(
            !qwen.mmproj_offload,
            "the projector belongs on the CPU here"
        );
        // Qwen's MTP head is inside the model, so there is no sidecar drafter
        // to place or give a cache to.
        assert!(qwen.speculative.gpu_layers.is_empty());

        assert_eq!(qwen.sampling.top_k, Some(20));
        assert_eq!(qwen.sampling.reasoning_effort.as_deref(), Some("medium"));

        let gemma = &tuning.architectures["gemma4"];
        assert_eq!(gemma.speculative.n_max, 1);
        assert_eq!(gemma.speculative.gpu_layers, "all");
        assert_eq!(gemma.sampling.top_k, Some(64));
        // The two families want genuinely different samplers - that difference
        // is the reason sampling is keyed by architecture at all.
        assert_ne!(gemma.sampling.top_k, qwen.sampling.top_k);
        // Neither family sets a temperature: that belongs to the calling role.
        assert!(tuning.default.sampling == super::super::SamplingTuning::default());
        // The draft cache stays q8_0 while the main cache runs q4_0.
        assert_eq!(gemma.cache_type_k, "q4_0");
        assert_eq!(gemma.speculative.cache_type_k, "q8_0");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_hand_edit_survives_the_next_load() {
        let root = temp_root("edit");
        load_tuning(&root).unwrap();
        fs::write(
            tuning_path(&root),
            "architectures:\n  gemma4:\n    threads: 12\n",
        )
        .unwrap();
        let tuning = load_tuning(&root).unwrap();
        assert_eq!(tuning.architectures["gemma4"].threads, 12);
        // Unspecified fields fall to the built-in defaults, not to the
        // template's values - the file the user wrote is the whole truth.
        assert_eq!(tuning.architectures["gemma4"].cache_type_k, "q8_0");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_yaml_is_an_error_not_a_silent_default() {
        let root = temp_root("bad");
        load_tuning(&root).unwrap();
        fs::write(tuning_path(&root), "architectures: [this is not a map]\n").unwrap();
        let error = load_tuning(&root).unwrap_err();
        assert!(error.contains("not valid YAML"), "got: {error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_unknown_key_is_rejected_rather_than_ignored() {
        let root = temp_root("typo");
        load_tuning(&root).unwrap();
        // `cache_type_kk` is the kind of typo that would otherwise leave the
        // server running q8_0 while the file claims q4_0.
        fs::write(
            tuning_path(&root),
            "architectures:\n  gemma4:\n    cache_type_kk: q4_0\n",
        )
        .unwrap();
        assert!(load_tuning(&root).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_bad_sampling_value_is_rejected_with_its_architecture_named() {
        let root = temp_root("sampling");
        load_tuning(&root).unwrap();
        fs::write(
            tuning_path(&root),
            "architectures:\n  qwen35:\n    sampling:\n      reasoning_effort: maximum\n",
        )
        .unwrap();
        let error = load_tuning(&root).unwrap_err();
        assert!(error.contains("architectures.qwen35"), "got: {error}");
        assert!(error.contains("reasoning_effort"), "got: {error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_zero_thread_count_is_rejected_with_its_key_named() {
        let root = temp_root("zero");
        load_tuning(&root).unwrap();
        fs::write(
            tuning_path(&root),
            "architectures:\n  gemma4:\n    threads: 0\n",
        )
        .unwrap();
        let error = load_tuning(&root).unwrap_err();
        assert!(error.contains("architectures.gemma4"), "got: {error}");
        let _ = fs::remove_dir_all(root);
    }
}
