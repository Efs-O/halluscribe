// HalluScribe - settings persistence with explicit corruption handling.

use super::{HalluScribeSettings, SettingsError};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Load settings from `<archive_dir>/settings.json`.
/// A missing file is a fresh installation and therefore returns defaults.
/// An existing unreadable or malformed file is an explicit error so callers
/// never mistake corruption for a user choosing default settings.
pub fn load_settings(archive_dir: &Path) -> Result<HalluScribeSettings, SettingsError> {
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
        let _ = load_settings(archive_dir)?;
    }
    fs::create_dir_all(archive_dir)?;
    let json = serde_json::to_string_pretty(settings)?;
    crate::atomic_file::write_atomic(&settings_path(archive_dir), json)?;
    Ok(())
}

fn settings_path(archive_dir: &Path) -> PathBuf {
    archive_dir.join("settings.json")
}
