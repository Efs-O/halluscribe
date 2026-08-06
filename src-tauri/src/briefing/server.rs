// HalluScribe - warm briefing server state.
// Keeps one llama-server alive across chat turns and, crucially, remembers what
// it was spawned with. llama-server serves whatever weights it loaded and
// ignores the `model` field in the request body, so reusing a warm server after
// the user picks a different GGUF would silently keep answering from the old
// model until the idle watchdog or a sweep happened to kill it. Matching on the
// spawn spec makes a model change take effect on the next turn instead.

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
    pub(crate) gpu_layers: i32,
    pub(crate) ctx_size: u32,
    pub(crate) reasoning_enabled: bool,
    pub(crate) multimodal: bool,
}

impl ServerSpec {
    pub(crate) fn new(
        model: &Path,
        gpu_layers: i32,
        ctx_size: u32,
        reasoning_enabled: bool,
        multimodal: bool,
    ) -> Self {
        Self {
            model: model.to_path_buf(),
            gpu_layers,
            ctx_size,
            reasoning_enabled,
            multimodal,
        }
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
    if &server.spec == spec {
        let port = server.port;
        if server_ready(port) {
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
        ServerSpec::new(Path::new(model), -1, 4096, false, multimodal)
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
    fn spec_differs_on_each_spawn_flag() {
        let base = ServerSpec::new(Path::new("/models/a.gguf"), -1, 4096, false, false);
        assert_ne!(
            base,
            ServerSpec::new(Path::new("/models/a.gguf"), 20, 4096, false, false)
        );
        assert_ne!(
            base,
            ServerSpec::new(Path::new("/models/a.gguf"), -1, 8192, false, false)
        );
        assert_ne!(
            base,
            ServerSpec::new(Path::new("/models/a.gguf"), -1, 4096, true, false)
        );
    }

    #[test]
    fn reusable_port_is_none_when_no_server_is_warm() {
        kill_server();
        assert_eq!(reusable_port(&spec("/models/a.gguf", false)), None);
    }
}
