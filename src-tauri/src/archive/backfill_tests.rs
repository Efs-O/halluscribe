// HalluScribe - raw backfill tests: per-session slicing, in-place repair, and
// ownership-based source resolution (chat imports via settings, coding tools
// via the recorded path).

use super::backfill::{backfill_raw, backfill_raw_with};
use crate::settings::HalluScribeSettings;
use std::fs;
use std::path::Path;

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

fn write_index(dir: &Path, entries: Vec<serde_json::Value>) {
    let idx = serde_json::json!({ "sessions": entries });
    fs::write(
        dir.join("index.json"),
        serde_json::to_string_pretty(&idx).unwrap(),
    )
    .unwrap();
}

fn chatgpt_settings(import_dir: &Path) -> HalluScribeSettings {
    HalluScribeSettings {
        chatgpt_import_path: import_dir.to_string_lossy().into_owned(),
        ..Default::default()
    }
}

/// The headline regression: several sessions sharing one export file must
/// each recover their OWN conversation, not a copy of the whole export.
#[test]
fn multi_session_source_recovers_a_distinct_slice_per_session() {
    let dir = tmp_dir("multi_session");
    let imports = dir.join("imports");
    fs::create_dir_all(&imports).unwrap();
    let export = imports.join("conversations.json");
    fs::write(&export, chatgpt_export("ALPHA-MARKER", "BETA-MARKER")).unwrap();
    let source = export.to_string_lossy().to_string();

    write_index(
        &dir,
        vec![
            import_entry_json("conv-a", &source, "chatgpt"),
            import_entry_json("conv-b", &source, "chatgpt"),
        ],
    );

    let result = backfill_raw_with(&dir, &chatgpt_settings(&imports)).unwrap();
    assert_eq!(result.recovered, 2);
    assert_eq!(result.source_missing, 0);

    let entries = crate::archive::read_sessions(&dir);
    let raw_a = crate::archive::read_raw(&dir, "conv-a").expect("conv-a raw");
    let raw_b = crate::archive::read_raw(&dir, "conv-b").expect("conv-b raw");

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
    let imports = dir.join("imports");
    fs::create_dir_all(&imports).unwrap();
    let export = imports.join("conversations.json");
    fs::write(&export, chatgpt_export("ALPHA-MARKER", "BETA-MARKER")).unwrap();
    let source = export.to_string_lossy().to_string();

    // Reproduce the defect: both sessions get the whole export as raw.
    let rel_a = crate::archive::preserve_raw(&dir, "conv-a", &export).unwrap();
    let rel_b = crate::archive::preserve_raw(&dir, "conv-b", &export).unwrap();
    assert_eq!(
        crate::archive::read_raw(&dir, "conv-a").unwrap(),
        crate::archive::read_raw(&dir, "conv-b").unwrap(),
        "precondition: both raws are the same whole-export copy"
    );

    let mut entry_a = import_entry_json("conv-a", &source, "chatgpt");
    entry_a["raw_path"] = serde_json::json!(rel_a);
    let mut entry_b = import_entry_json("conv-b", &source, "chatgpt");
    entry_b["raw_path"] = serde_json::json!(rel_b);
    write_index(&dir, vec![entry_a, entry_b]);

    let settings = chatgpt_settings(&imports);
    let result = backfill_raw_with(&dir, &settings).unwrap();
    assert_eq!(result.repaired, 2, "both wrong raws must be replaced");
    assert_eq!(result.already_had, 0);
    assert_eq!(result.recovered, 0);

    let raw_a = crate::archive::read_raw(&dir, "conv-a").unwrap();
    let raw_b = crate::archive::read_raw(&dir, "conv-b").unwrap();
    assert!(raw_a.contains("ALPHA-MARKER") && !raw_a.contains("BETA-MARKER"));
    assert!(raw_b.contains("BETA-MARKER") && !raw_b.contains("ALPHA-MARKER"));

    // Idempotent: a second pass finds them correct and changes nothing.
    let again = backfill_raw_with(&dir, &settings).unwrap();
    assert_eq!(again.repaired, 0);
    assert_eq!(again.already_had, 2);

    let _ = fs::remove_dir_all(&dir);
}

/// The efso-145 case: the project folder moved, so every recorded
/// `source_jsonl` points at a path that no longer exists. The export is at the
/// configured import path, and that alone is what locates it.
#[test]
fn chat_import_recovers_from_the_configured_path_when_the_recorded_one_is_stale() {
    let dir = tmp_dir("stale_recorded_path");
    let imports = dir.join("imports").join("chatgpt");
    fs::create_dir_all(&imports).unwrap();
    fs::write(
        imports.join("conversations.json"),
        chatgpt_export("ALPHA-MARKER", "BETA-MARKER"),
    )
    .unwrap();

    // Recorded when the export lived somewhere that is now gone.
    let stale = dir
        .join("moved-away")
        .join("chatgpt chats")
        .join("conversations.json");
    let stale = stale.to_string_lossy().to_string();
    assert!(
        !Path::new(&stale).exists(),
        "precondition: recorded path is stale"
    );

    write_index(
        &dir,
        vec![
            import_entry_json("conv-a", &stale, "chatgpt"),
            import_entry_json("conv-b", &stale, "chatgpt"),
        ],
    );

    let result = backfill_raw_with(&dir, &chatgpt_settings(&imports)).unwrap();
    assert_eq!(result.recovered, 2);
    assert_eq!(result.source_missing, 0);
    assert!(crate::archive::read_raw(&dir, "conv-a")
        .unwrap()
        .contains("ALPHA-MARKER"));

    let _ = fs::remove_dir_all(&dir);
}

