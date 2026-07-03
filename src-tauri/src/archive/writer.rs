// HalluScribe - markdown archive writer and archive-specific tests.

use super::index::append_index;
use super::redact::{apply_rules, rules_for_session};
use super::{ArchiveError, IndexEntry, SessionMeta};
use crate::gemma::{GemmaOutput, SessionType};
use chrono::{DateTime, Utc};
use std::fs;
use std::path::{Path, PathBuf};

pub fn write_session(
    archive_dir: &Path,
    meta: &SessionMeta,
    output: &GemmaOutput,
    now: DateTime<Utc>,
) -> Result<PathBuf, ArchiveError> {
    let sweep_date_str = now.format("%Y-%m-%d").to_string();
    let time_str = now.format("%H-%M-%S").to_string();
    let session_date_str = meta.session_timestamp.format("%Y-%m-%d").to_string();
    let slug = tool_slug(&meta.tool);

    let rel = PathBuf::from("sessions")
        .join(&meta.project)
        .join(&sweep_date_str)
        .join(format!("{time_str}-{slug}-sweep.md"));
    let abs = archive_dir.join(&rel);

    fs::create_dir_all(abs.parent().expect("path has parent"))?;

    let title = if output.title.is_empty() {
        format!(
            "{} - {session_date_str} {}",
            meta.project,
            meta.session_timestamp.format("%H:%M")
        )
    } else {
        output.title.clone()
    };

    let markdown = build_markdown(&title, meta, output, now);
    let rules = rules_for_session(archive_dir, &meta.id);
    let markdown = if rules.is_empty() {
        markdown
    } else {
        apply_rules(&markdown, &rules)
    };
    fs::write(&abs, markdown)?;

    append_index(
        archive_dir,
        IndexEntry {
            id: meta.id.clone(),
            project: meta.project.clone(),
            date: session_date_str,
            title,
            tool: meta.tool.clone(),
            fill_pct: meta.fill_pct,
            session_timestamp: meta.session_timestamp.to_rfc3339(),
            updated_at: meta
                .updated_at
                .map(|date| date.to_rfc3339())
                .unwrap_or_default(),
            session_type: session_type_str(&output.session_type),
            error_tags: output.error_tags.clone(),
            topic_tags: output.topic_tags.clone(),
            archive_path: rel.to_string_lossy().replace('\\', "/"),
            source_jsonl: meta.source.to_string_lossy().into_owned(),
            source_size_bytes: fs::metadata(&meta.source).map(|m| m.len()).unwrap_or(0),
            provider: meta.provider.clone(),
            fill_estimated: meta.fill_estimated,
            transcript_hash: meta.transcript_hash.clone(),
        },
    )?;

    Ok(abs)
}

fn build_markdown(
    title: &str,
    meta: &SessionMeta,
    output: &GemmaOutput,
    now: DateTime<Utc>,
) -> String {
    format!(
        "# {title}\n\n\
         **Tool:** {tool}  \n\
         **Provider:** {provider}  \n\
         **Date created:** {session_dt}  \n\
         {updated_line}\
         **Archived:** {archived_dt}  \n\
         **Fill at compact:** {fill_label}  \n\
         **Source:** {source}  \n\
         \n---\n\n\
         {summary}\n\n\
         ---\n\
         *Archived by HalluScribe - Gemma 4 26B via {backend}*\n",
        tool = meta.tool,
        provider = meta.provider,
        session_dt = meta.session_timestamp.format("%Y-%m-%d %H:%M UTC"),
        updated_line = meta
            .updated_at
            .map(|date| format!(
                "**Last modified:** {}  \n",
                date.format("%Y-%m-%d %H:%M UTC")
            ))
            .unwrap_or_default(),
        archived_dt = now.format("%Y-%m-%d %H:%M UTC"),
        fill_label = if meta.fill_estimated {
            format!("~{:.1}%", meta.fill_pct)
        } else {
            format!("{:.1}%", meta.fill_pct)
        },
        source = meta.source.display(),
        summary = output.summary,
        backend = meta.backend,
    )
}

