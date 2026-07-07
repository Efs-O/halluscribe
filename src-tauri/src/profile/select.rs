// HalluScribe - profile source selection: consent + watermark filtering and
// batching. Pure functions over `IndexEntry`, no I/O, so the distiller's
// scope can be unit-tested without an archive on disk.

use crate::archive::IndexEntry;

/// Sessions per map-step batch. Reduced from 30 to 18 in Phase 2b when the
/// per-session evidence window grew from a flat 1,500-char truncation to a
/// ~3,000-char head+tail window (see `distill.rs` HEAD_CHARS/TAIL_CHARS):
/// 18 x (~3,000 body + ~200 metadata) ~= 57,600 evidence chars/map-call,
/// under the plan's ~60K char/call budget target (docs/internal/
/// PROFILE_QUALITY_PLAN.md Phase 2b) with margin for metadata variance.
pub const BATCH_SIZE: usize = 18;

/// Filter `entries` to those whose provider is in `profile_sources` (consent)
/// and, if `watermark` is set, whose timestamp is strictly newer than it
/// (incremental refresh). Returns newest-first.
pub fn select_sources<'a>(
    entries: &'a [IndexEntry],
    profile_sources: &[String],
    watermark: Option<&str>,
) -> Vec<&'a IndexEntry> {
    let mut filtered: Vec<&IndexEntry> = entries
        .iter()
        .filter(|entry| {
            profile_sources
                .iter()
                .any(|source| source == &entry.provider)
        })
        .filter(|entry| match watermark {
            Some(wm) if !wm.is_empty() => timestamp_of(entry) > wm,
            _ => true,
        })
        .collect();
    filtered.sort_by(|a, b| timestamp_of(b).cmp(timestamp_of(a)));
    filtered
}

/// The timestamp used for ordering and watermark comparison: `session_timestamp`
/// (RFC3339, sortable lexicographically) with a fallback to the coarser `date`
/// field for legacy entries that predate that column.
fn timestamp_of(entry: &IndexEntry) -> &str {
    if !entry.session_timestamp.is_empty() {
        &entry.session_timestamp
    } else {
        &entry.date
    }
}

/// Split an already-ordered selection into fixed-size batches for the map step.
pub fn chunk_batches<'a>(entries: &[&'a IndexEntry], size: usize) -> Vec<Vec<&'a IndexEntry>> {
    entries
        .chunks(size.max(1))
        .map(|chunk| chunk.to_vec())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, provider: &str, ts: &str) -> IndexEntry {
        IndexEntry {
            id: id.to_string(),
            project: "proj".to_string(),
            date: ts[..10.min(ts.len())].to_string(),
            title: format!("Session {id}"),
            tool: "Claude Code".to_string(),
            fill_pct: 90.0,
            session_timestamp: ts.to_string(),
            updated_at: String::new(),
            session_type: "building".to_string(),
            error_tags: Vec::new(),
            topic_tags: Vec::new(),
            archive_path: format!("sessions/proj/{id}.md"),
            source_jsonl: String::new(),
            source_size_bytes: 0,
            secret_flags: Vec::new(),
            provider: provider.to_string(),
            fill_estimated: false,
            transcript_hash: String::new(),
            raw_path: String::new(),
        }
    }

    #[test]
    fn consent_filtering_keeps_only_listed_providers() {
        let entries = vec![
            entry("a", "claude_code", "2026-06-01T00:00:00+00:00"),
            entry("b", "chatgpt", "2026-06-02T00:00:00+00:00"),
            entry("c", "codex", "2026-06-03T00:00:00+00:00"),
        ];
        let sources = vec!["claude_code".to_string(), "codex".to_string()];
        let selected = select_sources(&entries, &sources, None);
        let ids: Vec<&str> = selected.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "a"]);
    }

    #[test]
    fn watermark_filtering_excludes_sessions_not_newer() {
        let entries = vec![
            entry("old", "claude_code", "2026-06-01T00:00:00+00:00"),
            entry("same", "claude_code", "2026-06-05T00:00:00+00:00"),
            entry("new", "claude_code", "2026-06-10T00:00:00+00:00"),
        ];
        let sources = vec!["claude_code".to_string()];
        let selected = select_sources(&entries, &sources, Some("2026-06-05T00:00:00+00:00"));
        let ids: Vec<&str> = selected.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["new"]);
    }

    #[test]
    fn empty_watermark_behaves_like_full_rebuild() {
        let entries = vec![entry("a", "claude_code", "2026-06-01T00:00:00+00:00")];
        let sources = vec!["claude_code".to_string()];
        let selected = select_sources(&entries, &sources, Some(""));
        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn newest_first_ordering() {
        let entries = vec![
            entry("a", "claude_code", "2026-01-01T00:00:00+00:00"),
            entry("b", "claude_code", "2026-06-01T00:00:00+00:00"),
            entry("c", "claude_code", "2026-03-01T00:00:00+00:00"),
        ];
        let sources = vec!["claude_code".to_string()];
        let selected = select_sources(&entries, &sources, None);
        let ids: Vec<&str> = selected.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
    }

    #[test]
    fn chunk_batches_splits_into_fixed_size_groups() {
        let entries: Vec<IndexEntry> = (0..65)
            .map(|i| entry(&format!("s{i}"), "claude_code", "2026-06-01T00:00:00+00:00"))
            .collect();
        let refs: Vec<&IndexEntry> = entries.iter().collect();
        let batches = chunk_batches(&refs, BATCH_SIZE);
        assert_eq!(batches.len(), 4);
        assert_eq!(batches[0].len(), 18);
        assert_eq!(batches[1].len(), 18);
        assert_eq!(batches[2].len(), 18);
        assert_eq!(batches[3].len(), 11);
    }

    #[test]
    fn chunk_batches_empty_input_yields_no_batches() {
        let refs: Vec<&IndexEntry> = Vec::new();
        assert!(chunk_batches(&refs, BATCH_SIZE).is_empty());
    }
}
