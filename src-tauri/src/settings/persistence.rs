// HalluScribe - settings persistence with explicit corruption handling.

use super::{host, HalluScribeSettings, SettingsError};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Load the settings governing `archive_dir`.
///
/// For the host this is simply its own file. For a guest workspace the machine
/// fields are read THROUGH to the host's file (see `host::resolve`), so one
/// model change on the host moves every workspace and no stale copy survives.
/// This is the single chokepoint every caller in the app already goes through.
pub fn load_settings(archive_dir: &Path) -> Result<HalluScribeSettings, SettingsError> {
    host::resolve(archive_dir, load_own(archive_dir)?)
}

/// Read `<archive_dir>/settings.json` exactly as written, with no host overlay.
/// A missing file is a fresh installation and therefore returns defaults.
/// An existing unreadable or malformed file is an explicit error so callers
/// never mistake corruption for a user choosing default settings.
pub(super) fn load_own(archive_dir: &Path) -> Result<HalluScribeSettings, SettingsError> {
    let path = settings_path(archive_dir);
    if !path.exists() {
        return Ok(HalluScribeSettings::default());
    }
    let content = fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&content)?)
}

/// Persist settings to `<archive_dir>/settings.json`.
/// Creates the directory if it does not exist.
pub fn save_settings(
    archive_dir: &Path,
    settings: &HalluScribeSettings,
) -> Result<(), SettingsError> {
    // Reject a direct save over an existing unreadable settings file too. Most
    // callers load first, but this makes the persistence boundary safe on its
    // own and preserves the file for an explicit recovery action.
    if settings_path(archive_dir).exists() {
        let _ = load_own(archive_dir)?;
    }
    fs::create_dir_all(archive_dir)?;
    // A guest's file keeps only that person's own fields. The machine fields it
    // was seeded with are dropped here rather than rewritten, so the host stays
    // the only copy and this doubles as the Phase D migration: the stale values
    // disappear the first time each workspace saves.
    let stripped = host::strip_host_owned(archive_dir, settings);
    let to_write = stripped.as_ref().unwrap_or(settings);
    let json = serde_json::to_string_pretty(to_write)?;
    crate::atomic_file::write_atomic(&settings_path(archive_dir), json)?;
    Ok(())
}

fn settings_path(archive_dir: &Path) -> PathBuf {
    archive_dir.join("settings.json")
}
