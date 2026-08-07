// HalluScribe - one-time raw transcript backfill (Persona Parity Phase A).
//
// Raw preservation (`archive::raw`) only ever runs during a sweep, so the
// ~1300 sessions archived before raw preservation existed have no
// raw copy. This is a non-destructive, no-inference recovery pass: for every
// already-archived session missing a raw copy, if its original
// `source_jsonl` still exists on disk, compress and record it exactly as the
// sweep would have. Sessions whose source tool already pruned the file are
// counted, never treated as an error - the backfill always completes.

use super::ArchiveError;
use std::collections::HashMap;
use std::path::Path;

/// Per-session raw slices for one source file, or `None` when that source maps
/// 1:1 to a session and the whole file is the correct raw.
type SourceSlices = Option<HashMap<String, String>>;

/// Outcome of one backfill pass, returned to the UI.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct BackfillResult {
    pub recovered: usize,
    pub already_had: usize,
    pub source_missing: usize,
    /// Sessions that HAD a raw, but the wrong one: archives written before
    /// per-session slicing stored the whole export for every conversation in
    /// it. Replaced in place with the correct slice.
    pub repaired: usize,
    pub total: usize,
}

/// Recover raw transcripts for archived sessions that don't have one yet.
/// Never aborts on a single session's missing/unreadable source - that
/// session is simply counted under `source_missing` and the pass continues.
pub fn backfill_raw(archive_dir: &Path) -> Result<BackfillResult, ArchiveError> {
    let entries = super::read_sessions(archive_dir);
    let mut result = BackfillResult {
        total: entries.len(),
        ..Default::default()
    };

    // One parse per multi-session source file, shared by every session that
    // came out of it: an 81-conversation export is read once, not 81 times.
    let mut slice_cache: HashMap<(String, String), SourceSlices> = HashMap::new();

    for entry in entries {
        let has_raw = !entry.raw_path.is_empty() && archive_dir.join(&entry.raw_path).is_file();
        let multi_session = crate::readers::is_multi_session_provider(&entry.provider);

        // A raw stored for a 1:1 source is correct by construction. Only
        // multi-session sources need their existing raw re-checked, because
        // pre-slicing builds stored the whole export under every session id.
        if has_raw && !multi_session {
            result.already_had += 1;
            continue;
        }

        let source = Path::new(&entry.source_jsonl);
        if entry.source_jsonl.is_empty() || !source.is_file() {
            // Without the source there is nothing to compare against or
            // rebuild from; a wrong-but-present raw is left untouched rather
            // than discarded.
            if has_raw {
                result.already_had += 1;
            } else {
                result.source_missing += 1;
            }
            continue;
        }

        let slices = slice_cache
            .entry((entry.source_jsonl.clone(), entry.provider.clone()))
            .or_insert_with(|| crate::readers::raw_slices_for_source(source, &entry.provider));

        let preserved = match slices {
            // Multi-session source: only this session's own slice may be
            // preserved. An id absent from the current export (conversation
            // deleted upstream, or the export failed to parse) has nothing to
            // recover — never fall back to copying the whole file here.
            Some(by_id) => match by_id.get(&entry.id) {
                Some(slice) => {
                    // Already correct: leave the stored bytes alone.
                    if has_raw && stored_raw_matches(archive_dir, &entry, slice) {
                        result.already_had += 1;
                        continue;
                    }
                    super::preserve_raw_bytes(archive_dir, &entry.id, slice.as_bytes())
                }
                None => {
                    if has_raw {
                        result.already_had += 1;
                    } else {
                        result.source_missing += 1;
                    }
                    continue;
                }
            },
            None => super::preserve_raw(archive_dir, &entry.id, source),
        };

        match preserved {
            Ok(rel) => {
                super::set_raw_path(archive_dir, &entry.id, rel)?;
                if has_raw {
                    result.repaired += 1;
                } else {
                    result.recovered += 1;
                }
            }
            Err(_) => result.source_missing += 1,
        }
    }

    Ok(result)
}

