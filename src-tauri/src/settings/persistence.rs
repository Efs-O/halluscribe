use super::{HalluScribeSettings, SettingsError};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Load settings from `<archive_dir>/settings.json`.
/// Returns defaults if the file is absent, empty, or malformed.
pub fn load_settings(archive_dir: &Path) -> HalluScribeSettings {
    let path = settings_path(archive_dir);
    let Ok(content) = fs::read_to_string(&path) else {
        return HalluScribeSettings::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

/// Persist settings to `<archive_dir>/settings.json`.
/// Creates the directory if it does not exist.
pub fn save_settings(
    archive_dir: &Path,
    settings: &HalluScribeSettings,
) -> Result<(), SettingsError> {
    fs::create_dir_all(archive_dir)?;
    let json = serde_json::to_string_pretty(settings)?;
    fs::write(settings_path(archive_dir), json)?;
    Ok(())
}

fn settings_path(archive_dir: &Path) -> PathBuf {
    archive_dir.join("settings.json")
}