/// A stale coding-tool path is NOT rescued by any import setting: coding
/// sources belong to the tool that wrote them, and there is no second attempt.
#[test]
fn coding_provider_with_a_stale_path_is_not_rescued_by_import_settings() {
    let dir = tmp_dir("coding_not_rescued");
    let imports = dir.join("imports").join("chatgpt");
    fs::create_dir_all(&imports).unwrap();
    // A file with the same name sits in the import folder - it must be ignored.
    fs::write(imports.join("session.jsonl"), "{\"role\":\"user\"}\n").unwrap();

    let stale = dir.join("gone").join("session.jsonl");
    let mut entry = index_entry_json("s1", &stale.to_string_lossy(), "");
    entry["provider"] = serde_json::json!("claude_code");
    write_index(&dir, vec![entry]);

    let result = backfill_raw_with(&dir, &chatgpt_settings(&imports)).unwrap();
    assert_eq!(result.recovered, 0);
    assert_eq!(result.source_missing, 1);
    assert!(crate::archive::read_raw(&dir, "s1").is_err());

    let _ = fs::remove_dir_all(&dir);
}

/// A split export must resolve to the chunk the session actually came from,
/// not to whichever chunk the import folder lists first.
#[test]
fn split_export_resolves_to_the_recorded_chunk() {
    let dir = tmp_dir("split_export");
    let imports = dir.join("imports").join("chatgpt");
    fs::create_dir_all(&imports).unwrap();
    fs::write(
        imports.join("conversations-000.json"),
        chatgpt_export("CHUNK0-A", "CHUNK0-B"),
    )
    .unwrap();
    // Same conversation ids, different content: only the recorded chunk's
    // markers may end up in the recovered raw.
    fs::write(
        imports.join("conversations-003.json"),
        chatgpt_export("CHUNK3-A", "CHUNK3-B"),
    )
    .unwrap();

    let recorded = dir
        .join("elsewhere")
        .join("conversations-003.json")
        .to_string_lossy()
        .into_owned();
    write_index(
        &dir,
        vec![import_entry_json("conv-a", &recorded, "chatgpt")],
    );

    let result = backfill_raw_with(&dir, &chatgpt_settings(&imports)).unwrap();
    assert_eq!(result.recovered, 1);
    let raw = crate::archive::read_raw(&dir, "conv-a").unwrap();
    assert!(raw.contains("CHUNK3-A"), "resolved the wrong chunk");
    assert!(!raw.contains("CHUNK0-A"));

    let _ = fs::remove_dir_all(&dir);
}

/// With no import path configured there is nothing to resolve through, so a
/// chat import is honestly reported as missing rather than guessed at.
#[test]
fn chat_import_without_a_configured_path_is_reported_missing() {
    let dir = tmp_dir("no_import_path");
    let export = dir.join("conversations.json");
    fs::write(&export, chatgpt_export("ALPHA-MARKER", "BETA-MARKER")).unwrap();

    write_index(
        &dir,
        vec![import_entry_json(
            "conv-a",
            &export.to_string_lossy(),
            "chatgpt",
        )],
    );

    let result = backfill_raw_with(&dir, &HalluScribeSettings::default()).unwrap();
    assert_eq!(result.recovered, 0);
    assert_eq!(result.source_missing, 1);

    let _ = fs::remove_dir_all(&dir);
}

/// A correct raw for a 1:1 coding source must never be touched, even
/// though its source file is still present.
#[test]
fn one_file_per_session_raws_are_left_alone() {
    let dir = tmp_dir("leave_1to1_alone");
    let source = dir.join("session.jsonl");
    fs::write(&source, "{\"role\":\"user\"}\n").unwrap();
    let rel = crate::archive::preserve_raw(&dir, "a", &source).unwrap();

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
    let imports = dir.join("imports");
    fs::create_dir_all(&imports).unwrap();
    let export = imports.join("conversations.json");
    fs::write(&export, chatgpt_export("ALPHA-MARKER", "BETA-MARKER")).unwrap();

    write_index(
        &dir,
        vec![import_entry_json(
            "conv-deleted",
            &export.to_string_lossy(),
            "chatgpt",
        )],
    );

    let result = backfill_raw_with(&dir, &chatgpt_settings(&imports)).unwrap();
    assert_eq!(result.recovered, 0);
    assert_eq!(result.source_missing, 1);
    assert!(crate::archive::read_raw(&dir, "conv-deleted").is_err());

    let _ = fs::remove_dir_all(&dir);
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
    let rel_c = crate::archive::preserve_raw(&dir, "c", &{
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

    let entries = crate::archive::read_sessions(&dir);
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
    let entries = crate::archive::read_sessions(&dir);
    let entry = entries.iter().find(|e| e.id == "s1").unwrap();
    assert_eq!(entry.raw_path, "raw/s1.jsonl.zst");
    assert!(dir.join(&entry.raw_path).is_file());

    let _ = fs::remove_dir_all(&dir);
}
