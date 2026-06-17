// HalluScribe - shared llama-server runtime helpers.
// Single home for the subprocess plumbing the gemma, briefing, and embedding
// runtimes all need: free-port scanning, binary resolution, the Windows
// no-window guard, and readiness polling. Extracted so a fix lands once
// instead of drifting across three near-identical copies (audit Q-1).

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

/// Why a spawned llama-server never became ready. Callers map this to their
/// own error type (`GemmaError`, formatted `String`, ...).
pub enum ServerWaitError {
    /// The child process exited before the HTTP endpoint came up.
    ExitedEarly,
    /// The readiness deadline elapsed while the child was still running.
    Timeout,
}

/// Scans upward from `start` (up to 20 ports) and returns the first port that
/// is not bound by any process. Falls back to `start` if all are taken,
/// letting the OS error surface naturally on spawn.
pub fn find_free_port(start: u16) -> u16 {
    for offset in 0u16..20 {
        let port = start.saturating_add(offset);
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    start
}

/// Resolve a llama-server binary path. Absolute paths are used as-is (with a
/// `.exe` fallback on Windows); a bare name is looked up on `PATH` via `which`.
/// Returns `None` when nothing resolves, so each caller can format its own
/// "binary not found" error.
pub fn resolve_bin(bin: &Path) -> Option<PathBuf> {
    if bin.is_absolute() {
        if bin.exists() {
            return Some(bin.to_path_buf());
        }
        #[cfg(target_os = "windows")]
        if bin.extension().is_none() {
            let with_exe = bin.with_extension("exe");
            if with_exe.exists() {
                return Some(with_exe);
            }
        }
        return None;
    }
    which::which(bin).ok()
}

/// Apply the Windows `CREATE_NO_WINDOW` flag so a spawned llama-server never
/// flashes a console. No-op on other platforms.
pub fn apply_no_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}

/// Poll `GET /v1/models` once per second until the server answers 200, the
/// child exits, or `timeout_secs` elapses.
pub fn wait_until_ready(
    port: u16,
    child: &mut Child,
    timeout_secs: u32,
) -> Result<(), ServerWaitError> {
    let client = reqwest::blocking::Client::new();
    let url = format!("http://127.0.0.1:{port}/v1/models");
    for _ in 0..timeout_secs {
        thread::sleep(Duration::from_secs(1));
        if child.try_wait().ok().flatten().is_some() {
            return Err(ServerWaitError::ExitedEarly);
        }
        if client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .map(|response| response.status().is_success())
            .unwrap_or(false)
        {
            return Ok(());
        }
    }
    Err(ServerWaitError::Timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_free_port_returns_a_bindable_port() {
        let port = find_free_port(49_500);
        // The returned port must itself be bindable right now.
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    }

    #[test]
    fn resolve_bin_returns_none_for_missing_absolute_path() {
        let missing = Path::new("/nonexistent/halluscribe/llama-server-xyz");
        assert!(resolve_bin(missing).is_none());
    }

    #[test]
    fn resolve_bin_accepts_existing_absolute_path() {
        let this_file = std::env::current_exe().unwrap();
        assert_eq!(resolve_bin(&this_file), Some(this_file));
    }
}
