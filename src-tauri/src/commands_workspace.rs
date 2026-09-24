// HalluScribe - Tauri command wrappers for multi-person workspaces.
// Thin adapters over the pure `workspace` registry helpers: they resolve the
// registry from the DEFAULT archive root, mutate it, and persist. All business
// logic lives in `workspace` and `settings` so these commands stay untestable-thin.

use crate::app_state::{
    BriefingCancel, CancelState, CaptureCancel, ChatCancel, ProfileCancel, SweepCancel,
};
use crate::app_support::default_archive_dir;
use crate::workspace::Workspace;
use crate::{settings, workspace};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::Manager;

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
    let reg = workspace::load_registry(&default_root)?;
    Ok(WorkspaceListDto {
        default_root: default_root.to_string_lossy().into_owned(),
        default_name: reg.default_name,
        active: reg.active.map(|p| p.to_string_lossy().into_owned()),
        workspaces: reg.workspaces,
    })
}

/// Suggested archive folder for a workspace named `name`:
/// `<default_root>/users/<slug>`. The UI pre-fills the folder picker with this
/// so every person lands in the same hierarchy; the user may still pick another
/// absolute path, which `create_workspace` accepts unchanged.
#[tauri::command]
pub(crate) fn suggest_workspace_path(
    app: tauri::AppHandle,
    name: String,
) -> Result<String, String> {
    let default_root = default_archive_dir(&app)?;
    Ok(workspace::suggested_workspace_path(&default_root, &name)
        .to_string_lossy()
        .into_owned())
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

    let host = settings::load_settings(&default_root).map_err(|error| error.to_string())?;
    let mut seeded = host.seed_workspace_settings();
    // `seed_workspace_settings` blanks the import paths so a guest can never
    // inherit the host's folders; deriving them here - where the new archive
    // root is finally known - gives the guest its own `imports/…` instead.
    settings::ensure_import_paths(&ws_path, &mut seeded);
    settings::save_settings(&ws_path, &seeded).map_err(|e| e.to_string())?;

    let mut reg = workspace::load_registry(&default_root)?;
    let ws = Workspace {
        name,
        path: ws_path,
        import_only,
    };
    workspace::add_workspace(&mut reg, ws.clone())?;
    workspace::save_registry(&default_root, &reg)?;
    Ok(ws)
}

/// What a completed move copied, plus where the old archive still is.
#[derive(serde::Serialize)]
pub struct MoveWorkspaceResult {
    pub files: usize,
    pub bytes: u64,
    /// The previous folder, left fully intact for the user to check and then
    /// send to the Recycle Bin themselves.
    pub old_path: String,
    pub new_path: String,
}

/// Move a registered workspace's archive to `new_path`.
///
/// The archive is COPIED and its file count and total size are checked against
/// the source before the registry is repointed; the old folder is never
/// deleted, so a failed or half-trusted move always leaves an intact archive
/// behind. No background job may run during the move. Settings, index and raws travel with
/// the folder untouched - unlike un-register + re-register, which would seed a
/// fresh settings.json over them.
#[tauri::command]
pub(crate) async fn move_workspace(
    app: tauri::AppHandle,
    path: String,
    new_path: String,
) -> Result<MoveWorkspaceResult, String> {
    // Copying a large archive takes a while; off the UI thread it cannot
    // freeze the window.
    tauri::async_runtime::spawn_blocking(move || move_workspace_blocking(&app, &path, &new_path))
        .await
        .map_err(|error| format!("the workspace move stopped unexpectedly: {error}"))?
}

fn move_workspace_blocking(
    app: &tauri::AppHandle,
    path: &str,
    new_path: &str,
) -> Result<MoveWorkspaceResult, String> {
    let from = PathBuf::from(path.trim());
    let to = PathBuf::from(new_path.trim());
    if !to.is_absolute() {
        return Err("the new folder must be an absolute path".to_string());
    }

    let default_root = default_archive_dir(app)?;
    if to == default_root {
        return Err("cannot move a workspace onto the default archive root".to_string());
    }

    let mut reg = workspace::load_registry(&default_root)?;
    if !reg.workspaces.iter().any(|ws| ws.path == from) {
        return Err("no workspace registered at that path".to_string());
    }

    // A job writing into the archive mid-copy would make the copy miss its
    // writes (or fail verification), so no job may run until the move is done.
    let _held = hold_all_jobs(app)?;
    let stats = workspace::copy_archive(&from, &to)?;
    workspace::set_workspace_path(&mut reg, &from, to.clone())?;
    workspace::save_registry(&default_root, &reg)?;

    Ok(MoveWorkspaceResult {
        files: stats.files,
        bytes: stats.bytes,
        old_path: from.to_string_lossy().into_owned(),
        new_path: to.to_string_lossy().into_owned(),
    })
}

/// Every background job's run slot, held so none can start; released on drop.
struct HeldJobs<'a>(Vec<(&'a CancelState, Arc<AtomicBool>)>);

impl Drop for HeldJobs<'_> {
    fn drop(&mut self) {
        for (slot, token) in &self.0 {
            slot.finish_run(token);
        }
    }
}

/// Claim the run slot of every job that writes into the archive, or say which
/// one is running. Slots claimed before a busy one are released on return.
fn hold_all_jobs(app: &tauri::AppHandle) -> Result<HeldJobs<'_>, String> {
    let slots: [(&str, &CancelState); 5] = [
        ("A sweep", &app.state::<SweepCancel>().inner().0),
        ("A profile refresh", &app.state::<ProfileCancel>().inner().0),
        ("A briefing", &app.state::<BriefingCancel>().inner().0),
        ("A chat reply", &app.state::<ChatCancel>().inner().0),
        (
            "The startup raw capture",
            &app.state::<CaptureCancel>().inner().0,
        ),
    ];
    let mut held = HeldJobs(Vec::new());
    for (job, slot) in slots {
        let token = slot.try_begin_run().ok_or_else(|| {
            format!("{job} is running. Let it finish or cancel it, then move the workspace.")
        })?;
        held.0.push((slot, token));
    }
    Ok(held)
}

/// Switch the active workspace. `None` selects the default root. A `Some(path)`
/// must be registered and still present on disk.
#[tauri::command]
pub(crate) fn switch_workspace(app: tauri::AppHandle, path: Option<String>) -> Result<(), String> {
    let default_root = default_archive_dir(&app)?;
    let mut reg = workspace::load_registry(&default_root)?;
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
    let mut reg = workspace::load_registry(&default_root)?;
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
    let mut reg = workspace::load_registry(&default_root)?;
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
    let mut reg = workspace::load_registry(&default_root)?;
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
    let mut reg = workspace::load_registry(&default_root)?;
    workspace::set_import_only(&mut reg, Path::new(path.trim()), import_only)?;
    workspace::save_registry(&default_root, &reg)?;
    Ok(())
}
