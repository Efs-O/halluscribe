// HalluScribe - unit/integration tests for Persona Pack export.

use super::*;
use chrono::TimeZone;
use std::io::Read;

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 5, 12, 0, 0).unwrap()
}

fn tmp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("halluscribe_pack_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn entry(id: &str, provider: &str, archive_path: &str, raw_path: &str) -> IndexEntry {
    IndexEntry {
        id: id.to_string(),
        project: "proj".to_string(),
        date: "2026-07-01".to_string(),
        title: format!("Session {id}"),
        tool: "Claude Code".to_string(),
        fill_pct: 80.0,
        session_timestamp: "2026-07-01T00:00:00+00:00".to_string(),
        updated_at: String::new(),
        session_type: "building".to_string(),
        error_tags: Vec::new(),
        topic_tags: Vec::new(),
        archive_path: archive_path.to_string(),
        source_jsonl: String::new(),
        source_size_bytes: 0,
        provider: provider.to_string(),
        fill_estimated: false,
        transcript_hash: String::new(),
        secret_flags: Vec::new(),
        raw_path: raw_path.to_string(),
    }
}

fn seed_archive(dir: &Path, entries: &[IndexEntry]) {
    let index = serde_json::json!({ "sessions": entries });
    std::fs::write(
        dir.join("index.json"),
        serde_json::to_string_pretty(&index).unwrap(),
    )
    .unwrap();
    for e in entries {
        let md = dir.join(&e.archive_path);
        std::fs::create_dir_all(md.parent().unwrap()).unwrap();
        std::fs::write(&md, format!("# {}\n\nbody", e.title)).unwrap();
    }
}

fn zip_names(path: &Path) -> Vec<String> {
    let file = std::fs::File::open(path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .collect()
}

fn zip_read(path: &Path, name: &str) -> Option<String> {
    let file = std::fs::File::open(path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut entry = archive.by_name(name).ok()?;
    let mut out = String::new();
    entry.read_to_string(&mut out).unwrap();
    Some(out)
}

#[test]
fn select_pack_entries_keeps_only_consented_providers() {
    let entries = vec![
        entry("a", "claude_code", "sessions/proj/a.md", ""),
        entry("b", "chatgpt", "sessions/proj/b.md", ""),
        entry("c", "codex", "sessions/proj/c.md", ""),
    ];
    let sources = vec!["claude_code".to_string(), "codex".to_string()];
    let kept = select_pack_entries(entries, &sources);
    let ids: Vec<&str> = kept.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["a", "c"]);
}

#[test]
fn default_pack_name_sanitises_user_and_dates() {
    assert_eq!(
        default_pack_name("Efso Office", fixed_now()),
        "efso-office-persona-2026-07-05.zip"
    );
    assert_eq!(
        default_pack_name("", fixed_now()),
        "user-persona-2026-07-05.zip"
    );
}

#[test]
fn fingerprint_is_deterministic_and_content_sensitive() {
    assert_eq!(fingerprint(b"hello"), fingerprint(b"hello"));
    assert_ne!(fingerprint(b"hello"), fingerprint(b"hallo"));
}

#[test]
fn export_excludes_personal_sessions_and_raw_by_default() {
    let dir = tmp_dir("default_export");
    let entries = vec![
        entry(
            "work1",
            "claude_code",
            "sessions/proj/work1.md",
            "raw/work1.jsonl.zst",
        ),
        entry("chat1", "chatgpt", "sessions/proj/chat1.md", ""),
    ];
    seed_archive(&dir, &entries);
    archive::preserve_raw(&dir, "work1", &dir.join("index.json")).unwrap();

    let dest = dir.join("out.zip");
    let summary = export_persona_pack(
        &dir,
        &dest,
        &["claude_code".to_string()],
        "# Work profile",
        &[("digest-2026-W27.md".to_string(), "week 27".to_string())],
        false,
        "0.2.1",
        "embeddinggemma-300m",
        fixed_now(),
    )
    .unwrap();

    assert_eq!(summary.session_count, 1);
    assert_eq!(summary.digest_count, 1);
    assert_eq!(summary.raw_count, 0);

    let names = zip_names(&dest);
    assert!(names.contains(&"profile.md".to_string()));
    assert!(names.contains(&"manifest.json".to_string()));
    assert!(names.contains(&"index.json".to_string()));
    assert!(names.contains(&"digest/digest-2026-W27.md".to_string()));
    assert!(names.contains(&"archive/sessions/proj/work1.md".to_string()));
    // Personal-provider session and its (absent) raw are excluded.
    assert!(!names.contains(&"archive/sessions/proj/chat1.md".to_string()));
    assert!(!names.iter().any(|n| n.starts_with("raw/")));

    // index.json holds only the exported session.
    let index = zip_read(&dest, "index.json").unwrap();
    assert!(index.contains("work1"));
    assert!(!index.contains("chat1"));
}

#[test]
fn export_includes_raw_when_opted_in() {
    let dir = tmp_dir("raw_export");
    let entries = vec![entry(
        "work1",
        "claude_code",
        "sessions/proj/work1.md",
        "raw/work1.jsonl.zst",
    )];
    seed_archive(&dir, &entries);
    archive::preserve_raw(&dir, "work1", &dir.join("index.json")).unwrap();

    let dest = dir.join("out.zip");
    let summary = export_persona_pack(
        &dir,
        &dest,
        &["claude_code".to_string()],
        "# Work profile",
        &[],
        true,
        "0.2.1",
        "",
        fixed_now(),
    )
    .unwrap();

    assert_eq!(summary.raw_count, 1);
    assert!(summary.includes_raw);
    assert!(zip_names(&dest).contains(&"raw/work1.jsonl.zst".to_string()));

    let manifest = zip_read(&dest, "manifest.json").unwrap();
    assert!(manifest.contains("\"includes_raw\": true"));
    assert!(manifest.contains("\"scope\": \"work\""));
}

#[test]
fn count_available_raw_matches_what_export_would_bundle() {
    let dir = tmp_dir("count_raw");
    let entries = vec![
        // Work session with a raw file actually on disk → counts.
        entry(
            "work1",
            "claude_code",
            "sessions/proj/work1.md",
            "raw/work1.jsonl.zst",
        ),
        // Work session whose raw_path points at a missing file → excluded.
        entry(
            "work2",
            "claude_code",
            "sessions/proj/work2.md",
            "raw/work2.jsonl.zst",
        ),
        // Work session with no raw preserved → excluded.
        entry("work3", "claude_code", "sessions/proj/work3.md", ""),
        // Personal-provider session with raw → excluded by consent filter.
        entry(
            "chat1",
            "chatgpt",
            "sessions/proj/chat1.md",
            "raw/chat1.jsonl.zst",
        ),
    ];
    seed_archive(&dir, &entries);
    // Only work1 gets its .zst written to disk.
    archive::preserve_raw(&dir, "work1", &dir.join("index.json")).unwrap();

    assert_eq!(count_available_raw(&dir, &["claude_code".to_string()]), 1);
}
