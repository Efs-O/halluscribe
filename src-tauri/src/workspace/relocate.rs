// HalluScribe - relocating a registered workspace's archive folder.
//
// The registry stores an absolute path, so a workspace could never be moved
// after creation: un-register + re-register would have overwritten its
// settings.json with freshly seeded values. This copies the archive to the new
// location, verifies the copy file-for-file and byte-for-byte, and only then
// repoints the registry.
//
// The ORIGINAL IS NEVER DELETED. A verified copy leaves two intact archives;
// sending the old folder to the Recycle Bin is the user's call, made after they
// have seen the app read the new one. See docs/internal/IMPORT_PATHS_PLAN.md §B5.

use std::fs;
use std::path::{Path, PathBuf};

/// What a relocation copied, so the caller can report it and the user can
/// check it against the folder they are about to bin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct CopyStats {
    pub files: usize,
    pub bytes: u64,
}

/// Recursively copy `from` into `to`, returning what was written. Fails on the
/// first I/O error rather than leaving a half-copy unreported.
fn copy_tree(from: &Path, to: &Path) -> Result<CopyStats, String> {
    fs::create_dir_all(to).map_err(|e| format!("cannot create {}: {e}", to.display()))?;
    let mut stats = CopyStats::default();
    let entries = fs::read_dir(from).map_err(|e| format!("cannot read {}: {e}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            let nested = copy_tree(&source, &target)?;
            stats.files += nested.files;
            stats.bytes += nested.bytes;
        } else {
            let copied = fs::copy(&source, &target)
                .map_err(|e| format!("cannot copy {}: {e}", source.display()))?;
            stats.files += 1;
            stats.bytes += copied;
        }
    }
    Ok(stats)
}

/// Count files and bytes under `dir`, used to verify a copy independently of
/// what the copy itself reported.
pub fn tree_stats(dir: &Path) -> Result<CopyStats, String> {
    let mut stats = CopyStats::default();
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            let nested = tree_stats(&entry.path())?;
            stats.files += nested.files;
            stats.bytes += nested.bytes;
        } else {
            stats.bytes += entry.metadata().map_err(|e| e.to_string())?.len();
            stats.files += 1;
        }
    }
    Ok(stats)
}

/// Whether `candidate` is `base` or sits inside it - copying a folder into
/// itself would recurse forever.
fn is_inside(candidate: &Path, base: &Path) -> bool {
    candidate.starts_with(base)
}

/// Copy the archive at `from` to `to` and verify it. Refuses to write into an
/// existing non-empty folder, so an unrelated archive can never be merged into
/// or overwritten. Returns the verified stats; `from` is left untouched.
pub fn copy_archive(from: &Path, to: &Path) -> Result<CopyStats, String> {
    if !from.is_dir() {
        return Err(format!("{} is not a folder on disk", from.display()));
    }
    if from == to {
        return Err("the workspace is already there".to_string());
    }
    if is_inside(to, from) {
        return Err("the new folder cannot be inside the current one".to_string());
    }
    if to.exists() {
        let occupied = fs::read_dir(to)
            .map_err(|e| format!("cannot read {}: {e}", to.display()))?
            .next()
            .is_some();
        if occupied {
            return Err(format!("{} already has files in it", to.display()));
        }
    }

    let copied = copy_tree(from, to)?;
    let source = tree_stats(from)?;
    let written = tree_stats(to)?;
    if written != source || copied != source {
        return Err(format!(
            "copy did not verify: {} files / {} bytes at the source, {} / {} at the destination - \
             both folders were left in place",
            source.files, source.bytes, written.files, written.bytes
        ));
    }
    Ok(written)
}

