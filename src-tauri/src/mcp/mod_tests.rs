// HalluScribe - tests for MCP archive-dir resolution. `resolve_archive_dir`
// takes its override as a parameter rather than reading the environment
// itself, so these tests never mutate process-wide env state (which would
// be racy under parallel test execution).

use super::*;

#[test]
fn override_pointing_at_existing_dir_is_used_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let resolved = resolve_archive_dir(Some(dir.path().to_string_lossy().into_owned())).unwrap();
    assert_eq!(resolved, dir.path());
}

#[test]
fn override_pointing_at_missing_dir_errors() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist");
    let result = resolve_archive_dir(Some(missing.to_string_lossy().into_owned()));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("HALLUSCRIBE_DIR"));
}

mod identity {
    use super::*;
    use crate::workspace::{Workspace, WorkspaceRegistry};

    fn registry_with(workspaces: Vec<Workspace>, default_name: Option<&str>) -> WorkspaceRegistry {
        WorkspaceRegistry {
            active: None,
            workspaces,
            default_name: default_name.map(str::to_string),
        }
    }

    #[test]
    fn default_root_uses_registry_label_when_named() {
        let root = Path::new("/home/user/.halluscribe");
        let registry = registry_with(vec![], Some("EFSO"));
        let label = identity_label(root, Some(root), &registry);
        assert!(label.contains("\"EFSO\" (host default)"));
        assert!(label.contains(".halluscribe"));
    }

    #[test]
    fn default_root_without_label_says_host_default() {
        let root = Path::new("/home/user/.halluscribe");
        let label = identity_label(root, Some(root), &registry_with(vec![], None));
        assert!(label.contains("host default"));
    }

    #[test]
    fn registered_workspace_is_named() {
        let root = Path::new("/home/user/.halluscribe");
        let guest = PathBuf::from("/data/guest_archive");
        let registry = registry_with(
            vec![Workspace {
                name: "Maria".to_string(),
                path: guest.clone(),
                import_only: false,
            }],
            None,
        );
        let label = identity_label(&guest, Some(root), &registry);
        assert!(label.contains("workspace \"Maria\""));
        assert!(!label.contains("import-only"));
    }

    #[test]
    fn import_only_workspace_is_flagged_as_guest() {
        let guest = PathBuf::from("/data/guest_archive");
        let registry = registry_with(
            vec![Workspace {
                name: "Maria".to_string(),
                path: guest.clone(),
                import_only: true,
            }],
            None,
        );
        let label = identity_label(&guest, None, &registry);
        assert!(label.contains("workspace \"Maria\" (import-only guest)"));
    }

    #[test]
    fn unknown_path_is_marked_unregistered() {
        let root = Path::new("/home/user/.halluscribe");
        let label = identity_label(
            Path::new("/somewhere/else"),
            Some(root),
            &registry_with(vec![], None),
        );
        assert!(label.contains("unregistered archive"));
    }
}

#[test]
fn no_override_falls_back_to_home_dir_default() {
    // Without an override, resolution depends on the real HOME/USERPROFILE
    // and the real ~/.halluscribe existing or not - both outcomes are valid
    // here; this just checks the function does not panic and, when it does
    // resolve, that the path ends in the expected directory name.
    let result = resolve_archive_dir(None);
    if let Ok(path) = result {
        assert_eq!(path.file_name().unwrap(), ".halluscribe");
    }
}