fn tool_slug(tool: &str) -> &str {
    let t = tool.to_lowercase();
    if t.contains("claude") {
        "claudecode"
    } else if t.contains("codex") {
        "codex"
    } else if t.contains("forge") {
        "forge"
    } else if t.contains("chatgpt") {
        "chatgpt"
    } else if t.contains("gemma") {
        "gemma4"
    } else if t.contains("gemini") {
        "gemini"
    } else if t.contains("claude.ai") || t == "claude" {
        "claude"
    } else {
        "continue"
    }
}

fn session_type_str(t: &SessionType) -> String {
    match t {
        SessionType::Debugging => "debugging",
        SessionType::Building => "building",
        SessionType::Refactoring => "refactoring",
        SessionType::Exploration => "exploration",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{
        archived_source_size, delete_sessions, is_archived, read_sessions, session_id,
    };
    use chrono::TimeZone;

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 15, 14, 30, 0).unwrap()
    }

    fn sample_meta(source: &Path) -> SessionMeta {
        SessionMeta {
            id: session_id(source),
            source: source.to_path_buf(),
            project: "my-project".into(),
            tool: "Claude Code".into(),
            provider: "claude_code".into(),
            fill_pct: 78.5,
            fill_estimated: false,
            backend: "llama.cpp".into(),
            session_timestamp: fixed_now(),
            updated_at: None,
            transcript_hash: "abc123".into(),
        }
    }

    fn sample_output() -> GemmaOutput {
        GemmaOutput {
            title: "Fix JWT bug".into(),
            summary: "The session fixed a JWT expiry issue.".into(),
            session_type: SessionType::Debugging,
            error_tags: vec!["JWT".into()],
            topic_tags: vec!["auth".into(), "Rust".into()],
        }
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("halluscribe_test_{name}"));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn session_id_uses_file_stem() {
        let p = Path::new("/foo/bar/abc-123-def.jsonl");
        assert_eq!(crate::archive::session_id(p), "abc-123-def");
    }

    #[test]
    fn is_archived_false_when_no_index() {
        let dir = tmp_dir("no_index");
        assert!(!is_archived(&dir, "some-id"));
    }

    #[test]
    fn write_session_creates_markdown_file() {
        let dir = tmp_dir("write_md");
        let src = Path::new("/fake/project/d4632e2c-dead-beef.jsonl");
        let meta = sample_meta(src);
        let out = sample_output();

        let path = write_session(&dir, &meta, &out, fixed_now()).unwrap();
        assert!(path.exists());

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("# Fix JWT bug"));
        assert!(content.contains("**Tool:** Claude Code"));
        assert!(content.contains("**Provider:** claude_code"));
        assert!(content.contains("78.5%"));
        assert!(content.contains("llama.cpp"));
        assert!(content.contains("The session fixed a JWT expiry issue."));
    }

    #[test]
    fn write_session_path_uses_date_and_tool_slug() {
        let dir = tmp_dir("path_check");
        let src = Path::new("/fake/d4632e2c-dead-beef.jsonl");
        let meta = sample_meta(src);
        let path = write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();
        let s = path.to_string_lossy();
        assert!(s.contains("2026-04-15"));
        assert!(s.contains("14-30-00"));
        assert!(s.contains("claudecode"));
        assert!(s.contains("sweep"));
    }

    #[test]
    fn write_session_updates_index() {
        let dir = tmp_dir("index_check");
        let src = Path::new("/fake/abc-uuid.jsonl");
        let meta = sample_meta(src);
        write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();

        let sessions = read_sessions(&dir);
        assert_eq!(sessions.len(), 1);
        let e = &sessions[0];
        assert_eq!(e.id, "abc-uuid");
        assert_eq!(e.title, "Fix JWT bug");
        assert_eq!(e.session_type, "debugging");
        assert_eq!(e.error_tags, vec!["JWT"]);
        assert_eq!(e.tool, "Claude Code");
        assert_eq!(e.session_timestamp, "2026-04-15T14:30:00+00:00");
    }

    #[test]
    fn write_session_is_idempotent() {
        let dir = tmp_dir("idempotent");
        let src = Path::new("/fake/abc-uuid.jsonl");
        let meta = sample_meta(src);
        write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();
        write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();

        let sessions = read_sessions(&dir);
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn is_archived_true_after_write() {
        let dir = tmp_dir("is_archived");
        let src = Path::new("/fake/my-session-id.jsonl");
        let meta = sample_meta(src);
        assert!(!is_archived(&dir, "my-session-id"));
        write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();
        assert!(is_archived(&dir, "my-session-id"));
    }

    #[test]
    fn fallback_title_when_output_title_empty() {
        let dir = tmp_dir("fallback_title");
        let src = Path::new("/fake/uuid.jsonl");
        let meta = sample_meta(src);
        let mut out = sample_output();
        out.title = String::new();
        let path = write_session(&dir, &meta, &out, fixed_now()).unwrap();
        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("my-project"));
        assert!(content.contains("2026-04-15"));
    }

    #[test]
    fn tool_slug_variants() {
        assert_eq!(tool_slug("Claude Code"), "claudecode");
        assert_eq!(tool_slug("Codex"), "codex");
        assert_eq!(tool_slug("Forge"), "forge");
        assert_eq!(tool_slug("Gemma 4"), "gemma4");
        assert_eq!(tool_slug("Continue"), "continue");
        assert_eq!(tool_slug("Unknown"), "continue");
    }

    #[test]
    fn session_type_str_all_variants() {
        assert_eq!(session_type_str(&SessionType::Debugging), "debugging");
        assert_eq!(session_type_str(&SessionType::Building), "building");
        assert_eq!(session_type_str(&SessionType::Refactoring), "refactoring");
        assert_eq!(session_type_str(&SessionType::Exploration), "exploration");
    }

    #[test]
    fn delete_sessions_removes_file_and_index_entry() {
        let dir = tmp_dir("delete_sessions");
        let src = Path::new("/fake/to-delete.jsonl");
        let meta = sample_meta(src);
        let path = write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();
        assert!(path.exists());
        let deleted = delete_sessions(&dir, &["to-delete".to_string()]).unwrap();
        assert_eq!(deleted, vec!["to-delete"]);
        assert!(!path.exists());
        assert!(!is_archived(&dir, "to-delete"));
    }

    #[test]
    fn delete_sessions_ignores_unknown_ids() {
        let dir = tmp_dir("delete_unknown");
        let deleted = delete_sessions(&dir, &["nonexistent".to_string()]).unwrap();
        assert!(deleted.is_empty());
    }

    #[test]
    fn archived_source_size_none_when_absent() {
        let dir = tmp_dir("arc_size_absent");
        assert!(archived_source_size(&dir, "no-such-id").is_none());
    }

    #[test]
    fn archived_source_size_stored_and_retrieved() {
        let dir = tmp_dir("arc_size_stored");
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("session.jsonl");
        fs::write(&src, b"hello world").unwrap();
        let meta = SessionMeta {
            id: session_id(&src),
            source: src.clone(),
            project: "p".into(),
            tool: "Claude Code".into(),
            provider: "claude_code".into(),
            fill_pct: 50.0,
            fill_estimated: false,
            backend: "Ollama".into(),
            session_timestamp: fixed_now(),
            updated_at: None,
            transcript_hash: "hash".into(),
        };
        write_session(&dir, &meta, &sample_output(), fixed_now()).unwrap();
        let stored = archived_source_size(&dir, "session");
        assert!(stored.is_some());
        assert_eq!(stored.unwrap(), 11);
    }
}
