// HalluScribe - shared llama-server runtime helpers.
// Single home for the subprocess plumbing the gemma, briefing, and embedding
// runtimes all need: free-port scanning, binary resolution, the Windows
// no-window guard, and readiness polling. Extracted so a fix lands once
// instead of drifting across three near-identical copies (audit Q-1).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
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

/// True when `bin` is llama.cpp's unified CLI dispatcher rather than a
/// single-purpose binary. Builds from b10237 ship `llama`/`llama.exe`, which
/// does no work itself: it reads a subcommand (`serve`, `cli`, `download`, ...)
/// as its first argument and rejects anything starting with `-`. Pointing the
/// settings at it otherwise fails with the opaque `unknown command '-m'`.
pub fn is_dispatcher(bin: &Path) -> bool {
    bin.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("llama"))
}

/// Prefix the `serve` subcommand when `bin` is the dispatcher, so one flag list
/// works for both layouts. Must be called before any other argument is pushed —
/// the subcommand has to come first.
pub fn apply_serve_subcommand(command: &mut Command, bin: &Path) {
    if is_dispatcher(bin) {
        command.arg("serve");
    }
}

/// Largest the log may grow before the next spawn rotates it. One failed load
/// writes a few hundred lines, so this holds many sessions of history while
/// staying small enough to attach to a bug report.
const LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// Where release builds write llama-server's stderr. Set once at startup from
/// the Tauri path API — never derived here, so no OS path is hardcoded. Unset
/// means "nowhere known yet", and output is discarded rather than guessed at.
fn log_dir() -> &'static OnceLock<PathBuf> {
    static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();
    &LOG_DIR
}

/// Point release-build llama-server logging at `<archive_dir>/logs`. Called
/// once during setup; later calls are ignored.
pub fn set_log_dir(dir: PathBuf) {
    if log_dir().set(dir).is_err() {
        eprintln!("[llama] log directory was already configured; keeping the original location");
    }
}

/// Record a bounded, human-readable operational diagnostic beside llama-server
/// stderr logs. When even that directory is unavailable, preserve the message
/// on stderr rather than silently discarding it.
pub fn record_diagnostic(component: &str, message: &str) {
    let line = format!("[{}] {}\n", component, message);
    let Some(dir) = log_dir().get() else {
        eprintln!("{line}");
        return;
    };
    let outcome = (|| -> std::io::Result<()> {
        fs::create_dir_all(dir)?;
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("halluscribe.log"))?
            .write_all(line.as_bytes())
    })();
    if let Err(error) = outcome {
        eprintln!(
            "[{}] {} (also failed to write diagnostic: {})",
            component, message, error
        );
    }
}

/// Decide what happens to a spawned llama-server's stdout/stderr.
///
/// stdout is always discarded — it carries no diagnosis. stderr is where the
/// loader explains itself (CUDA OOM, unsupported flag, corrupt GGUF), and
/// HalluScribe's own `ExitedEarly` / `Timeout` verdict says that startup failed
/// but never why, so that stream is worth keeping:
///
/// - Debug builds inherit it, putting the detail straight in the `tauri dev`
///   terminal.
/// - Release builds append it to `<archive_dir>/logs/llama-server.log`. The app
///   is a windowless GUI process with no terminal to inherit, and the nightly
///   sweep fails while nobody is watching — a file is the only way that
///   explanation survives to be read in the morning.
///
/// Falls back to discarding when the log cannot be opened. An unread *pipe*
/// would eventually block the child, so the one thing never done here is leave
/// the stream buffered with no reader.
pub fn apply_output_capture(command: &mut Command) {
    command.stdout(Stdio::null());
    #[cfg(debug_assertions)]
    command.stderr(Stdio::inherit());
    #[cfg(not(debug_assertions))]
    command.stderr(match open_log() {
        Ok(stderr) => stderr,
        Err(error) => {
            record_diagnostic(
                "llama",
                &format!("could not open llama-server stderr log: {error}"),
            );
            Stdio::null()
        }
    });
}

/// Open the log for appending, rotating first if it has grown past the cap.
/// Only the release path calls this.
#[cfg_attr(debug_assertions, allow(dead_code))]
fn open_log() -> Result<Stdio, String> {
    let dir = log_dir()
        .get()
        .ok_or_else(|| "log directory is not configured".to_string())?;
    fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let path = dir.join("llama-server.log");
    // Keep exactly one previous generation: enough to survive a rotation
    // mid-investigation, bounded at 2x LOG_MAX_BYTES on disk.
    if fs::metadata(&path).is_ok_and(|meta| meta.len() > LOG_MAX_BYTES) {
        if let Err(error) = fs::rename(&path, dir.join("llama-server.log.1")) {
            record_diagnostic(
                "llama",
                &format!("could not rotate {}: {error}", path.display()),
            );
        }
    }
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map(Stdio::from)
        .map_err(|error| error.to_string())
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
    fn is_dispatcher_detects_the_unified_cli() {
        assert!(is_dispatcher(Path::new("llama.exe")));
        assert!(is_dispatcher(Path::new("/usr/local/bin/llama")));
        // Detection stays case-insensitive regardless of the host platform.
        assert!(is_dispatcher(Path::new("LLAMA.EXE")));
    }

    #[test]
    fn is_dispatcher_rejects_single_purpose_binaries() {
        assert!(!is_dispatcher(Path::new("llama-server.exe")));
        assert!(!is_dispatcher(Path::new("/usr/local/bin/llama-server")));
        assert!(!is_dispatcher(Path::new("llama-cli.exe")));
    }

    #[test]
    fn apply_serve_subcommand_prefixes_only_the_dispatcher() {
        let mut dispatcher = Command::new("llama");
        apply_serve_subcommand(&mut dispatcher, Path::new("llama.exe"));
        let args: Vec<_> = dispatcher.get_args().collect();
        assert_eq!(args, ["serve"]);

        let mut server = Command::new("llama-server");
        apply_serve_subcommand(&mut server, Path::new("llama-server.exe"));
        assert_eq!(server.get_args().count(), 0);
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
