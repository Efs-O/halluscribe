// HalluScribe - multi-token-prediction (MTP) speculative decoding for llama.cpp.
// Decides, from the model directory alone, whether llama-server should be asked
// to draft ahead — and with which drafter. Kept out of `llama_runtime` so the
// shared subprocess plumbing stays under the file-size limit.

use crate::llama_tuning::ResolvedTuning;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Markers that describe how a model was PACKAGED rather than which model it
/// is. Unsloth ships one MTP drafter per family, named without any of these, so
/// they have to come off the target name before the two can be compared.
///
/// `qat` is on this list on purpose, and it did not used to be. Quantisation-
/// aware training does produce different weights, and llama-server really did
/// exit while loading `mtp-gemma-4-26B-A4B-it.gguf` against the QAT 26B - which
/// is why an earlier fix concluded the pairing was invalid and blocked it. That
/// conclusion was wrong. Forge runs exactly this pairing daily
/// (`.forge/config.yaml`, `gemma4-26b-a4b-it-qat-q4kxl`, whose own comment calls
/// it "its matching Q8_0 MTP assistant draft model"), so the drafter does belong
/// to this model. The load failure was the flags around it, not the file:
/// HalluScribe passed `--model-draft` with no `--spec-draft-ngl` while GPU
/// layers sat at -2, and it ran on llama.cpp b10237 rather than the b10430 Forge
/// uses. Both are addressed elsewhere now - the tuning file supplies the full
/// draft flag set, and layers are 999.
const PACKAGING_MARKERS: [&str; 2] = ["-ud", "-qat"];

/// Strip the terminal quantisation suffix and any packaging markers from a
/// target name, leaving the model identity a drafter is bound to.
///
/// `gemma-4-26B-A4B-it-qat-UD-Q4_K_XL` reduces to `gemma-4-26b-a4b-it`, which is
/// what `mtp-gemma-4-26B-A4B-it.gguf` names itself for.
fn mtp_target_family(model_stem: &str) -> &str {
    let Some(mut family) = model_stem
        .rfind("-q")
        .filter(|&index| {
            model_stem
                .as_bytes()
                .get(index + 2)
                .is_some_and(u8::is_ascii_digit)
        })
        .map(|index| &model_stem[..index])
    else {
        return model_stem;
    };
    // Markers can stack in either order (`-qat-UD-Q4…`), so peel until none is
    // left rather than assuming one arrangement.
    while let Some(shorter) = PACKAGING_MARKERS
        .iter()
        .find_map(|marker| family.strip_suffix(marker))
    {
        family = shorter;
    }
    family
}

