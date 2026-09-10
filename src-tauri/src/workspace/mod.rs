// HalluScribe - multi-person workspace registry. A single JSON file that always
// lives in the DEFAULT archive root (<home>/.halluscribe) records the list of
// workspaces and which one is active, so archive_dir can resolve to the active
// workspace root while host-global state (registry, llama-server PID marker)
// stays anchored at the default root. See docs/internal/PERSONAL_PARITY_PLAN.md §E1.

mod layout;
mod relocate;

pub use layout::suggested_workspace_path;
pub use relocate::{copy_archive, set_workspace_path, CopyStats};

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// One person's isolated archive root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub path: PathBuf,
    /// Guest/import-only: the sweep skips the host's local coding-tool scan and
    /// ingests only this workspace's configured chat imports. (Enforced in E3.)
    #[serde(default)]
    pub import_only: bool,
}

/// The registry file contents. `active` is an absolute workspace path; `None`
/// (or a path no longer on disk) means "use the default root".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceRegistry {
    #[serde(default)]
    pub active: Option<PathBuf>,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    /// Display label for the default (host) root. Purely cosmetic — the default
    /// always resolves by path (`<home>/.halluscribe`), so renaming it never
    /// touches the archive and never triggers a rebuild. `None` => "Default".
    #[serde(default)]
    pub default_name: Option<String>,
}

/// `<default_root>/workspaces.json` — always in the default root.
pub fn registry_path(default_root: &Path) -> PathBuf {
    default_root.join("workspaces.json")
}

/// Load the registry from the default root. A missing file is a valid empty
/// registry; an existing unreadable or malformed file is an explicit error so
/// workspace mutations can never replace corruption with a new empty registry.
pub fn load_registry(default_root: &Path) -> Result<WorkspaceRegistry, String> {
    let path = registry_path(default_root);
    if !path.exists() {
        return Ok(WorkspaceRegistry::default());
    }
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

/// Persist the registry to the default root, creating the directory if needed.
pub fn save_registry(default_root: &Path, registry: &WorkspaceRegistry) -> Result<(), String> {
    // Keep this safe even for callers that do not load the registry first.
    // A corrupted registry must be recovered deliberately, never overwritten
    // by an apparently empty one.
    if registry_path(default_root).exists() {
        let _ = load_registry(default_root)?;
    }
    fs::create_dir_all(default_root).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(registry).map_err(|error| error.to_string())?;
    crate::atomic_file::write_atomic(&registry_path(default_root), json)
        .map_err(|error| error.to_string())
}

/// Resolve the active archive root. Falls back to `default_root` when there is no
/// registry, no active pointer, or the active path no longer exists on disk
/// (e.g. an external drive is unplugged) — this guarantees full back-compat.
pub fn resolve_active_dir(default_root: &Path) -> Result<PathBuf, String> {
    Ok(match load_registry(default_root)?.active {
        Some(active) if active.exists() => active,
        _ => default_root.to_path_buf(),
    })
}

/// Register a new workspace. Rejects a duplicate `path` so two entries can never
/// point at the same archive root.
pub fn add_workspace(reg: &mut WorkspaceRegistry, ws: Workspace) -> Result<(), String> {
    if reg
        .workspaces
        .iter()
        .any(|existing| existing.path == ws.path)
    {
        return Err("a workspace is already registered at that path".to_string());
    }
    reg.workspaces.push(ws);
    Ok(())
}

/// Set the active workspace. `None` means "use the default root" and is always
/// valid; `Some(path)` must match a registered workspace.
pub fn set_active(reg: &mut WorkspaceRegistry, active: Option<PathBuf>) -> Result<(), String> {
    if let Some(ref path) = active {
        if !reg.workspaces.iter().any(|ws| &ws.path == path) {
            return Err("no workspace registered at that path".to_string());
        }
    }
    reg.active = active;
    Ok(())
}

/// Remove a registered workspace, found by `path`. If it was the active one, the
/// active pointer resets to `None` (the default root) so the caller never ends up
/// pointing at a workspace that no longer exists in the registry. Only the
/// registry entry is dropped here — the archive folder on disk is left untouched.
pub fn remove_workspace(reg: &mut WorkspaceRegistry, path: &Path) -> Result<(), String> {
    let before = reg.workspaces.len();
    reg.workspaces.retain(|ws| ws.path != path);
    if reg.workspaces.len() == before {
        return Err("no workspace registered at that path".to_string());
    }
    if reg.active.as_deref() == Some(path) {
        reg.active = None;
    }
    Ok(())
}

/// Set (or clear, with an empty string) the default root's display label.
pub fn set_default_name(reg: &mut WorkspaceRegistry, name: &str) {
    let trimmed = name.trim();
    reg.default_name = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    };
}

