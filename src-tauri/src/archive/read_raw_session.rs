// HalluScribe - `read_raw_session` core (feature 4e): paged access to one
// session's preserved raw transcript, shared by both agent surfaces (MCP
// `mcp/server.rs` and the in-app Gemma chat `briefing/tools.rs`).
//
// Split out of `archive::raw` to stay under the 350 LOC/file limit — the
// preservation/decompression primitives it builds on (`read_raw_at`,
// `raw_rel_path`) stay in `raw.rs` unchanged.

use super::{find_session, load_captured, raw_rel_path, read_raw_at, ArchiveError};
use std::path::Path;

/// One page of a preserved raw transcript, returned by `read_raw_session`.
/// Mirrors `mcp::server::do_get_digest`'s slice shape so both agent-facing
/// surfaces present a paged raw read the same way.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RawSessionPage {
    pub session_id: String,
    pub text: String,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub total_bytes: usize,
    pub truncated: bool,
}

/// Largest byte index `<= i` that lands on a UTF-8 character boundary of `s`.
/// The same small helper is duplicated in `mcp::server` and `archive::redact`
/// (each private to its module, none `pub` to reuse from here) — this is a
/// third copy rather than a cross-module dependency for six lines of logic.
fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Resolve which archive-relative raw file backs `session_id`: prefer the
/// index entry's `raw_path` (set by the sweep or `backfill_raw`), falling
/// back to the startup-capture location (`raw/<id>.jsonl.zst`) for sessions
/// that only ever got a capture-pass raw and no index entry (L2's
/// "unsummarised raw" sessions).
fn resolve_raw_rel_path(archive_dir: &Path, session_id: &str) -> Option<String> {
    if let Some(entry) = find_session(archive_dir, session_id) {
        if !entry.raw_path.is_empty() && archive_dir.join(&entry.raw_path).is_file() {
            return Some(entry.raw_path);
        }
    }
    let captured_rel = raw_rel_path(session_id);
    if load_captured(archive_dir).contains_key(session_id)
        || archive_dir.join(&captured_rel).is_file()
    {
        return Some(captured_rel);
    }
    None
}

