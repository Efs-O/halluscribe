// HalluScribe - read-only MCP (Model Context Protocol) server surface.
// Exposes the existing archive search/profile read capabilities over stdio
// JSON-RPC so any MCP client (Claude Code, Codex, etc.) can query a user's
// HalluScribe archive. No Tauri runtime, no write/redact/delete tools -
// strictly read-only by design. See docs/internal/PERSONA_PROTOCOL_PLAN.md
// Phase 3.

mod server;

pub use server::HalluscribeServer;

use std::path::{Path, PathBuf};

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

/// Human-readable identity of the archive this server is bound to: the
/// resolved directory plus WHOSE archive it is according to the workspace
/// registry in the default root. Registrations are long-lived and invisible,
/// so a mixed-up per-person `HALLUSCRIBE_DIR` would otherwise only surface as
/// a subtly wrong portrait; this label makes it visible in the first reply.
pub fn archive_identity(archive_dir: &Path) -> String {
    let default_root = home_dir().ok().map(crate::briefing::archive_dir_path);
    let registry = default_root
        .as_deref()
        .map(crate::workspace::load_registry)
        .unwrap_or_default();
    identity_label(archive_dir, default_root.as_deref(), &registry)
}

/// Pure mapping behind `archive_identity`: "path - owner", where owner is the
/// registry's default-root label, a registered workspace's name, or an
/// explicit "unregistered" marker when the path matches neither.
pub fn identity_label(
    archive_dir: &Path,
    default_root: Option<&Path>,
    registry: &crate::workspace::WorkspaceRegistry,
) -> String {
    let owner = if default_root.is_some_and(|root| same_dir(archive_dir, root)) {
        match &registry.default_name {
            Some(name) => format!("\"{name}\" (host default)"),
            None => "host default".to_string(),
        }
    } else if let Some(ws) = registry
        .workspaces
        .iter()
        .find(|ws| same_dir(&ws.path, archive_dir))
    {
        if ws.import_only {
            format!("workspace \"{}\" (import-only guest)", ws.name)
        } else {
            format!("workspace \"{}\"", ws.name)
        }
    } else {
        "unregistered archive (not in workspaces.json)".to_string()
    };
    format!("{} - {owner}", archive_dir.display())
}

/// Path equality tolerant of case/slash/relative differences: literal match
/// first, then canonicalized when both paths exist on disk.
fn same_dir(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(canon_a), Ok(canon_b)) => canon_a == canon_b,
        _ => false,
    }
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