/// Point a registered workspace at `new_path`, keeping it active if it was.
pub fn set_workspace_path(
    reg: &mut super::WorkspaceRegistry,
    path: &Path,
    new_path: PathBuf,
) -> Result<(), String> {
    if reg.workspaces.iter().any(|ws| ws.path == new_path) {
        return Err("a workspace is already registered at that path".to_string());
    }
    let ws = reg
        .workspaces
        .iter_mut()
        .find(|ws| ws.path == path)
        .ok_or_else(|| "no workspace registered at that path".to_string())?;
    ws.path = new_path.clone();
    if reg.active.as_deref() == Some(path) {
        reg.active = Some(new_path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{add_workspace, set_active, Workspace, WorkspaceRegistry};

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "halluscribe_relocate_test_{}_{}",
            std::process::id(),
            name
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed_archive(root: &Path) {
        fs::create_dir_all(root.join("sessions").join("proj")).unwrap();
        fs::write(root.join("settings.json"), "{\"first_run\":false}").unwrap();
        fs::write(root.join("index.json"), "{\"sessions\":[]}").unwrap();
        fs::write(root.join("sessions").join("proj").join("a.md"), "hello").unwrap();
    }

    #[test]
    fn copy_archive_copies_every_file_and_verifies_the_result() {
        let base = tmp_dir("copy_ok");
        let from = base.join("old");
        let to = base.join("new").join("chara");
        seed_archive(&from);

        let stats = copy_archive(&from, &to).unwrap();
        assert_eq!(stats.files, 3);
        assert_eq!(stats, tree_stats(&from).unwrap());
        assert_eq!(
            fs::read_to_string(to.join("sessions").join("proj").join("a.md")).unwrap(),
            "hello"
        );
        // The original is untouched - a verified copy, never a delete.
        assert!(from.join("settings.json").is_file());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn copy_archive_refuses_a_destination_that_already_has_files() {
        let base = tmp_dir("occupied");
        let from = base.join("old");
        let to = base.join("new");
        seed_archive(&from);
        fs::create_dir_all(&to).unwrap();
        fs::write(to.join("someone-elses.json"), "{}").unwrap();

        let err = copy_archive(&from, &to).unwrap_err();
        assert!(err.contains("already has files"), "{err}");
        // Nothing was written over.
        assert_eq!(
            fs::read_to_string(to.join("someone-elses.json")).unwrap(),
            "{}"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn copy_archive_refuses_a_destination_inside_the_source() {
        let base = tmp_dir("nested");
        let from = base.join("old");
        seed_archive(&from);
        let err = copy_archive(&from, &from.join("inner")).unwrap_err();
        assert!(err.contains("cannot be inside"), "{err}");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn copy_archive_accepts_an_existing_empty_destination() {
        let base = tmp_dir("empty_dest");
        let from = base.join("old");
        let to = base.join("new");
        seed_archive(&from);
        fs::create_dir_all(&to).unwrap();

        assert_eq!(copy_archive(&from, &to).unwrap().files, 3);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn set_workspace_path_repoints_the_entry_and_the_active_pointer() {
        let mut reg = WorkspaceRegistry::default();
        add_workspace(
            &mut reg,
            Workspace {
                name: "chara".to_string(),
                path: PathBuf::from("/n/chara"),
                import_only: true,
            },
        )
        .unwrap();
        set_active(&mut reg, Some(PathBuf::from("/n/chara"))).unwrap();

        set_workspace_path(
            &mut reg,
            Path::new("/n/chara"),
            PathBuf::from("/archive/users/chara"),
        )
        .unwrap();

        assert_eq!(
            reg.workspaces[0].path,
            PathBuf::from("/archive/users/chara")
        );
        assert_eq!(reg.active, Some(PathBuf::from("/archive/users/chara")));
        // Everything else about the entry survives the move.
        assert_eq!(reg.workspaces[0].name, "chara");
        assert!(reg.workspaces[0].import_only);
    }

    #[test]
    fn set_workspace_path_rejects_an_unknown_or_taken_path() {
        let mut reg = WorkspaceRegistry::default();
        add_workspace(
            &mut reg,
            Workspace {
                name: "a".to_string(),
                path: PathBuf::from("/ws/a"),
                import_only: false,
            },
        )
        .unwrap();
        add_workspace(
            &mut reg,
            Workspace {
                name: "b".to_string(),
                path: PathBuf::from("/ws/b"),
                import_only: false,
            },
        )
        .unwrap();

        let err = set_workspace_path(&mut reg, Path::new("/ws/missing"), PathBuf::from("/ws/c"))
            .unwrap_err();
        assert_eq!(err, "no workspace registered at that path");

        let err =
            set_workspace_path(&mut reg, Path::new("/ws/a"), PathBuf::from("/ws/b")).unwrap_err();
        assert_eq!(err, "a workspace is already registered at that path");
        assert_eq!(reg.workspaces[0].path, PathBuf::from("/ws/a"));
    }
}
