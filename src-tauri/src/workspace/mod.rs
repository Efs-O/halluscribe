// HalluScribe - multi-person workspace registry. A single JSON file that always
// lives in the DEFAULT archive root (<home>/.halluscribe) records the list of
// workspaces and which one is active, so archive_dir can resolve to the active
// workspace root while host-global state (registry, llama-server PID marker)
// stays anchored at the default root. See docs/internal/PERSONAL_PARITY_PLAN.md §E1.

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
}

/// `<default_root>/workspaces.json` — always in the default root.
pub fn registry_path(default_root: &Path) -> PathBuf {
    default_root.join("workspaces.json")
}

/// Load the registry from the default root. Absent, empty, or malformed file =>
/// an empty registry (active None) so the app always falls back to the default.
pub fn load_registry(default_root: &Path) -> WorkspaceRegistry {
    let path = registry_path(default_root);
    let Ok(content) = fs::read_to_string(&path) else {
        return WorkspaceRegistry::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

/// Persist the registry to the default root, creating the directory if needed.
pub fn save_registry(default_root: &Path, registry: &WorkspaceRegistry) -> Result<(), String> {
    fs::create_dir_all(default_root).map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(registry).map_err(|error| error.to_string())?;
    fs::write(registry_path(default_root), json).map_err(|error| error.to_string())
}

/// Resolve the active archive root. Falls back to `default_root` when there is no
/// registry, no active pointer, or the active path no longer exists on disk
/// (e.g. an external drive is unplugged) — this guarantees full back-compat.
pub fn resolve_active_dir(default_root: &Path) -> PathBuf {
    match load_registry(default_root).active {
        Some(active) if active.exists() => active,
        _ => default_root.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("halluscribe_ws_test_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_falls_back_when_no_registry() {
        let root = tmp_dir("no_registry");
        assert_eq!(resolve_active_dir(&root), root);
    }

    #[test]
    fn resolve_returns_active_existing_dir() {
        let root = tmp_dir("active_exists");
        let active = root.join("person_a");
        std::fs::create_dir_all(&active).unwrap();
        let registry = WorkspaceRegistry {
            active: Some(active.clone()),
            workspaces: vec![],
        };
        save_registry(&root, &registry).unwrap();
        assert_eq!(resolve_active_dir(&root), active);
    }

    #[test]
    fn resolve_falls_back_when_active_missing() {
        let root = tmp_dir("active_missing");
        let missing = root.join("gone");
        let registry = WorkspaceRegistry {
            active: Some(missing),
            workspaces: vec![],
        };
        save_registry(&root, &registry).unwrap();
        assert_eq!(resolve_active_dir(&root), root);
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
        };
        save_registry(&root, &registry).unwrap();
        let loaded = load_registry(&root);
        assert_eq!(loaded.active, Some(active));
        assert_eq!(loaded.workspaces, registry.workspaces);
    }

    #[test]
    fn malformed_registry_falls_back_to_default() {
        let root = tmp_dir("malformed");
        std::fs::write(registry_path(&root), b"{ not json").unwrap();
        let loaded = load_registry(&root);
        assert!(loaded.active.is_none());
        assert!(loaded.workspaces.is_empty());
        assert_eq!(resolve_active_dir(&root), root);
    }
}