/// Add the MTP speculative-decoding flags when this model can draft ahead.
///
/// Two ways it can. A SIDECAR drafter beside the model gets `--model-draft`
/// pointed at it. A head built INTO the model GGUF - which the header declares
/// and `ResolvedTuning::identity` carries - gets the same `--spec-*` block with
/// no `--model-draft`, because there is no second file to point at. A model with
/// neither gets no flags at all, so swapping the model can never leave a stale
/// `--model-draft` behind to kill the server on load.
///
/// The values come from the model's own block of `llama-tuning.yaml`; a family
/// that drafts badly at 4 tokens can be dialled down without a rebuild.
pub fn apply_mtp_flags(
    command: &mut Command,
    model: &Path,
    resolved: &ResolvedTuning,
) -> Result<(), String> {
    match find_mtp_drafter(model)? {
        Some(drafter) => resolved.tuning.apply_speculative(command, Some(&drafter)),
        // A sidecar wins when both exist: it is a separate, larger drafter that
        // the user deliberately placed there.
        None if resolved.identity.has_builtin_mtp => {
            resolved.tuning.apply_speculative(command, None)
        }
        None => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llama_tuning::{ArchTuning, ModelIdentity, SpeculativeTuning};

    /// A resolved tuning that drafts 2 tokens, so a flag carrying that value is
    /// visibly from the file rather than from a leftover constant.
    fn resolved(has_builtin_mtp: bool) -> ResolvedTuning {
        ResolvedTuning {
            identity: ModelIdentity {
                architecture: "test-arch".to_string(),
                has_builtin_mtp,
            },
            matched_key: "test-arch".to_string(),
            tuning: ArchTuning {
                speculative: SpeculativeTuning {
                    n_max: 2,
                    ..SpeculativeTuning::default()
                },
                ..ArchTuning::default()
            },
        }
    }

    fn args_of(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

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
    fn find_mtp_drafter_pairs_the_qat_variant_with_the_family_drafter() {
        // Unsloth ships ONE drafter per family and the QAT download carries it
        // (along with an mmproj named the same way). Forge runs this exact pair
        // daily. An earlier fix blocked it after a load failure that turned out
        // to be the surrounding flags - see PACKAGING_MARKERS.
        let (dir, model) = model_dir_with(
            "qat",
            "gemma-4-26B-A4B-it-qat-UD-Q4_K_XL.gguf",
            &["mtp-gemma-4-26B-A4B-it.gguf", "mmproj-BF16.gguf"],
        );
        assert_eq!(
            find_mtp_drafter(&model).unwrap(),
            Some(dir.join("mtp-gemma-4-26B-A4B-it.gguf"))
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn target_family_drops_stacked_packaging_markers() {
        // Both markers, in the order the real filename carries them.
        assert_eq!(
            mtp_target_family("gemma-4-26b-a4b-it-qat-ud-q4_k_xl"),
            "gemma-4-26b-a4b-it"
        );
    }

    #[test]
    fn target_family_drops_an_unsloth_dynamic_marker() {
        assert_eq!(
            mtp_target_family("gemma-4-12b-it-ud-q5_k_xl"),
            "gemma-4-12b-it"
        );
    }

    #[test]
    fn a_ud_quant_finds_the_sidecar_sitting_next_to_it() {
        // The regression this fix exists for: the drafter was right there, named
        // without the -UD the model carried, and was never matched.
        let (dir, model) = model_dir_with(
            "ud",
            "gemma-4-12b-it-UD-Q5_K_XL.gguf",
            &["mtp-gemma-4-12b-it.gguf", "mmproj-F16.gguf"],
        );
        assert_eq!(
            find_mtp_drafter(&model).unwrap(),
            Some(dir.join("mtp-gemma-4-12b-it.gguf"))
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_drafter_for_a_different_family_is_still_refused() {
        // Peeling packaging markers must not turn into "close enough": a 12B
        // drafter against a 26B target is the pairing that genuinely cannot load.
        let (dir, model) = model_dir_with(
            "cross-family",
            "gemma-4-26B-A4B-it-qat-UD-Q4_K_XL.gguf",
            &["mtp-gemma-4-12b-it.gguf"],
        );
        assert_eq!(find_mtp_drafter(&model).unwrap(), None);
        let _ = fs::remove_dir_all(dir);
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
        apply_mtp_flags(&mut command, &model, &resolved(false)).unwrap();
        let args = args_of(&command);
        assert!(args.contains(&"--model-draft".to_string()));
        assert!(args.contains(&"--spec-type".to_string()));
        assert!(args.contains(&"draft-mtp".to_string()));
        // The count comes from the tuning file now, not from a constant.
        let index = args
            .iter()
            .position(|arg| arg == "--spec-draft-n-max")
            .unwrap();
        assert_eq!(args[index + 1], "2");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_builtin_mtp_head_drafts_without_a_sidecar_file() {
        // Qwen 3.8 carries its drafter inside the model GGUF. Before the header
        // was read, this model got no speculative decoding at all.
        let (dir, model) = model_dir_with("builtin", "Qwen3.8-27B-Q3_K_M.gguf", &[]);
        let mut command = Command::new("llama-server");
        apply_mtp_flags(&mut command, &model, &resolved(true)).unwrap();
        assert_eq!(
            args_of(&command),
            vec!["--spec-type", "draft-mtp", "--spec-draft-n-max", "2"]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn apply_mtp_flags_is_a_no_op_without_a_drafter() {
        let (dir, model) = model_dir_with("bare", "gemma-4-26B-A4B-it-Q4_K_M.gguf", &[]);
        let mut command = Command::new("llama-server");
        apply_mtp_flags(&mut command, &model, &resolved(false)).unwrap();
        assert_eq!(command.get_args().count(), 0);
        let _ = fs::remove_dir_all(dir);
    }
}