/// Read one page of a session's preserved raw transcript — the verbatim
/// source file, not the distilled summary. `offset`/`max_chars` are byte
/// offsets into the decompressed UTF-8 text, clamped to character
/// boundaries, identical semantics to `get_digest`'s paging. Never fabricates
/// content: a session with no raw anywhere, or a raw that isn't valid UTF-8
/// text, is an honest error rather than an empty or partial page.
pub fn read_raw_session(
    archive_dir: &Path,
    session_id: &str,
    offset: usize,
    max_chars: usize,
) -> Result<RawSessionPage, ArchiveError> {
    let rel_path = resolve_raw_rel_path(archive_dir, session_id)
        .ok_or_else(|| ArchiveError::Invalid("no raw copy exists for this session".to_string()))?;
    let full = read_raw_at(archive_dir, &rel_path).map_err(|error| match error {
        ArchiveError::Invalid(_) => ArchiveError::Invalid(
            "raw transcript is not valid UTF-8 text for this session".to_string(),
        ),
        other => other,
    })?;

    let total = full.len();
    let start = floor_char_boundary(&full, offset);
    let end = floor_char_boundary(&full, start.saturating_add(max_chars).min(total));
    let truncated = end < total;
    let next_offset = if truncated { Some(end) } else { None };

    Ok(RawSessionPage {
        session_id: session_id.to_string(),
        text: full[start..end].to_string(),
        offset: start,
        next_offset,
        total_bytes: total,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::preserve_raw;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "halluscribe_read_raw_session_test_{}_{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_index_entry(dir: &Path, id: &str, raw_path: &str) {
        let idx = serde_json::json!({
            "sessions": [{
                "id": id,
                "project": "proj",
                "date": "2026-07-15",
                "title": "Title",
                "tool": "Claude Code",
                "fill_pct": 60.0,
                "session_type": "debugging",
                "error_tags": [],
                "topic_tags": [],
                "archive_path": "sessions/proj/2026-07-15/session.md",
                "source_jsonl": "unused.jsonl",
                "raw_path": raw_path,
            }]
        });
        std::fs::write(
            dir.join("index.json"),
            serde_json::to_string_pretty(&idx).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn pages_an_indexed_session() {
        let dir = tmp_dir("indexed");
        let source = dir.join("session.jsonl");
        std::fs::write(&source, "hello world\n".repeat(10)).unwrap();
        let rel = preserve_raw(&dir, "id1", &source).unwrap();
        write_index_entry(&dir, "id1", &rel);

        let page = read_raw_session(&dir, "id1", 0, 1_000).unwrap();
        assert_eq!(page.session_id, "id1");
        assert_eq!(page.text, "hello world\n".repeat(10));
        assert_eq!(page.offset, 0);
        assert!(page.next_offset.is_none());
        assert!(!page.truncated);
        assert_eq!(page.total_bytes, page.text.len());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pages_a_captured_only_session() {
        let dir = tmp_dir("captured_only");
        let source = dir.join("session.jsonl");
        std::fs::write(&source, "captured but never summarised\n").unwrap();
        preserve_raw(&dir, "id2", &source).unwrap();
        // No index.json entry at all: only the raw file on disk backs this id
        // (mirrors the L2 "unsummarised raw" case, whether or not captured.json
        // itself has a record - the on-disk file is the other half of the check).

        let page = read_raw_session(&dir, "id2", 0, 1_000).unwrap();
        assert_eq!(page.text, "captured but never summarised\n");
        assert!(!page.truncated);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clamps_paging_to_utf8_char_boundaries() {
        let dir = tmp_dir("utf8_paging");
        let source = dir.join("session.jsonl");
        let contents = "δοκιμή κειμένου ".repeat(50);
        std::fs::write(&source, &contents).unwrap();
        preserve_raw(&dir, "id3", &source).unwrap();

        // Walk small pages across the whole text; every page must be valid
        // UTF-8 (no split codepoint) and the pages must fully reconstruct the
        // source with nothing dropped or duplicated at a page edge.
        let mut offset = 0usize;
        let mut rebuilt = String::new();
        loop {
            let page = read_raw_session(&dir, "id3", offset, 7).unwrap();
            rebuilt.push_str(&page.text);
            match page.next_offset {
                Some(next) => offset = next,
                None => break,
            }
        }
        assert_eq!(rebuilt, contents);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn offset_beyond_end_is_empty_and_not_truncated() {
        let dir = tmp_dir("offset_beyond_end");
        let source = dir.join("session.jsonl");
        std::fs::write(&source, "short\n").unwrap();
        preserve_raw(&dir, "id4", &source).unwrap();

        let page = read_raw_session(&dir, "id4", 10_000, 100).unwrap();
        assert_eq!(page.text, "");
        assert!(!page.truncated);
        assert!(page.next_offset.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_raw_is_an_honest_error() {
        let dir = tmp_dir("missing");
        let error = read_raw_session(&dir, "no-such-id", 0, 100).unwrap_err();
        assert!(error
            .to_string()
            .contains("no raw copy exists for this session"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn binary_raw_is_an_honest_error() {
        let dir = tmp_dir("binary");
        let binary = vec![0xFF, 0xFE, 0x00, 0xFF, 0x00, 0x01];
        let compressed = zstd::encode_all(binary.as_slice(), 3).unwrap();
        std::fs::create_dir_all(dir.join("raw")).unwrap();
        std::fs::write(dir.join(raw_rel_path("id5")), compressed).unwrap();

        let error = read_raw_session(&dir, "id5", 0, 100).unwrap_err();
        assert!(error
            .to_string()
            .contains("raw transcript is not valid UTF-8 text for this session"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