/// Whether the raw already on disk for `entry` is byte-identical to `slice`.
/// An unreadable stored raw counts as a mismatch, so the repair rewrites it.
fn stored_raw_matches(archive_dir: &Path, entry: &super::IndexEntry, slice: &str) -> bool {
    super::read_raw_at(archive_dir, &entry.raw_path)
        .map(|stored| stored == slice)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("halluscribe_backfill_test_{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn index_entry_json(id: &str, source_jsonl: &str, raw_path: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "project": "proj",
            "date": "2026-04-15",
            "title": "Title",
            "tool": "Claude Code",
            "fill_pct": 60.0,
            "session_type": "debugging",
            "error_tags": [],
            "topic_tags": [],
            "archive_path": "sessions/proj/2026-04-15/12-00-00-claudecode-sweep.md",
            "source_jsonl": source_jsonl,
            "raw_path": raw_path,
        })
    }

    fn import_entry_json(id: &str, source: &str, provider: &str) -> serde_json::Value {
        let mut entry = index_entry_json(id, source, "");
        entry["provider"] = serde_json::json!(provider);
        entry
    }

    fn chatgpt_export(marker_a: &str, marker_b: &str) -> String {
        format!(
            r#"[
              {{
                "id": "conv-a", "title": "A", "create_time": 1710000000, "current_node": "n1",
                "mapping": {{ "n1": {{ "id": "n1", "parent": null, "children": [],
                  "message": {{ "author": {{ "role": "user" }},
                    "content": {{ "content_type": "text", "parts": ["{marker_a}"] }} }} }} }}
              }},
              {{
                "id": "conv-b", "title": "B", "create_time": 1710009999, "current_node": "n2",
                "mapping": {{ "n2": {{ "id": "n2", "parent": null, "children": [],
                  "message": {{ "author": {{ "role": "user" }},
                    "content": {{ "content_type": "text", "parts": ["{marker_b}"] }} }} }} }}
              }}
            ]"#
        )
    }

    /// The headline regression: several sessions sharing one export file must
    /// each recover their OWN conversation, not a copy of the whole export.
    #[test]
    fn multi_session_source_recovers_a_distinct_slice_per_session() {
        let dir = tmp_dir("multi_session");
        let export = dir.join("conversations.json");
        fs::write(&export, chatgpt_export("ALPHA-MARKER", "BETA-MARKER")).unwrap();
        let source = export.to_string_lossy().to_string();

        write_index(
            &dir,
            vec![
                import_entry_json("conv-a", &source, "chatgpt"),
                import_entry_json("conv-b", &source, "chatgpt"),
            ],
        );

        let result = backfill_raw(&dir).unwrap();
        assert_eq!(result.recovered, 2);
        assert_eq!(result.source_missing, 0);

        let entries = super::super::read_sessions(&dir);
        let raw_a = super::super::read_raw(&dir, "conv-a").expect("conv-a raw");
        let raw_b = super::super::read_raw(&dir, "conv-b").expect("conv-b raw");

        assert!(raw_a.contains("ALPHA-MARKER"));
        assert!(
            !raw_a.contains("BETA-MARKER"),
            "conv-a recovered the whole export instead of its own conversation"
        );
        assert!(raw_b.contains("BETA-MARKER"));
        assert!(!raw_b.contains("ALPHA-MARKER"));

        // Both raws are strictly smaller than the export they came from.
        let export_len = fs::read(&export).unwrap().len();
        assert!(raw_a.len() < export_len && raw_b.len() < export_len);
        assert!(entries.iter().all(|e| !e.raw_path.is_empty()));

        let _ = fs::remove_dir_all(&dir);
    }

    /// Chara's exact situation: sessions already carry a raw, but it is the
    /// whole export stored under every conversation id. The repair must replace
    /// each with its own slice instead of skipping them as "already had".
    #[test]
    fn repairs_existing_whole_export_raws_in_place() {
        let dir = tmp_dir("repair_whole_export");
        let export = dir.join("conversations.json");
        fs::write(&export, chatgpt_export("ALPHA-MARKER", "BETA-MARKER")).unwrap();
        let source = export.to_string_lossy().to_string();

        // Reproduce the defect: both sessions get the whole export as raw.
        let rel_a = super::super::preserve_raw(&dir, "conv-a", &export).unwrap();
        let rel_b = super::super::preserve_raw(&dir, "conv-b", &export).unwrap();
        assert_eq!(
            super::super::read_raw(&dir, "conv-a").unwrap(),
            super::super::read_raw(&dir, "conv-b").unwrap(),
            "precondition: both raws are the same whole-export copy"
        );

        let mut entry_a = import_entry_json("conv-a", &source, "chatgpt");
        entry_a["raw_path"] = serde_json::json!(rel_a);
        let mut entry_b = import_entry_json("conv-b", &source, "chatgpt");
        entry_b["raw_path"] = serde_json::json!(rel_b);
        write_index(&dir, vec![entry_a, entry_b]);

        let result = backfill_raw(&dir).unwrap();
        assert_eq!(result.repaired, 2, "both wrong raws must be replaced");
        assert_eq!(result.already_had, 0);
        assert_eq!(result.recovered, 0);

        let raw_a = super::super::read_raw(&dir, "conv-a").unwrap();
        let raw_b = super::super::read_raw(&dir, "conv-b").unwrap();
        assert!(raw_a.contains("ALPHA-MARKER") && !raw_a.contains("BETA-MARKER"));
        assert!(raw_b.contains("BETA-MARKER") && !raw_b.contains("ALPHA-MARKER"));

        // Idempotent: a second pass finds them correct and changes nothing.
        let again = backfill_raw(&dir).unwrap();
        assert_eq!(again.repaired, 0);
        assert_eq!(again.already_had, 2);

        let _ = fs::remove_dir_all(&dir);
    }

    /// A correct raw for a 1:1 coding source must never be touched, even
    /// though its source file is still present.
    #[test]
    fn one_file_per_session_raws_are_left_alone() {
        let dir = tmp_dir("leave_1to1_alone");
        let source = dir.join("session.jsonl");
        fs::write(&source, "{\"role\":\"user\"}\n").unwrap();
        let rel = super::super::preserve_raw(&dir, "a", &source).unwrap();

        let mut entry = index_entry_json("a", &source.to_string_lossy(), &rel);
        entry["provider"] = serde_json::json!("claude_code");
        write_index(&dir, vec![entry]);

        let result = backfill_raw(&dir).unwrap();
        assert_eq!(result.already_had, 1);
        assert_eq!(result.repaired, 0);

        let _ = fs::remove_dir_all(&dir);
    }

    /// A session whose conversation is no longer in the export must be counted
    /// as unrecoverable rather than silently handed the whole export.
    #[test]
    fn multi_session_source_skips_ids_absent_from_the_export() {
        let dir = tmp_dir("absent_id");
        let export = dir.join("conversations.json");
        fs::write(&export, chatgpt_export("ALPHA-MARKER", "BETA-MARKER")).unwrap();
        let source = export.to_string_lossy().to_string();

        write_index(
            &dir,
            vec![import_entry_json("conv-deleted", &source, "chatgpt")],
        );

        let result = backfill_raw(&dir).unwrap();
        assert_eq!(result.recovered, 0);
        assert_eq!(result.source_missing, 1);
        assert!(super::super::read_raw(&dir, "conv-deleted").is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    fn write_index(dir: &Path, entries: Vec<serde_json::Value>) {
        let idx = serde_json::json!({ "sessions": entries });
        fs::write(
            dir.join("index.json"),
            serde_json::to_string_pretty(&idx).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn recovers_only_entries_whose_source_file_exists() {
        let dir = tmp_dir("mixed");

        // Recoverable: source file exists, no raw yet.
        let source_a = dir.join("a.jsonl");
        fs::write(&source_a, "{\"role\":\"user\"}\n").unwrap();

        // Source gone: source_jsonl points at a path that no longer exists.
        let source_b = dir.join("b.jsonl");

        // Already had: raw_path set and the .zst actually present on disk.
        let rel_c = super::super::preserve_raw(&dir, "c", &{
            let source_c = dir.join("c.jsonl");
            fs::write(&source_c, "{\"role\":\"user\"}\n").unwrap();
            source_c
        })
        .unwrap();

        write_index(
            &dir,
            vec![
                index_entry_json("a", &source_a.to_string_lossy(), ""),
                index_entry_json("b", &source_b.to_string_lossy(), ""),
                index_entry_json("c", "unused-original-path.jsonl", &rel_c),
            ],
        );

        let result = backfill_raw(&dir).unwrap();
        assert_eq!(result.recovered, 1);
        assert_eq!(result.source_missing, 1);
        assert_eq!(result.already_had, 1);
        assert_eq!(result.total, 3);

        let entries = super::super::read_sessions(&dir);
        let recovered = entries.iter().find(|e| e.id == "a").unwrap();
        assert!(!recovered.raw_path.is_empty());
        assert!(dir.join(&recovered.raw_path).is_file());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn recovered_entry_persists_raw_path_via_set_raw_path() {
        let dir = tmp_dir("persist");
        let source = dir.join("session.jsonl");
        fs::write(&source, "{\"role\":\"assistant\"}\n").unwrap();

        write_index(
            &dir,
            vec![index_entry_json("s1", &source.to_string_lossy(), "")],
        );

        let result = backfill_raw(&dir).unwrap();
        assert_eq!(result.recovered, 1);
        assert_eq!(result.total, 1);

        // Re-read from disk (not in-memory) to prove the index write stuck.
        let entries = super::super::read_sessions(&dir);
        let entry = entries.iter().find(|e| e.id == "s1").unwrap();
        assert_eq!(entry.raw_path, "raw/s1.jsonl.zst");
        assert!(dir.join(&entry.raw_path).is_file());

        let _ = fs::remove_dir_all(&dir);
    }
}