/// Rename a registered workspace, found by `path`.
pub fn rename(reg: &mut WorkspaceRegistry, path: &Path, name: &str) -> Result<(), String> {
    let ws = reg
        .workspaces
        .iter_mut()
        .find(|ws| ws.path == path)
        .ok_or_else(|| "no workspace registered at that path".to_string())?;
    ws.name = name.to_string();
    Ok(())
}

/// True when the registered workspace at `active_dir` is import-only. The default
/// root (or an unregistered path) is never import-only.
pub fn is_import_only(reg: &WorkspaceRegistry, active_dir: &Path) -> bool {
    reg.workspaces
        .iter()
        .find(|ws| ws.path == active_dir)
        .map(|ws| ws.import_only)
        .unwrap_or(false)
}

/// Load the registry from `default_root` and report whether `active_dir` is an
/// import-only workspace.
pub fn is_active_import_only(default_root: &Path, active_dir: &Path) -> Result<bool, String> {
    Ok(is_import_only(&load_registry(default_root)?, active_dir))
}

/// Toggle a registered workspace's `import_only` flag, found by `path`.
pub fn set_import_only(
    reg: &mut WorkspaceRegistry,
    path: &Path,
    value: bool,
) -> Result<(), String> {
    let ws = reg
        .workspaces
        .iter_mut()
        .find(|ws| ws.path == path)
        .ok_or_else(|| "no workspace registered at that path".to_string())?;
    ws.import_only = value;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "halluscribe_ws_test_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_falls_back_when_no_registry() {
        let root = tmp_dir("no_registry");
        assert_eq!(resolve_active_dir(&root).unwrap(), root);
    }

    #[test]
    fn resolve_returns_active_existing_dir() {
        let root = tmp_dir("active_exists");
        let active = root.join("person_a");
        std::fs::create_dir_all(&active).unwrap();
        let registry = WorkspaceRegistry {
            active: Some(active.clone()),
            workspaces: vec![],
            ..Default::default()
        };
        save_registry(&root, &registry).unwrap();
        assert_eq!(resolve_active_dir(&root).unwrap(), active);
    }

    #[test]
    fn resolve_falls_back_when_active_missing() {
        let root = tmp_dir("active_missing");
        let missing = root.join("gone");
        let registry = WorkspaceRegistry {
            active: Some(missing),
            workspaces: vec![],
            ..Default::default()
        };
        save_registry(&root, &registry).unwrap();
        assert_eq!(resolve_active_dir(&root).unwrap(), root);
    }

    #[test]
    fn save_load_round_trips() {
        let root = tmp_dir("round_trip");
        let active = root.join("person_b");
        let registry = WorkspaceRegistry {
            active: Some(active.clone()),
            workspaces: vec![Workspace {
                name: "Person B".to_string(),
                path: active.clone(),
                import_only: true,
            }],
            ..Default::default()
        };
        save_registry(&root, &registry).unwrap();
        let loaded = load_registry(&root).unwrap();
        assert_eq!(loaded.active, Some(active));
        assert_eq!(loaded.workspaces, registry.workspaces);
    }

    #[test]
    fn malformed_registry_returns_an_error_without_falling_back() {
        let root = tmp_dir("malformed");
        std::fs::write(registry_path(&root), b"{ not json").unwrap();
        let error = load_registry(&root).unwrap_err();
        assert!(error.contains("failed to parse"));
        assert!(resolve_active_dir(&root).is_err());
    }

    #[test]
    fn save_refuses_to_replace_malformed_registry() {
        let root = tmp_dir("malformed-save");
        let path = registry_path(&root);
        std::fs::write(&path, b"{ not json").unwrap();

        assert!(save_registry(&root, &WorkspaceRegistry::default()).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "{ not json");
    }

    fn sample_ws(name: &str, path: &str) -> Workspace {
        Workspace {
            name: name.to_string(),
            path: PathBuf::from(path),
            import_only: false,
        }
    }

    #[test]
    fn add_workspace_pushes_and_rejects_duplicate_path() {
        let mut reg = WorkspaceRegistry::default();
        add_workspace(&mut reg, sample_ws("A", "/ws/a")).unwrap();
        assert_eq!(reg.workspaces.len(), 1);

        let err = add_workspace(&mut reg, sample_ws("A2", "/ws/a")).unwrap_err();
        assert_eq!(err, "a workspace is already registered at that path");
        assert_eq!(reg.workspaces.len(), 1);
    }

    #[test]
    fn set_active_none_is_always_ok() {
        let mut reg = WorkspaceRegistry::default();
        set_active(&mut reg, None).unwrap();
        assert!(reg.active.is_none());
    }

    #[test]
    fn set_active_known_path_ok_unknown_rejected() {
        let mut reg = WorkspaceRegistry::default();
        add_workspace(&mut reg, sample_ws("A", "/ws/a")).unwrap();

        set_active(&mut reg, Some(PathBuf::from("/ws/a"))).unwrap();
        assert_eq!(reg.active, Some(PathBuf::from("/ws/a")));

        let err = set_active(&mut reg, Some(PathBuf::from("/ws/missing"))).unwrap_err();
        assert_eq!(err, "no workspace registered at that path");
        // active pointer unchanged on failure
        assert_eq!(reg.active, Some(PathBuf::from("/ws/a")));
    }

    #[test]
    fn rename_updates_and_errors_when_absent() {
        let mut reg = WorkspaceRegistry::default();
        add_workspace(&mut reg, sample_ws("A", "/ws/a")).unwrap();

        rename(&mut reg, Path::new("/ws/a"), "Renamed").unwrap();
        assert_eq!(reg.workspaces[0].name, "Renamed");

        let err = rename(&mut reg, Path::new("/ws/missing"), "X").unwrap_err();
        assert_eq!(err, "no workspace registered at that path");
    }

    #[test]
    fn is_import_only_true_for_matching_guest_workspace() {
        let mut reg = WorkspaceRegistry::default();
        add_workspace(&mut reg, sample_ws("A", "/ws/a")).unwrap();
        set_import_only(&mut reg, Path::new("/ws/a"), true).unwrap();
        assert!(is_import_only(&reg, Path::new("/ws/a")));
    }

    #[test]
    fn is_import_only_false_for_non_guest_workspace() {
        let mut reg = WorkspaceRegistry::default();
        add_workspace(&mut reg, sample_ws("A", "/ws/a")).unwrap();
        assert!(!is_import_only(&reg, Path::new("/ws/a")));
    }

    #[test]
    fn is_import_only_false_for_unregistered_path() {
        let mut reg = WorkspaceRegistry::default();
        add_workspace(&mut reg, sample_ws("A", "/ws/a")).unwrap();
        set_import_only(&mut reg, Path::new("/ws/a"), true).unwrap();
        // The default root (unregistered) is never import-only.
        assert!(!is_import_only(&reg, Path::new("/default/root")));
    }

    #[test]
    fn remove_workspace_drops_entry_and_errors_when_absent() {
        let mut reg = WorkspaceRegistry::default();
        add_workspace(&mut reg, sample_ws("A", "/ws/a")).unwrap();
        add_workspace(&mut reg, sample_ws("B", "/ws/b")).unwrap();

        remove_workspace(&mut reg, Path::new("/ws/a")).unwrap();
        assert_eq!(reg.workspaces.len(), 1);
        assert_eq!(reg.workspaces[0].path, PathBuf::from("/ws/b"));

        let err = remove_workspace(&mut reg, Path::new("/ws/missing")).unwrap_err();
        assert_eq!(err, "no workspace registered at that path");
    }

    #[test]
    fn remove_workspace_resets_active_when_removing_active() {
        let mut reg = WorkspaceRegistry::default();
        add_workspace(&mut reg, sample_ws("A", "/ws/a")).unwrap();
        set_active(&mut reg, Some(PathBuf::from("/ws/a"))).unwrap();

        remove_workspace(&mut reg, Path::new("/ws/a")).unwrap();
        assert!(reg.active.is_none());
    }

    #[test]
    fn set_default_name_sets_and_clears() {
        let mut reg = WorkspaceRegistry::default();
        assert!(reg.default_name.is_none());

        set_default_name(&mut reg, "  EFSO  ");
        assert_eq!(reg.default_name.as_deref(), Some("EFSO"));

        set_default_name(&mut reg, "   ");
        assert!(reg.default_name.is_none());
    }

    #[test]
    fn set_import_only_toggles_and_errors_when_absent() {
        let mut reg = WorkspaceRegistry::default();
        add_workspace(&mut reg, sample_ws("A", "/ws/a")).unwrap();
        assert!(!reg.workspaces[0].import_only);

        set_import_only(&mut reg, Path::new("/ws/a"), true).unwrap();
        assert!(reg.workspaces[0].import_only);

        let err = set_import_only(&mut reg, Path::new("/ws/missing"), true).unwrap_err();
        assert_eq!(err, "no workspace registered at that path");
    }
}
