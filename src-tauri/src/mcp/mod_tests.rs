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
