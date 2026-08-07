// HalluScribe - default chat-import folders derived from an archive root.
//
// A fresh install used to ship three BLANK import paths, so every user invented
// their own layout and the folders later drifted away from what the archive had
// recorded. Import folders are now derived from the archive root that owns them:
// `<archive_root>/imports/{chatgpt,claude,gemini}`. Every workspace therefore
// gets its own folders - a guest can never inherit the host's - and the sweep
// and the raw backfill both resolve user-owned imports through the same
// settings values. See docs/internal/IMPORT_PATHS_PLAN.md Phase A1.

use super::HalluScribeSettings;
use std::fs;
use std::path::{Path, PathBuf};

/// Folder under an archive root holding that person's chat exports.
pub const IMPORTS_DIR: &str = "imports";

/// Where a Google Takeout export unpacks inside the Gemini import folder. The
/// scanner accepts either this nested path or a bare `My Activity.json`; the
/// nested one is created so a Takeout drop needs no thought.
const GEMINI_TAKEOUT_SUBPATH: [&str; 3] = ["Takeout", "My Activity", "Gemini Apps"];

/// The per-provider import folder for `archive_dir`.
pub fn default_import_dir(archive_dir: &Path, provider_leaf: &str) -> PathBuf {
    archive_dir.join(IMPORTS_DIR).join(provider_leaf)
}

/// Fill any BLANK import path in `settings` with the folder derived from
/// `archive_dir`, creating it on disk so Settings always shows a path that
/// exists. A non-empty value is the user's choice and is never overwritten.
/// Returns true when a value changed, so the caller knows to persist.
pub fn ensure_import_paths(archive_dir: &Path, settings: &mut HalluScribeSettings) -> bool {
    let mut changed = false;
    for (leaf, field) in [
        ("chatgpt", &mut settings.chatgpt_import_path),
        ("claude", &mut settings.claudeai_import_path),
        ("gemini", &mut settings.gemini_import_path),
    ] {
        if !field.trim().is_empty() {
            continue;
        }
        let dir = default_import_dir(archive_dir, leaf);
        let _ = fs::create_dir_all(&dir);
        if leaf == "gemini" {
            let takeout = GEMINI_TAKEOUT_SUBPATH
                .iter()
                .fold(dir.clone(), |path, part| path.join(part));
            let _ = fs::create_dir_all(&takeout);
        }
        *field = dir.to_string_lossy().into_owned();
        changed = true;
    }
    changed
}

/// Load settings for `archive_dir`, derive any missing import folders, and
/// persist when that changed anything. The single runtime entry point for
/// A1 seeding - both app startup and workspace creation go through it.
pub fn load_with_import_paths(archive_dir: &Path) -> HalluScribeSettings {
    let mut settings = super::load_settings(archive_dir);
    if ensure_import_paths(archive_dir, &mut settings) {
        let _ = super::save_settings(archive_dir, &settings);
    }
    settings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("halluscribe_imports_test_{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn blank_paths_are_derived_from_the_archive_root_and_created() {
        let dir = tmp_dir("derive");
        let mut settings = HalluScribeSettings::default();
        assert!(ensure_import_paths(&dir, &mut settings));

        assert_eq!(
            settings.chatgpt_import_path,
            dir.join("imports").join("chatgpt").to_string_lossy()
        );
        assert_eq!(
            settings.claudeai_import_path,
            dir.join("imports").join("claude").to_string_lossy()
        );
        assert_eq!(
            settings.gemini_import_path,
            dir.join("imports").join("gemini").to_string_lossy()
        );
        assert!(dir.join("imports").join("chatgpt").is_dir());
        assert!(dir.join("imports").join("claude").is_dir());
        assert!(dir
            .join("imports")
            .join("gemini")
            .join("Takeout")
            .join("My Activity")
            .join("Gemini Apps")
            .is_dir());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_values_are_never_overwritten() {
        let dir = tmp_dir("preserve");
        let mut settings = HalluScribeSettings {
            chatgpt_import_path: "D:/my/own/folder".to_string(),
            ..Default::default()
        };
        assert!(ensure_import_paths(&dir, &mut settings));
        assert_eq!(settings.chatgpt_import_path, "D:/my/own/folder");
        // The other two were blank, so they were derived.
        assert!(!settings.claudeai_import_path.is_empty());

        // Nothing left to derive: a second pass reports no change.
        assert!(!ensure_import_paths(&dir, &mut settings));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_guest_workspace_gets_its_own_paths_not_the_hosts() {
        let host = tmp_dir("owner_host");
        let guest = host.join("users").join("alex");
        fs::create_dir_all(&guest).unwrap();

        let mut host_settings = HalluScribeSettings::default();
        ensure_import_paths(&host, &mut host_settings);
        // A guest is seeded from the host, which blanks archive-level fields.
        let mut guest_settings = host_settings.seed_workspace_settings();
        ensure_import_paths(&guest, &mut guest_settings);

        assert_ne!(
            guest_settings.chatgpt_import_path,
            host_settings.chatgpt_import_path
        );
        assert!(guest_settings
            .chatgpt_import_path
            .starts_with(&guest.to_string_lossy().into_owned()));

        let _ = fs::remove_dir_all(&host);
    }

    #[test]
    fn load_with_import_paths_persists_the_derived_values() {
        let dir = tmp_dir("persist");
        let first = load_with_import_paths(&dir);
        assert!(!first.chatgpt_import_path.is_empty());

        // Re-read from disk (not in memory) to prove the save stuck.
        let reloaded = super::super::load_settings(&dir);
        assert_eq!(reloaded.chatgpt_import_path, first.chatgpt_import_path);

        let _ = fs::remove_dir_all(&dir);
    }
}
