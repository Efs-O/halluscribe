// HalluScribe - Tauri command wrappers for multi-person workspaces.
// Thin adapters over the pure `workspace` registry helpers: they resolve the
// registry from the DEFAULT archive root, mutate it, and persist. All business
// logic lives in `workspace` and `settings` so these commands stay untestable-thin.

use crate::app_support::default_archive_dir;
use crate::workspace::Workspace;
use crate::{settings, workspace};
use std::path::{Path, PathBuf};

/// Snapshot of the workspace registry for the frontend switcher.
#[derive(serde::Serialize)]
pub struct WorkspaceListDto {
    /// Absolute path of the default archive root (`<home>/.halluscribe`).
    pub default_root: String,
    /// Display label for the default root, or `None` when unset (UI shows "Default").
    pub default_name: Option<String>,
    /// Active workspace path, or `None` when the default root is active.
    pub active: Option<String>,
    pub workspaces: Vec<Workspace>,
}

/// List all registered workspaces plus the active pointer and default root.
#[tauri::command]
pub(crate) fn list_workspaces(app: tauri::AppHandle) -> Result<WorkspaceListDto, String> {
    let default_root = default_archive_dir(&app)?;
    let reg = workspace::load_registry(&default_root);
    Ok(WorkspaceListDto {
        default_root: default_root.to_string_lossy().into_owned(),
        default_name: reg.default_name,
        active: reg.active.map(|p| p.to_string_lossy().into_owned()),
        workspaces: reg.workspaces,
    })
}

/// Register (but do NOT switch to) a new workspace. Creates its folder, seeds a
/// fresh settings.json from the host's, and appends it to the registry.
#[tauri::command]
pub(crate) fn create_workspace(
    app: tauri::AppHandle,
    name: String,
    path: String,
    import_only: bool,
) -> Result<Workspace, String> {
    let name = name.trim().to_string();
    let path = path.trim().to_string();
    if name.is_empty() {
        return Err("workspace name is required".to_string());
    }
    if path.is_empty() {
        return Err("workspace folder is required".to_string());
    }

    let ws_path = PathBuf::from(path.trim());
    if !ws_path.is_absolute() {
        return Err("workspace folder must be an absolute path".to_string());
    }

    let default_root = default_archive_dir(&app)?;
    if ws_path == default_root {
        return Err("cannot register the default archive root as a workspace".to_string());
    }

    std::fs::create_dir_all(&ws_path).map_err(|e| e.to_string())?;

    let host = settings::load_settings(&default_root);
    let seeded = host.seed_workspace_settings();
    settings::save_settings(&ws_path, &seeded).map_err(|e| e.to_string())?;

    let mut reg = workspace::load_registry(&default_root);
    let ws = Workspace {
        name,
        path: ws_path,
        import_only,
    };
    workspace::add_workspace(&mut reg, ws.clone())?;
    workspace::save_registry(&default_root, &reg)?;
    Ok(ws)
}

/// Switch the active workspace. `None` selects the default root. A `Some(path)`
/// must be registered and still present on disk.
#[tauri::command]
pub(crate) fn switch_workspace(app: tauri::AppHandle, path: Option<String>) -> Result<(), String> {
    let default_root = default_archive_dir(&app)?;
    let mut reg = workspace::load_registry(&default_root);
    let target = path.map(|p| PathBuf::from(p.trim()));
    if let Some(ref p) = target {
        if !p.exists() {
            return Err("workspace folder no longer exists on disk".to_string());
        }
    }
    workspace::set_active(&mut reg, target)?;
    workspace::save_registry(&default_root, &reg)?;
    Ok(())
}

/// Rename a registered workspace.
#[tauri::command]
pub(crate) fn rename_workspace(
    app: tauri::AppHandle,
    path: String,
    name: String,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("workspace name is required".to_string());
    }
    let default_root = default_archive_dir(&app)?;
    let mut reg = workspace::load_registry(&default_root);
    workspace::rename(&mut reg, Path::new(path.trim()), name)?;
    workspace::save_registry(&default_root, &reg)?;
    Ok(())
}

/// Rename the default (host) root. This is a cosmetic label only — the default
/// always resolves by path, so this never touches the archive or forces a rebuild.
#[tauri::command]
pub(crate) fn rename_default_workspace(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("workspace name is required".to_string());
    }
    let default_root = default_archive_dir(&app)?;
    let mut reg = workspace::load_registry(&default_root);
    workspace::set_default_name(&mut reg, name);
    workspace::save_registry(&default_root, &reg)?;
    Ok(())
}

/// Un-register a workspace (guest only). Drops the registry entry and, if it was
/// active, falls back to the default root. The archive folder on disk is NOT
/// deleted — the person's data is preserved and the folder can be re-added later.
#[tauri::command]
pub(crate) fn delete_workspace(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let default_root = default_archive_dir(&app)?;
    let mut reg = workspace::load_registry(&default_root);
    workspace::remove_workspace(&mut reg, Path::new(path.trim()))?;
    workspace::save_registry(&default_root, &reg)?;
    Ok(())
}

/// Toggle a registered workspace's import-only (guest) flag.
#[tauri::command]
pub(crate) fn set_workspace_import_only(
    app: tauri::AppHandle,
    path: String,
    import_only: bool,
) -> Result<(), String> {
    let default_root = default_archive_dir(&app)?;
    let mut reg = workspace::load_registry(&default_root);
    workspace::set_import_only(&mut reg, Path::new(path.trim()), import_only)?;
    workspace::save_registry(&default_root, &reg)?;
    Ok(())
}
