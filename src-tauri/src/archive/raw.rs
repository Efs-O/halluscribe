// HalluScribe - raw transcript preservation (Persona Protocol Phase 1).
//
// The archive normally keeps only the Gemma summary plus a `source_jsonl`
// pointer to the original. Coding tools prune old logs, so that pointer dangles
// and the raw detail is lost forever. The sweep unconditionally copies each
// archived session's original transcript here, compressed with zstd (agent
// JSONL compresses roughly 10:1), so a persona layer built on summaries can
// always drill back to ground truth.
//
// Raw copies are the UNTOUCHED source: redaction and the secret scan apply only
// to the shareable `.md` summaries, never to these files. Persona Pack exports
// exclude the `raw/` directory unless the user opts in per-export, so raw secrets
// never leave the machine implicitly.
//
// A raw sliced out of a chat export is never destroyed by a re-import: a write
// that would replace one with DIFFERENT content moves the old copy into
// `raw/superseded/` first. A provider that hands back a trimmed or partially
// exported conversation therefore costs nothing - see `preserve_raw_bytes`.

use super::ArchiveError;
use chrono::Utc;
use std::path::{Component, Path};

/// Archive-relative directory holding preserved raw transcripts.
pub const RAW_DIR: &str = "raw";

/// Archive-relative directory holding raw copies that a later write replaced.
/// Nothing enumerates `raw/` - every reader (search, Persona Pack, the MCP
/// server, `read_raw_session`) reaches a raw through the index entry's
/// `raw_path` or through the capture manifest - so copies parked here are inert
/// until someone goes looking for them by hand.
pub const SUPERSEDED_DIR: &str = "raw/superseded";

/// zstd compression level. Level 3 (the library default) gives near-10:1 on
/// agent JSONL while staying fast enough to run inline during the sweep.
const ZSTD_LEVEL: i32 = 3;

/// Archive-relative path for a session's preserved raw transcript.
pub fn raw_rel_path(session_id: &str) -> String {
    format!("{RAW_DIR}/{session_id}.jsonl.zst")
}

/// Outcome of preserving one session's raw slice.
#[derive(Debug, Clone)]
pub struct PreservedRaw {
    /// Archive-relative path to the current raw, to record in the index entry.
    pub rel: String,
    /// Archive-relative path the previous raw was moved to, set only when this
    /// write replaced DIFFERENT content. `None` when there was no previous
    /// copy, or when the stored bytes already matched.
    pub superseded: Option<String>,
}

/// Compress `source` into `<archive_dir>/raw/<session_id>.jsonl.zst`, overwriting
/// any existing copy (a re-swept, changed session replaces its raw copy — the
/// transcript hash already detected the change). Returns the archive-relative
/// path to record in the index entry.
///
/// This is the one-file-per-session path: coding-tool transcripts, which their
/// tool only ever appends to. A later copy is a superset of the earlier one, so
/// a plain overwrite cannot lose anything and no version is kept. The
/// multi-session path ([`preserve_raw_bytes`]) does keep versions, because a
/// chat export is a snapshot of state the provider owns and can shrink.
pub fn preserve_raw(
    archive_dir: &Path,
    session_id: &str,
    source: &Path,
) -> Result<String, ArchiveError> {
    let bytes = std::fs::read(source)?;
    write_raw(archive_dir, session_id, &bytes)
}

/// Same as [`preserve_raw`] but for content already in memory, used when the
/// source file holds many sessions and only this session's slice may be
/// preserved (chat exports, the Ollama DB). Copying the file there would store
/// one copy of the entire export per conversation and make
/// `read_raw_session` return every other conversation alongside the wanted one.
///
/// **Never destroys the previous raw.** A chat export is a snapshot of a
/// conversation the PROVIDER owns, so a later export can hand back less than an
/// earlier one did — a conversation trimmed upstream, individual messages
/// deleted, or a partial/failed export. A plain overwrite would drop the
/// difference silently, with no way back. Instead, when the stored bytes differ
/// from `bytes`, the old copy is renamed into [`SUPERSEDED_DIR`] before the new
/// one is written.
///
/// Identical content is left alone, so re-running an import churns nothing.
pub fn preserve_raw_bytes(
    archive_dir: &Path,
    session_id: &str,
    bytes: &[u8],
) -> Result<PreservedRaw, ArchiveError> {
    // Order matters: park the old copy FIRST, and propagate a failure to do so
    // rather than writing anyway. Callers treat a preserve error as non-fatal
    // (the summary still archives, just without a raw pointer), so the worst
    // case leaves the previous raw intact on disk instead of losing it.
    let superseded = supersede_existing(archive_dir, session_id, bytes)?;
    let rel = write_raw(archive_dir, session_id, bytes)?;
    Ok(PreservedRaw { rel, superseded })
}

