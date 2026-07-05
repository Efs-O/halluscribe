// HalluScribe - Persona Pack export (Persona Protocol Phase 4, per-scope in
// Parity Phase B).
//
// One button → `<user>-<scope>-persona-<date>.zip`, the portable "protocol"
// artifact any future agent can ingest: `profile.md` at boot, `archive/`
// behind retrieval.
//
// Privacy model: the pack is per-scope — a Work pack carries only the Work
// profile and Work-consented coding providers; a Personal pack additionally
// carries the private-chat-derived life context (ChatGPT/Claude.ai/Gemini),
// since Personal's consent list is a superset of Work's (`sources_for_scope`).
// Which scope leaves the machine is the user's explicit choice on each
// export — the button lives on both profile panels. Raw transcripts (`raw/`)
// are excluded unless the caller explicitly opts in per-export, because raw
// copies are the un-redacted source. The `.md` summaries are already
// redaction-applied on disk (the sweep re-applies the ledger), so `archive/`
// needs no further scrubbing here.

use crate::archive::{self, IndexEntry};
use crate::profile::ProfileScope;
use chrono::{DateTime, Utc};
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

#[derive(Debug)]
pub enum PackError {
    Io(io::Error),
    Zip(zip::result::ZipError),
    Json(serde_json::Error),
    /// No distilled profile exists yet for the requested scope — a pack is
    /// meaningless without it.
    NoProfile,
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Zip(error) => write!(f, "zip error: {error}"),
            Self::Json(error) => write!(f, "JSON error: {error}"),
            Self::NoProfile => write!(
                f,
                "no profile has been built yet for this scope — build the profile before exporting a pack"
            ),
        }
    }
}

impl From<io::Error> for PackError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<zip::result::ZipError> for PackError {
    fn from(error: zip::result::ZipError) -> Self {
        Self::Zip(error)
    }
}
impl From<serde_json::Error> for PackError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// What an export produced, surfaced to the UI.
#[derive(Debug, Clone)]
pub struct PackSummary {
    pub path: PathBuf,
    pub session_count: usize,
    pub digest_count: usize,
    pub raw_count: usize,
    pub includes_raw: bool,
}

/// Keep only index entries whose provider is in the consented `sources` list.
/// Pure so the consent filter is unit-tested without touching disk.
pub fn select_pack_entries(entries: Vec<IndexEntry>, sources: &[String]) -> Vec<IndexEntry> {
    entries
        .into_iter()
        .filter(|entry| sources.iter().any(|source| source == &entry.provider))
        .collect()
}

/// Count sessions in `sources` consent with a preserved raw transcript on
/// disk — the exact number `export_persona_pack(.., include_raw = true)`
/// would bundle for that consent list. Lets the UI show "incl. raw (N
/// available)" for the panel's scope without running an export. Mirrors the
/// export loop's filter: non-empty `raw_path` whose `.zst` still exists.
pub fn count_available_raw(archive_dir: &Path, sources: &[String]) -> usize {
    select_pack_entries(archive::read_sessions(archive_dir), sources)
        .into_iter()
        .filter(|entry| !entry.raw_path.is_empty() && archive_dir.join(&entry.raw_path).is_file())
        .count()
}

/// Default file name `<user>-<scope>-persona-<YYYY-MM-DD>.zip`. `user` is
/// sanitised to lowercase alphanumerics and dashes; empty/odd values fall
/// back to "user".
pub fn default_pack_name(user: &str, scope: ProfileScope, now: DateTime<Utc>) -> String {
    let slug: String = user
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "user" } else { slug };
    format!(
        "{slug}-{}-persona-{}.zip",
        scope.as_str(),
        now.format("%Y-%m-%d")
    )
}

/// 64-bit FNV-1a content fingerprint (hex). A cheap integrity marker for the
/// manifest, not a cryptographic hash — upgrade to sha256 if pack import lands.
fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Write the Persona Pack zip to `dest_zip`. `sources` is the consented
/// provider list for `scope` (`sources_for_scope(profile_sources, scope)`);
/// `profile_md` is the distilled profile for that scope; `digests` are
/// `(filename, contents)` pairs. When `include_raw` is on, each exported
/// session's `raw/<id>.jsonl.zst` is added verbatim (already compressed →
/// stored, not re-deflated).
#[allow(clippy::too_many_arguments)]
pub fn export_persona_pack(
    archive_dir: &Path,
    dest_zip: &Path,
    sources: &[String],
    profile_md: &str,
    digests: &[(String, String)],
    include_raw: bool,
    version: &str,
    embedding_model: &str,
    scope: ProfileScope,
    now: DateTime<Utc>,
) -> Result<PackSummary, PackError> {
    let entries = select_pack_entries(archive::read_sessions(archive_dir), sources);

    if let Some(parent) = dest_zip.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::create(dest_zip)?;
    let mut zip = ZipWriter::new(file);
    let text = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

    // profile.md
    zip.start_file("profile.md", text)?;
    zip.write_all(profile_md.as_bytes())?;

    // digest/<filename>
    for (name, contents) in digests {
        zip.start_file(format!("digest/{name}"), text)?;
        zip.write_all(contents.as_bytes())?;
    }

    // archive/<archive_path> — the redaction-applied .md summaries.
    for entry in &entries {
        let src = archive_dir.join(&entry.archive_path);
        let Ok(bytes) = std::fs::read(&src) else {
            continue; // index entry without its .md (deleted out of band) — skip.
        };
        zip.start_file(format!("archive/{}", entry.archive_path), text)?;
        zip.write_all(&bytes)?;
    }

    // raw/<id>.jsonl.zst — opt-in only.
    let mut raw_count = 0usize;
    if include_raw {
        for entry in &entries {
            if entry.raw_path.is_empty() {
                continue;
            }
            let Ok(bytes) = std::fs::read(archive_dir.join(&entry.raw_path)) else {
                continue;
            };
            zip.start_file(entry.raw_path.clone(), stored)?;
            zip.write_all(&bytes)?;
            raw_count += 1;
        }
    }

    // index.json — filtered to the exported sessions.
    let index_json = serde_json::to_string_pretty(&serde_json::json!({ "sessions": entries }))?;
    zip.start_file("index.json", text)?;
    zip.write_all(index_json.as_bytes())?;

    // manifest.json — last, so its hashes cover the emitted content.
    let manifest = serde_json::json!({
        "version": version,
        "generated_at": now.to_rfc3339(),
        "scope": scope.as_str(),
        "session_count": entries.len(),
        "digest_count": digests.len(),
        "consent_sources": sources,
        "includes_raw": include_raw,
        "raw_count": raw_count,
        "embedding_model": embedding_model,
        "profile_fingerprint_fnv1a": fingerprint(profile_md.as_bytes()),
    });
    zip.start_file("manifest.json", text)?;
    zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;

    zip.finish()?;

    Ok(PackSummary {
        path: dest_zip.to_path_buf(),
        session_count: entries.len(),
        digest_count: digests.len(),
        raw_count,
        includes_raw: include_raw,
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
