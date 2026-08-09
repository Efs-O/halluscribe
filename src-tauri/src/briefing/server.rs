// HalluScribe - warm briefing server state.
// Keeps one llama-server alive across chat turns and, crucially, remembers what
// it was spawned with. llama-server serves whatever weights it loaded and
// ignores the `model` field in the request body, so reusing a warm server after
// the user picks a different GGUF would silently keep answering from the old
// model until the idle watchdog or a sweep happened to kill it. Matching on the
// spawn spec makes a model change take effect on the next turn instead.

use crate::llama_gpu::GpuConfig;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// The spawn inputs that decide what a running llama-server holds in memory.
/// Any difference means the warm server cannot answer for the new request and
/// must be replaced.
#[derive(PartialEq, Eq, Clone, Debug)]
pub(crate) struct ServerSpec {
    pub(crate) model: PathBuf,
    pub(crate) gpu: GpuConfig,
    pub(crate) ctx_size: u32,
    pub(crate) reasoning_enabled: bool,
    pub(crate) multimodal: bool,
}

impl ServerSpec {
    pub(crate) fn new(
        model: &Path,
        gpu: &GpuConfig,
        ctx_size: u32,
        reasoning_enabled: bool,
        multimodal: bool,
    ) -> Self {
        Self {
            model: model.to_path_buf(),
            gpu: gpu.clone(),
            ctx_size,
            reasoning_enabled,
            multimodal,
        }
    }

    /// Whether a server spawned with `self` can answer a request wanting
    /// `wanted`.
    ///
    /// Every field must match except multimodality, which is a *superset*
    /// rather than a mode: a server holding the mmproj answers text-only turns
    /// perfectly well, so only the reverse - needing vision without it - forces
    /// a reload. Treating it as a plain equality made every switch between an
    /// image turn and a text turn reload the whole model, 18-49 s each time.
    /// The cost of this asymmetry is that the mmproj stays resident (~1.1 GB)
    /// once any image has been sent, until the idle watchdog unloads the server.
    fn can_serve(&self, wanted: &ServerSpec) -> bool {
        self.model == wanted.model
            && self.gpu == wanted.gpu
            && self.ctx_size == wanted.ctx_size
            && self.reasoning_enabled == wanted.reasoning_enabled
            && (self.multimodal || !wanted.multimodal)
    }
}

struct WarmServer {
    child: Child,
    spec: ServerSpec,
    port: u16,
}

fn warm() -> &'static Mutex<Option<WarmServer>> {
    static SERVER: OnceLock<Mutex<Option<WarmServer>>> = OnceLock::new();
    SERVER.get_or_init(|| Mutex::new(None))
}

/// Recover from a poisoned lock (a previous holder panicked) rather than
/// crashing the calling stream thread.
fn lock_warm() -> std::sync::MutexGuard<'static, Option<WarmServer>> {
    match warm().lock() {
        Ok(guard) => guard,
        Err(poison) => poison.into_inner(),
    }
}

/// The warm server's port when it was spawned with exactly `spec` and is still
/// answering. Returns `None` — after killing the stale process — when the spec
/// differs, so the caller spawns a fresh server with the new settings.
pub(crate) fn reusable_port(spec: &ServerSpec) -> Option<u16> {
    let mut guard = lock_warm();
    let server = guard.as_ref()?;
    if server.spec.can_serve(spec) {
        let port = server.port;
        if server_ready(port) {
            // The stored spec is deliberately left alone: a multimodal server
            // reused for a text-only turn is still multimodal, and recording it
            // as text-only would force a reload on the next image.
            return Some(port);
        }
    }
    // Either the settings changed under us or the process died. Both mean the
    // handle is useless; drop it (killing the process if it is still alive) so
    // its VRAM is free before the caller loads the new model.
    if let Some(mut stale) = guard.take() {
        let pid = stale.child.id();
        let _ = stale.child.kill();
        // Block until it is actually gone. `kill` only signals: without the
        // wait we race the dying process, spawning the replacement while the
        // old model still holds its VRAM and its port. Two large models
        // overlapping is enough to fail the new load on a 16 GB card.
        let _ = stale.child.wait();
        crate::llama_pids::unregister(pid);
    }
    None
}

