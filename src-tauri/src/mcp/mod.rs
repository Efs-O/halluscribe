// HalluScribe - read-only MCP (Model Context Protocol) server surface.
// Exposes the existing archive search/profile read capabilities over stdio
// JSON-RPC so any MCP client (Claude Code, Codex, etc.) can query a user's
// HalluScribe archive. No Tauri runtime, no write/redact/delete tools -
// strictly read-only by design. See docs/internal/PERSONA_PROTOCOL_PLAN.md
// Phase 3.

mod server;

pub use server::HalluscribeServer;

use std::path::PathBuf;

/// Resolve the archive directory the MCP server should read from.
///
/// `halluscribe_dir_override` is the `HALLUSCRIBE_DIR` environment variable,
/// read by the caller (the `halluscribe-mcp` binary's `main`) so this
/// function stays a pure, easily testable mapping rather than reaching into
/// process env itself. When set, it takes precedence over the home-directory
/// default and must already exist. Otherwise the home directory is resolved
/// from `USERPROFILE` (Windows) or `HOME` (elsewhere) and joined the same way
/// as the desktop app's archive dir (`briefing::archive_dir_path`).
pub fn resolve_archive_dir(halluscribe_dir_override: Option<String>) -> Result<PathBuf, String> {
    if let Some(dir) = halluscribe_dir_override {
        let path = PathBuf::from(dir);
        if !path.is_dir() {
            return Err(format!(
                "HALLUSCRIBE_DIR does not exist or is not a directory: {}",
                path.display()
            ));
        }
        return Ok(path);
    }

    let home = home_dir()?;
    let dir = crate::briefing::archive_dir_path(home);
    if !dir.is_dir() {
        return Err(format!(
            "archive directory does not exist: {} (run HalluScribe at least once to create it, or set HALLUSCRIBE_DIR to point at an existing archive)",
            dir.display()
        ));
    }
    Ok(dir)
}

#[cfg(windows)]
fn home_dir() -> Result<PathBuf, String> {
    std::env::var("USERPROFILE")
        .map(PathBuf::from)
        .map_err(|_| "could not resolve home directory: USERPROFILE is not set".to_string())
}

#[cfg(not(windows))]
fn home_dir() -> Result<PathBuf, String> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "could not resolve home directory: HOME is not set".to_string())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod mod_tests;
