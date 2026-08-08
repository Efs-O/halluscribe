// HalluScribe - multi-token-prediction (MTP) speculative decoding for llama.cpp.
// Decides, from the model directory alone, whether llama-server should be asked
// to draft ahead — and with which drafter. Kept out of `llama_runtime` so the
// shared subprocess plumbing stays under the file-size limit.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// How many tokens the MTP head drafts per step. llama.cpp's own default is 3.
/// Not a user setting: the best value is a property of the drafter/GPU pair, not
/// of the archive, and a wrong one costs throughput rather than correctness.
const SPEC_DRAFT_N_MAX: &str = "4";

/// Locate the multi-token-prediction drafter GGUF belonging to `model`.
///
/// An MTP drafter is bound to one target model — its head shares that model's
/// embedding table — so pointing llama-server at a foreign one makes it exit
/// before the HTTP port ever opens. There is therefore no config toggle for it:
/// the drafter is derived from what sits next to the model, exactly as `--mmproj`
/// is. Drafters ship as `mtp-<family>.gguf`, so one claims a model when the
/// model's *pre-quantisation* file name exactly matches that family. A QAT
/// variant is a distinct model identity, not merely a quantisation of the
/// base model: its MTP drafter must therefore identify the QAT variant too.
///
/// `Ok(None)` is the ordinary answer for a non-MTP model and means the flags are
/// simply not requested. `Err` means two drafters both claim this model, where
/// picking either would be a guess.
pub fn find_mtp_drafter(model: &Path) -> Result<Option<PathBuf>, String> {
    let Some(dir) = model.parent() else {
        return Ok(None);
    };
    let model_stem = model
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(None);
    };
    let mut matches: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let lowered = name.to_ascii_lowercase();
        let Some(family) = lowered
            .strip_prefix("mtp-")
            .and_then(|rest| rest.strip_suffix(".gguf"))
        else {
            continue;
        };
        if mtp_target_family(&model_stem) == family {
            matches.push(path);
        }
    }
    if matches.len() > 1 {
        let names = matches
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Found multiple MTP drafter GGUF files claiming {}: {names}. Leave exactly one next to the model.",
            model.display()
        ));
    }
    Ok(matches.pop())
}

/// Remove only the terminal `-Q…` quantisation suffix from a target name.
///
/// `gemma-…-qat-UD-Q4_K_XL` keeps its `-qat-UD` identity, so it cannot be
/// paired with the base `mtp-gemma-….gguf` drafter. The previous prefix check
/// made precisely that invalid pairing and llama-server exited during load.
fn mtp_target_family(model_stem: &str) -> &str {
    model_stem
        .rfind("-q")
        .filter(|&index| {
            model_stem
                .as_bytes()
                .get(index + 2)
                .is_some_and(u8::is_ascii_digit)
        })
        .map(|index| &model_stem[..index])
        .unwrap_or(model_stem)
}

/// Add the MTP speculative-decoding flags when a drafter sits beside `model`.
/// A model with no drafter gets no flags, so a single argument list serves both
/// MTP and non-MTP models and swapping the model can never leave a stale
/// `--model-draft` behind to kill the server on load.
pub fn apply_mtp_flags(command: &mut Command, model: &Path) -> Result<(), String> {
    let Some(drafter) = find_mtp_drafter(model)? else {
        return Ok(());
    };
    command.args([
        "--model-draft",
        &drafter.to_string_lossy(),
        "--spec-type",
        "draft-mtp",
        "--spec-draft-n-max",
        SPEC_DRAFT_N_MAX,
    ]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a throwaway model directory containing `files`, returning its path
    /// and the path of the model itself.
    fn model_dir_with(tag: &str, model: &str, files: &[&str]) -> (PathBuf, PathBuf) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("halluscribe-mtp-{tag}-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        for name in files {
            fs::write(dir.join(name), "").unwrap();
        }
        fs::write(dir.join(model), "").unwrap();
        let model_path = dir.join(model);
        (dir, model_path)
    }

    #[test]
    fn find_mtp_drafter_matches_the_quantised_target_model() {
        let (dir, model) = model_dir_with(
            "hit",
            "gemma-4-26B-A4B-it-Q4_K_XL.gguf",
            &["mtp-gemma-4-26B-A4B-it.gguf", "mmproj-BF16.gguf"],
        );
        let found = find_mtp_drafter(&model).unwrap();
        assert_eq!(found, Some(dir.join("mtp-gemma-4-26B-A4B-it.gguf")));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn find_mtp_drafter_rejects_base_drafter_for_qat_variant() {
        let (dir, model) = model_dir_with(
            "qat-miss",
            "gemma-4-26B-A4B-it-qat-UD-Q4_K_XL.gguf",
            &["mtp-gemma-4-26B-A4B-it.gguf"],
        );
        assert_eq!(find_mtp_drafter(&model).unwrap(), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn target_family_keeps_qat_identity_but_drops_quantisation() {
        assert_eq!(
            mtp_target_family("gemma-4-26b-a4b-it-qat-ud-q4_k_xl"),
            "gemma-4-26b-a4b-it-qat-ud"
        );
    }

    #[test]
    fn find_mtp_drafter_ignores_a_drafter_for_another_model() {
        let (dir, model) = model_dir_with(
            "miss",
            "Qwen3.6-30B-Q4_K_M.gguf",
            &["mtp-gemma-4-26B-A4B-it.gguf"],
        );
        // Swapping in a model the drafter does not claim must emit no flags at
        // all, rather than a --model-draft that kills the server on load.
        assert_eq!(find_mtp_drafter(&model).unwrap(), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn find_mtp_drafter_ignores_a_less_specific_base_drafter() {
        let (dir, model) = model_dir_with(
            "ambiguous",
            "gemma-4-26B-A4B-it-Q4_K_XL.gguf",
            &["mtp-gemma-4-26B-A4B-it.gguf", "mtp-gemma-4-26B.gguf"],
        );
        assert_eq!(
            find_mtp_drafter(&model).unwrap(),
            Some(dir.join("mtp-gemma-4-26B-A4B-it.gguf"))
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn apply_mtp_flags_appends_the_speculative_decoding_trio() {
        let (dir, model) = model_dir_with(
            "flags",
            "gemma-4-26B-A4B-it-Q4_K_XL.gguf",
            &["mtp-gemma-4-26B-A4B-it.gguf"],
        );
        let mut command = Command::new("llama-server");
        apply_mtp_flags(&mut command, &model).unwrap();
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"--spec-type".to_string()));
        assert!(args.contains(&"draft-mtp".to_string()));
        assert!(args.contains(&"--spec-draft-n-max".to_string()));
        assert!(args.contains(&SPEC_DRAFT_N_MAX.to_string()));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn apply_mtp_flags_is_a_no_op_without_a_drafter() {
        let (dir, model) = model_dir_with("bare", "gemma-4-26B-A4B-it-Q4_K_M.gguf", &[]);
        let mut command = Command::new("llama-server");
        apply_mtp_flags(&mut command, &model).unwrap();
        assert_eq!(command.get_args().count(), 0);
        let _ = fs::remove_dir_all(dir);
    }
}
