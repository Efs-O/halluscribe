// HalluScribe - removal of a session's private files and stale summaries.

use super::index::archive_path;
use super::{ArchiveError, IndexEntry, SUPERSEDED_DIR};
use std::fs;
use std::path::{Path, PathBuf};

/// The session id that owns a summary file (or a redaction backup of one), read
/// from its name: `{HH-MM-SS}-{slug}-{id}-sweep.md[.bak-{stamp}]`. Slugs never
/// contain a dash, so everything between the slug and `-sweep.md` is the id.
pub(super) fn summary_owner(file_name: &str) -> Option<&str> {
    let summary = match file_name.find(".md.bak-") {
        Some(end) => &file_name[..end + ".md".len()],
        None => file_name,
    };
    let time = summary.get(..9)?.as_bytes();
    let is_time = time.iter().enumerate().all(|(at, byte)| match at {
        2 | 5 | 8 => *byte == b'-',
        _ => byte.is_ascii_digit(),
    });
    if !is_time {
        return None;
    }
    let (_slug, rest) = summary[9..].split_once('-')?;
    rest.strip_suffix("-sweep.md").filter(|id| !id.is_empty())
}

/// Whether `file_name` is a superseded raw copy of exactly `id`:
/// `{id}.{YYYYMMDD}T...`. A bare `{id}.` prefix would also match a different
/// session whose id merely starts with `{id}.` (for example `abc` vs `abc.1`).
fn is_superseded_copy_of(file_name: &str, id: &str) -> bool {
    let Some(stamp) = file_name
        .strip_prefix(id)
        .and_then(|rest| rest.strip_prefix('.'))
    else {
        return false;
    };
    let bytes = stamp.as_bytes();
    bytes.len() > 9 && bytes[..8].iter().all(u8::is_ascii_digit) && bytes[8] == b'T'
}

/// The entries of `dir`, or none when it does not exist. Any other read error
/// is returned: a purge that silently skipped a directory it could not list
/// would report a delete as complete while private copies remain.
fn read_dir_if_present(dir: &Path) -> Result<Vec<fs::DirEntry>, ArchiveError> {
    match fs::read_dir(dir) {
        Ok(entries) => Ok(entries.collect::<Result<_, _>>()?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

/// Every summary and redaction backup under `sessions/` that belongs to `id`,
/// except `keep`. A re-sweep writes the summary under a new dated name, so
/// older names of the same session pile up unless they are found by owner.
fn owned_summary_files(
    archive_dir: &Path,
    id: &str,
    keep: Option<&Path>,
) -> Result<Vec<PathBuf>, ArchiveError> {
    let mut found = Vec::new();
    let mut pending = vec![archive_dir.join("sessions")];
    while let Some(dir) = pending.pop() {
        for entry in read_dir_if_present(&dir)? {
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                pending.push(path);
                continue;
            }
            let name = entry.file_name();
            let owned = name.to_str().and_then(summary_owner) == Some(id);
            if owned && keep != Some(path.as_path()) {
                found.push(path);
            }
        }
    }
    Ok(found)
}

/// Remove the summary a re-sweep replaced, when it was written under another
/// name. Redaction backups of it stay, as they do for an in-place rewrite.
pub(super) fn remove_replaced_summary(
    archive_dir: &Path,
    previous_rel: &str,
) -> Result<(), ArchiveError> {
    remove_if_file(&archive_path(archive_dir, previous_rel)?)
}

/// Remove everything private a deleted session left behind: its raw copies
/// (current, indexed and superseded), and every summary and backup it owns
/// except `markdown`, the current summary, which the caller removes last so a
/// failure here leaves the session listed and the delete retryable.
pub(super) fn remove_private_session_files(
    archive_dir: &Path,
    entry: &IndexEntry,
    markdown: &Path,
) -> Result<(), ArchiveError> {
    remove_if_file(&archive_dir.join(super::raw_rel_path(&entry.id)))?;
    if !entry.raw_path.is_empty() {
        remove_if_file(&archive_path(archive_dir, &entry.raw_path)?)?;
    }
    for file in read_dir_if_present(&archive_dir.join(SUPERSEDED_DIR))? {
        let name = file.file_name();
        if name
            .to_str()
            .is_some_and(|name| is_superseded_copy_of(name, &entry.id))
        {
            remove_if_file(&file.path())?;
        }
    }
    for path in owned_summary_files(archive_dir, &entry.id, Some(markdown))? {
        remove_if_file(&path)?;
    }
    // A summary whose name does not follow the pattern (renamed by hand, or an
    // older layout) still has its backups next to it.
    if let (Some(parent), Some(file_name)) = (
        markdown.parent(),
        markdown.file_name().and_then(|name| name.to_str()),
    ) {
        let backup_prefix = format!("{file_name}.bak-");
        for file in read_dir_if_present(parent)? {
            if file
                .file_name()
                .to_string_lossy()
                .starts_with(&backup_prefix)
            {
                remove_if_file(&file.path())?;
            }
        }
    }
    Ok(())
}

fn remove_if_file(path: &Path) -> Result<(), ArchiveError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
