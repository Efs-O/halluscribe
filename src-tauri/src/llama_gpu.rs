// HalluScribe - GPU placement flags shared by every llama-server we spawn.
// One home for the multi-GPU decision so the sweep, the briefing chat and the
// embedding runtime all place work identically. llama.cpp defaults to
// `--main-gpu 0` across *all* visible devices, so on a mixed-card machine the
// card that happens to enumerate first holds the KV cache and compute buffers.
// Leaving that to PCIe slot order is how a 4 GB display card ends up hosting a
// 13 GB model's scratch space; these flags make the choice explicit instead.

use std::process::Command;

/// Where a llama-server should put the model.
///
/// Every string field is "unset" when empty, and an unset field emits no flag
/// at all — llama.cpp's own default then applies. Nothing here substitutes a
/// value the user did not choose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuConfig {
    /// `--n-gpu-layers`. -1 = all, 0 = let llama.cpp decide, n = exactly n.
    pub layers: i32,
    /// `--device`: comma-separated backend device names (`CUDA0,CUDA1`).
    /// Empty = offload to every device llama.cpp can see.
    pub devices: String,
    /// `--split-mode`: one of `none`, `layer`, `row`, `tensor`.
    pub split_mode: String,
    /// `--tensor-split`: per-device fractions (`0.7,0.3`). Empty = even split.
    pub tensor_split: String,
    /// `--main-gpu`: index of the device holding intermediate results and the
    /// KV cache. -1 = unset.
    pub main_gpu: i32,
}

/// The split modes llama.cpp accepts. Anything else is rejected rather than
/// passed through, so a typo surfaces here instead of as an opaque server exit.
const SPLIT_MODES: [&str; 4] = ["none", "layer", "row", "tensor"];

/// Hand-written rather than derived: `i32::default()` is 0, and 0 is a valid
/// *device index*, so a derived default would silently emit `--main-gpu 0` and
/// pin every server to whichever card enumerated first — the precise behaviour
/// this module exists to stop. -1 is the only correct "unset".
impl Default for GpuConfig {
    fn default() -> Self {
        Self {
            layers: 0,
            devices: String::new(),
            split_mode: String::new(),
            tensor_split: String::new(),
            main_gpu: -1,
        }
    }
}

impl GpuConfig {
    /// A config that only pins the layer count — what every caller had before
    /// multi-GPU placement existed.
    pub fn layers_only(layers: i32) -> Self {
        Self {
            layers,
            ..Self::default()
        }
    }

    /// Reject settings llama-server would only complain about after it has
    /// spawned, when its stderr is going to a log nobody is reading.
    pub fn validate(&self) -> Result<(), String> {
        if !self.split_mode.is_empty() && !SPLIT_MODES.contains(&self.split_mode.as_str()) {
            return Err(format!(
                "GPU split mode must be one of {} (got \"{}\")",
                SPLIT_MODES.join(", "),
                self.split_mode
            ));
        }
        if self.main_gpu < -1 {
            return Err(format!(
                "Main GPU index must be -1 (unset) or a device index (got {})",
                self.main_gpu
            ));
        }
        validate_devices(&self.devices)?;
        validate_tensor_split(&self.tensor_split)
    }

    /// Push this config's flags onto a llama-server command line. Call after
    /// `apply_serve_subcommand`, like every other argument.
    pub fn apply(&self, command: &mut Command) -> Result<(), String> {
        self.validate()?;
        command.args(["--n-gpu-layers", &layers_flag(self.layers)]);
        if !self.devices.is_empty() {
            command.args(["--device", &self.devices]);
        }
        if !self.split_mode.is_empty() {
            command.args(["--split-mode", &self.split_mode]);
        }
        if !self.tensor_split.is_empty() {
            command.args(["--tensor-split", &self.tensor_split]);
        }
        if self.main_gpu >= 0 {
            command.args(["--main-gpu", &self.main_gpu.to_string()]);
        }
        Ok(())
    }
}

/// llama.cpp spells the two sentinel layer counts as words, not numbers.
fn layers_flag(layers: i32) -> String {
    match layers {
        -1 => "all".to_string(),
        0 => "auto".to_string(),
        n => n.to_string(),
    }
}

/// Device names are backend-specific (`CUDA0`, `Vulkan1`, `SYCL0`), so the list
/// is passed through verbatim. Only the shape is checked: llama.cpp reads an
/// empty entry as a device named "" and fails the load.
fn validate_devices(devices: &str) -> Result<(), String> {
    if devices.is_empty() {
        return Ok(());
    }
    if devices.split(',').any(|name| name.trim().is_empty()) {
        return Err(format!(
            "GPU device list has an empty entry: \"{devices}\". Use names from \
             `llama-server --list-devices`, comma-separated (e.g. \"CUDA0,CUDA1\")."
        ));
    }
    Ok(())
}