/// Record a freshly spawned, ready server as the warm one.
pub(crate) fn store(child: Child, spec: ServerSpec, port: u16) {
    *lock_warm() = Some(WarmServer { child, spec, port });
}

pub(crate) fn kill_server() {
    let mut guard = lock_warm();
    if let Some(server) = guard.as_mut() {
        let pid = server.child.id();
        let _ = server.child.kill();
        crate::llama_pids::unregister(pid);
    }
    *guard = None;
}

fn server_ready(port: u16) -> bool {
    reqwest::blocking::Client::new()
        .get(format!("http://127.0.0.1:{port}/v1/models"))
        .timeout(Duration::from_secs(2))
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(model: &str, multimodal: bool) -> ServerSpec {
        ServerSpec::new(
            Path::new(model),
            &GpuConfig::layers_only(-1),
            4096,
            false,
            multimodal,
        )
    }

    #[test]
    fn spec_matches_itself() {
        assert_eq!(spec("/models/a.gguf", false), spec("/models/a.gguf", false));
    }

    #[test]
    fn spec_differs_when_the_model_path_changes() {
        assert_ne!(spec("/models/a.gguf", false), spec("/models/b.gguf", false));
    }

    #[test]
    fn spec_differs_when_multimodal_changes() {
        assert_ne!(spec("/models/a.gguf", false), spec("/models/a.gguf", true));
    }

    #[test]
    fn a_multimodal_server_serves_text_only_turns() {
        // The whole point of the asymmetry: switching from an image turn back
        // to a text turn must not reload 13 GB of weights.
        let warm = spec("/models/a.gguf", true);
        assert!(warm.can_serve(&spec("/models/a.gguf", false)));
        assert!(warm.can_serve(&spec("/models/a.gguf", true)));
    }

    #[test]
    fn a_text_only_server_cannot_serve_an_image_turn() {
        let warm = spec("/models/a.gguf", false);
        assert!(!warm.can_serve(&spec("/models/a.gguf", true)));
        assert!(warm.can_serve(&spec("/models/a.gguf", false)));
    }

    #[test]
    fn multimodality_never_excuses_a_different_model_or_flag() {
        let warm = spec("/models/a.gguf", true);
        assert!(!warm.can_serve(&spec("/models/b.gguf", false)));
        assert!(!warm.can_serve(&ServerSpec::new(
            Path::new("/models/a.gguf"),
            &GpuConfig::layers_only(-1),
            8192, // different ctx
            false,
            false
        )));
        assert!(!warm.can_serve(&ServerSpec::new(
            Path::new("/models/a.gguf"),
            &GpuConfig::layers_only(27), // different gpu layers
            4096,
            false,
            false
        )));
        assert!(!warm.can_serve(&ServerSpec::new(
            Path::new("/models/a.gguf"),
            &GpuConfig::layers_only(-1),
            4096,
            true, // different reasoning
            false
        )));
    }

    #[test]
    fn spec_differs_on_each_spawn_flag() {
        let all_layers = GpuConfig::layers_only(-1);
        let base = ServerSpec::new(Path::new("/models/a.gguf"), &all_layers, 4096, false, false);
        assert_ne!(
            base,
            ServerSpec::new(
                Path::new("/models/a.gguf"),
                &GpuConfig::layers_only(20),
                4096,
                false,
                false
            )
        );
        assert_ne!(
            base,
            ServerSpec::new(Path::new("/models/a.gguf"), &all_layers, 8192, false, false)
        );
        assert_ne!(
            base,
            ServerSpec::new(Path::new("/models/a.gguf"), &all_layers, 4096, true, false)
        );
    }

    #[test]
    fn changing_gpu_placement_retires_the_warm_server() {
        // Pinning to a different card changes which GPU holds the weights, so a
        // server spawned before the change cannot answer for the new setting.
        let warm = spec("/models/a.gguf", false);
        let pinned = ServerSpec::new(
            Path::new("/models/a.gguf"),
            &GpuConfig {
                devices: "CUDA1".to_string(),
                ..GpuConfig::layers_only(-1)
            },
            4096,
            false,
            false,
        );
        assert!(!warm.can_serve(&pinned));
    }

    #[test]
    fn reusable_port_is_none_when_no_server_is_warm() {
        kill_server();
        assert_eq!(reusable_port(&spec("/models/a.gguf", false)), None);
    }
}
