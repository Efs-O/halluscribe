// HalluScribe - where the tuning file lives on this machine.
// The tuning is host-level, so the three spawn sites all need the DEFAULT
// archive root - not the active workspace's. That root is only knowable through
// the Tauri path API, which the sweep thread, the briefing chat and the
// embedding runtime do not each carry an app handle for. It is therefore
// recorded once during setup, exactly as `llama_runtime::set_log_dir` records
// the log directory, and read from here afterwards.

use super::{resolve_for_model, ResolvedTuning};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn host_root() -> &'static OnceLock<PathBuf> {
    static HOST_ROOT: OnceLock<PathBuf> = OnceLock::new();
    &HOST_ROOT
}

/// Record the DEFAULT archive root (`<home>/.halluscribe`) as the home of
/// `llama-tuning.yaml`. Called once during setup; later calls are ignored, so a
/// workspace switch cannot move the host-level tuning file.
pub fn set_host_root(dir: PathBuf) {
    if host_root().set(dir).is_err() {
        eprintln!("[llama] tuning root was already configured; keeping the original location");
    }
}

/// Resolve the tuning governing `model` from this machine's tuning file.
///
/// An unset root is an error rather than a guessed path: the flags decide
/// whether a model fits in VRAM, and a server spawned from a tuning file that
/// was never found would run on built-in defaults while the user's edited file
/// sat unread.
pub fn resolve_host_tuning(model: &Path) -> Result<ResolvedTuning, String> {
    let Some(root) = host_root().get() else {
        return Err(
            "llama-server tuning is unavailable: the archive root was never resolved at startup."
                .to_string(),
        );
    };
    resolve_for_model(root, model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_root_is_an_error_rather_than_a_guessed_path() {
        // The app sets the root during setup. Under `cargo test` nothing does,
        // which is exactly the state this must refuse to paper over. (If a
        // future test sets the root, the OnceLock makes this assertion
        // order-dependent - hence no test here ever sets it.)
        let error = resolve_host_tuning(Path::new("model.gguf")).unwrap_err();
        assert!(error.contains("archive root"), "got: {error}");
    }
}
