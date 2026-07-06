// HalluScribe - one-time raw transcript backfill (Persona Parity Phase A).
//
// Raw preservation (`archive::raw`) only ever runs during a sweep, so the
// ~1300 sessions archived before `preserve_raw_transcripts` existed have no
// raw copy. This is a non-destructive, no-inference recovery pass: for every
// already-archived session missing a raw copy, if its original
// `source_jsonl` still exists on disk, compress and record it exactly as the
// sweep would have. Sessions whose source tool already pruned the file are
// counted, never treated as an error - the backfill always completes.

use super::ArchiveError;
use std::path::Path;

/// Outcome of one backfill pass, returned to the UI.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct BackfillResult {
    pub recovered: usize,
    pub already_had: usize,
    pub source_missing: usize,
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

    for entry in entries {
        if !entry.raw_path.is_empty() && archive_dir.join(&entry.raw_path).is_file() {
            result.already_had += 1;
            continue;
        }

        let source = Path::new(&entry.source_jsonl);
        if entry.source_jsonl.is_empty() || !source.is_file() {
            result.source_missing += 1;
            continue;
        }

        match super::preserve_raw(archive_dir, &entry.id, source) {
            Ok(rel) => {
                super::set_raw_path(archive_dir, &entry.id, rel)?;
                result.recovered += 1;
            }
            Err(_) => result.source_missing += 1,
        }
    }

    Ok(result)
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