/// Each entry is the fraction of the model for one device, in device order.
/// An all-zero split gives llama.cpp nowhere to put the weights.
fn validate_tensor_split(split: &str) -> Result<(), String> {
    if split.is_empty() {
        return Ok(());
    }
    let mut total = 0.0_f64;
    for entry in split.split(',') {
        let trimmed = entry.trim();
        let value: f64 = trimmed.parse().map_err(|_| {
            format!("GPU tensor split must be comma-separated numbers (got \"{trimmed}\")")
        })?;
        if value < 0.0 || !value.is_finite() {
            return Err(format!(
                "GPU tensor split entries must be finite and non-negative (got \"{trimmed}\")"
            ));
        }
        total += value;
    }
    if total <= 0.0 {
        return Err(
            "GPU tensor split must not be all zeros - no device would hold the model.".to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_for(config: &GpuConfig) -> Vec<String> {
        let mut command = Command::new("llama-server");
        config.apply(&mut command).expect("valid config");
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn an_unset_config_emits_only_the_layer_count() {
        assert_eq!(
            args_for(&GpuConfig::layers_only(-1)),
            ["--n-gpu-layers", "all"]
        );
    }

    #[test]
    fn the_default_main_gpu_is_unset_not_device_zero() {
        // Regression guard: a derived Default would make this 0, which is a
        // real device index and would pin every server to the first card.
        assert_eq!(GpuConfig::default().main_gpu, -1);
        assert_eq!(GpuConfig::layers_only(-1).main_gpu, -1);
    }

    #[test]
    fn layer_sentinels_are_spelled_as_words() {
        assert_eq!(layers_flag(-1), "all");
        assert_eq!(layers_flag(0), "auto");
        assert_eq!(layers_flag(27), "27");
    }

    #[test]
    fn each_set_field_adds_its_flag() {
        let config = GpuConfig {
            layers: -1,
            devices: "CUDA1".to_string(),
            split_mode: "layer".to_string(),
            tensor_split: "0.7,0.3".to_string(),
            main_gpu: 1,
        };
        assert_eq!(
            args_for(&config),
            [
                "--n-gpu-layers",
                "all",
                "--device",
                "CUDA1",
                "--split-mode",
                "layer",
                "--tensor-split",
                "0.7,0.3",
                "--main-gpu",
                "1",
            ]
        );
    }

    #[test]
    fn main_gpu_stays_off_the_command_line_when_unset() {
        let config = GpuConfig {
            main_gpu: -1,
            ..GpuConfig::layers_only(-1)
        };
        assert!(!args_for(&config).contains(&"--main-gpu".to_string()));
    }

    #[test]
    fn main_gpu_zero_is_a_real_choice_not_an_unset_value() {
        // Device 0 is a legitimate selection, so it must reach llama-server.
        let config = GpuConfig {
            main_gpu: 0,
            ..GpuConfig::layers_only(-1)
        };
        assert!(args_for(&config).contains(&"--main-gpu".to_string()));
    }

    #[test]
    fn an_unknown_split_mode_is_rejected() {
        let config = GpuConfig {
            split_mode: "sideways".to_string(),
            ..GpuConfig::layers_only(-1)
        };
        assert!(config.validate().unwrap_err().contains("split mode"));
    }

    #[test]
    fn every_documented_split_mode_is_accepted() {
        for mode in SPLIT_MODES {
            let config = GpuConfig {
                split_mode: mode.to_string(),
                ..GpuConfig::layers_only(-1)
            };
            assert!(config.validate().is_ok(), "{mode} should be accepted");
        }
    }

    #[test]
    fn a_non_numeric_tensor_split_is_rejected() {
        let config = GpuConfig {
            tensor_split: "0.7,half".to_string(),
            ..GpuConfig::layers_only(-1)
        };
        assert!(config.validate().unwrap_err().contains("tensor split"));
    }

    #[test]
    fn an_all_zero_tensor_split_is_rejected() {
        let config = GpuConfig {
            tensor_split: "0,0".to_string(),
            ..GpuConfig::layers_only(-1)
        };
        assert!(config.validate().unwrap_err().contains("all zeros"));
    }

    #[test]
    fn a_negative_tensor_split_entry_is_rejected() {
        let config = GpuConfig {
            tensor_split: "1,-0.5".to_string(),
            ..GpuConfig::layers_only(-1)
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn an_empty_device_entry_is_rejected() {
        let config = GpuConfig {
            devices: "CUDA0,".to_string(),
            ..GpuConfig::layers_only(-1)
        };
        assert!(config.validate().unwrap_err().contains("empty entry"));
    }

    #[test]
    fn a_below_range_main_gpu_is_rejected() {
        let config = GpuConfig {
            main_gpu: -2,
            ..GpuConfig::layers_only(-1)
        };
        assert!(config.validate().unwrap_err().contains("Main GPU"));
    }

    #[test]
    fn configs_differing_only_in_placement_are_not_equal() {
        // The briefing server reuses a warm process only on an exact spec
        // match, so placement changes must be visible to `PartialEq`.
        let base = GpuConfig::layers_only(-1);
        let pinned = GpuConfig {
            devices: "CUDA1".to_string(),
            ..base.clone()
        };
        assert_ne!(base, pinned);
    }
}