/// Move the stored raw for `session_id` aside when it holds content other than
/// `bytes`. Returns the archive-relative path it was moved to, or `None` when
/// nothing was stored or the stored bytes already matched.
fn supersede_existing(
    archive_dir: &Path,
    session_id: &str,
    bytes: &[u8],
) -> Result<Option<String>, ArchiveError> {
    let current = archive_dir.join(raw_rel_path(session_id));
    if !current.is_file() {
        return Ok(None);
    }
    let stored = std::fs::read(&current)?;
    // An unreadable or corrupt stored copy counts as different, so it is kept
    // rather than quietly overwritten — it is the only evidence of whatever
    // truncated it.
    let unchanged = zstd::decode_all(stored.as_slice())
        .map(|decoded| decoded == bytes)
        .unwrap_or(false);
    if unchanged {
        return Ok(None);
    }

    let rel = superseded_rel_path(archive_dir, session_id)?;
    let dest = archive_dir.join(&rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&current, &dest)?;
    Ok(Some(rel))
}

/// An unused archive-relative path under [`SUPERSEDED_DIR`] for this session,
/// stamped with the UTC time of the replacement so the copies read as a history.
fn superseded_rel_path(archive_dir: &Path, session_id: &str) -> Result<String, ArchiveError> {
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let base = format!("{SUPERSEDED_DIR}/{session_id}.{stamp}");
    for attempt in 0..100 {
        let rel = if attempt == 0 {
            format!("{base}.jsonl.zst")
        } else {
            format!("{base}-{attempt}.jsonl.zst")
        };
        if !archive_dir.join(&rel).exists() {
            return Ok(rel);
        }
    }
    // 100 replacements of one session inside the same millisecond is not a real
    // condition; refusing beats picking a path that would overwrite a version.
    Err(ArchiveError::Invalid(format!(
        "no free superseded raw path for session {session_id}"
    )))
}

/// Compress `bytes` to `raw/<session_id>.jsonl.zst`, replacing whatever is
/// there. The single write point for every raw in the archive.
///
/// Written atomically (`.zst.tmp` then rename) so a startup capture pass and a
/// concurrent sweep preserving the same session never race onto a torn file —
/// whichever rename lands last wins, and the content is identical either way.
fn write_raw(archive_dir: &Path, session_id: &str, bytes: &[u8]) -> Result<String, ArchiveError> {
    let rel = raw_rel_path(session_id);
    let dest = archive_dir.join(&rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let compressed = zstd::encode_all(bytes, ZSTD_LEVEL)?;
    let tmp_dest = dest.with_file_name(format!(
        "{}.tmp",
        dest.file_name().and_then(|n| n.to_str()).unwrap_or("raw")
    ));
    std::fs::write(&tmp_dest, compressed)?;
    std::fs::rename(&tmp_dest, &dest)?;
    Ok(rel)
}

/// Read and decompress a preserved raw transcript back to its original text.
/// Used by future readers (MCP `read_session`, Persona Pack) that want the
/// verbatim source rather than the summary.
pub fn read_raw(archive_dir: &Path, session_id: &str) -> Result<String, ArchiveError> {
    let path = archive_dir.join(raw_rel_path(session_id));
    let bytes = std::fs::read(&path)?;
    let decoded = zstd::decode_all(bytes.as_slice())?;
    String::from_utf8(decoded).map_err(|error| ArchiveError::Invalid(error.to_string()))
}

/// Read and decompress a raw transcript by its archive-relative path (the
/// index entry's recorded `raw_path`). Rejects paths that are absolute or
/// contain `..` components — index data must not escape the archive dir.
pub fn read_raw_at(archive_dir: &Path, rel_path: &str) -> Result<String, ArchiveError> {
    let rel = Path::new(rel_path);
    if rel_path.is_empty()
        || rel.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ArchiveError::Invalid(format!(
            "invalid archive-relative raw path: {rel_path}"
        )));
    }

    let bytes = std::fs::read(archive_dir.join(rel))?;
    let decoded = zstd::decode_all(bytes.as_slice())?;
    String::from_utf8(decoded).map_err(|error| ArchiveError::Invalid(error.to_string()))
}
