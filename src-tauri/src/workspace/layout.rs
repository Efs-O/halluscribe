// HalluScribe - where a new workspace's archive folder is suggested.
//
// Guest archives live under `<default_root>/users/<slug>`, so every person on
// this machine lands inside the same hierarchy instead of an invented folder
// that later drifts. This is a SUGGESTION only - `create_workspace` still
// accepts any absolute path the user picks in the folder picker.
// See docs/internal/IMPORT_PATHS_PLAN.md Phase A1b.

use std::path::{Path, PathBuf};

/// Folder under the default archive root holding guest workspaces.
pub const USERS_DIR: &str = "users";

/// Filesystem-safe folder name for a workspace display name. Alphanumerics
/// (including non-ASCII, so Greek names survive) are kept and lowercased;
/// everything else becomes a single `-`.
pub fn workspace_slug(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.trim().chars() {
        if ch.is_alphanumeric() {
            slug.extend(ch.to_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').to_string()
}

/// Suggested archive folder for a new workspace. A name that slugs to nothing
/// yields the `users` folder itself, leaving the user to finish the path.
pub fn suggested_workspace_path(default_root: &Path, name: &str) -> PathBuf {
    let base = default_root.join(USERS_DIR);
    let slug = workspace_slug(name);
    if slug.is_empty() {
        base
    } else {
        base.join(slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_lowercases_and_replaces_separators() {
        assert_eq!(workspace_slug("Alex"), "alex");
        assert_eq!(workspace_slug("  Mary Jane  "), "mary-jane");
        assert_eq!(workspace_slug("A/B\\C:D"), "a-b-c-d");
        assert_eq!(workspace_slug("chara__2"), "chara-2");
    }

    #[test]
    fn slug_keeps_non_ascii_letters() {
        assert_eq!(workspace_slug("Χαρά"), "χαρά");
    }

    #[test]
    fn slug_is_empty_when_nothing_usable_remains() {
        assert_eq!(workspace_slug("   "), "");
        assert_eq!(workspace_slug("///"), "");
    }

    #[test]
    fn suggested_path_nests_under_users() {
        let root = Path::new("/archive");
        assert_eq!(
            suggested_workspace_path(root, "Alex"),
            root.join("users").join("alex")
        );
    }

    #[test]
    fn suggested_path_is_the_users_folder_for_an_unusable_name() {
        let root = Path::new("/archive");
        assert_eq!(suggested_workspace_path(root, "  "), root.join("users"));
    }
}
