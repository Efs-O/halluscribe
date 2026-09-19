// HalluScribe - user settings.
// Loads from and saves to ~/.halluscribe/settings.json.
// Missing or partial files are always valid - every field has a serde default.
// Phase 7 resolves the archive_dir path via Tauri's path API and passes it here.

mod host;
mod imports;
mod paths;
mod persistence;
mod runtime;
mod tests;
mod types;

pub use host::{owning_dir, owning_dir as host_settings_dir, set_host_root};
pub use imports::{ensure_import_paths, load_with_import_paths};
pub use paths::validate_paths;
pub use persistence::{load_settings, save_settings};
pub(crate) use types::parse_schedule_time;
pub use types::{BackendKind, HalluScribeSettings, SettingsError};
