// HalluScribe - reader for the Ollama desktop app's local SQLite chat history.
// DB path: %LOCALAPPDATA%\Ollama\db.sqlite (Windows); see scanner for auto-detection.
// Opened read-only — safe while the Ollama process has the file open.

use super::{build_session, ChatProvider, MessageRole, ParsedMessage, ParsedSession, ReaderError};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

pub fn read(path: &Path) -> Result<Vec<ParsedSession>, ReaderError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| ReaderError::Database(e.to_string()))?;

    let mut stmt = conn
        .prepare("SELECT id, title, created_at FROM chats ORDER BY created_at ASC")
        .map_err(|e| ReaderError::Database(e.to_string()))?;

    let chats: Vec<(String, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| ReaderError::Database(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    let mut sessions = Vec::new();

    for (chat_id, raw_title, created_raw) in chats {
        let (messages, raw_rows) = load_messages(&conn, &chat_id)?;
        if messages.is_empty() {
            continue;
        }

        let created_at = parse_ollama_ts(&created_raw).unwrap_or_else(Utc::now);

        let updated_at = messages.iter().filter_map(|m| m.timestamp).max();

        // Ollama stores all chats untitled — derive title from first user message.
        let title = if raw_title.trim().is_empty() {
            messages
                .iter()
                .find(|m| matches!(m.role, MessageRole::User))
                .map(|m| truncate(&m.text, 80))
                .unwrap_or_else(|| "Ollama conversation".to_string())
        } else {
            raw_title.trim().to_string()
        };

        let total_chars: usize = messages.iter().map(|m| m.text.len()).sum();
        let fill_pct = ((total_chars as f64 / 600_000.0) * 100.0).clamp(0.0, 100.0);

        // This chat's own rows, not the whole database — the raw preserved for
        // a session must be that session alone.
        let raw_slice = serde_json::to_string_pretty(&serde_json::json!({
            "chat": {
                "id": &chat_id,
                "title": &raw_title,
                "created_at": &created_raw,
            },
            "messages": raw_rows,
        }))
        .unwrap_or_default();

        if let Some(session) = build_session(
            chat_id,
            title,
            created_at,
            updated_at,
            path.to_path_buf(),
            ChatProvider::OllamaChat,
            fill_pct,
            true,
            messages,
        ) {
            sessions.push(session.with_raw_slice(raw_slice));
        }
    }

    Ok(sessions)
}

/// Messages for one chat, plus the verbatim DB rows they were built from —
/// the latter becomes that session's raw slice, so a raw is this chat's rows
/// rather than a copy of the whole database.
type ChatRows = (Vec<ParsedMessage>, Vec<serde_json::Value>);

fn load_messages(conn: &Connection, chat_id: &str) -> Result<ChatRows, ReaderError> {
    let mut stmt = conn
        .prepare(
            "SELECT role, content, thinking, created_at \
             FROM messages WHERE chat_id = ?1 ORDER BY created_at ASC",
        )
        .map_err(|e| ReaderError::Database(e.to_string()))?;

    let rows: Vec<(String, String, String, String)> = stmt
        .query_map([chat_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2).unwrap_or_default(),
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| ReaderError::Database(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    let raw_rows = rows
        .iter()
        .map(|(role, content, thinking, created_at)| {
            serde_json::json!({
                "role": role,
                "content": content,
                "thinking": thinking,
                "created_at": created_at,
            })
        })
        .collect();

    let mut messages = Vec::new();
    for (role_str, content, thinking, ts_raw) in rows {
        let role = match role_str.as_str() {
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            _ => continue,
        };

        let mut text = content.trim().to_string();
        let thinking = thinking.trim();
        if !thinking.is_empty() {
            if text.is_empty() {
                text = format!("[Thinking]\n{thinking}");
            } else {
                text = format!("[Thinking]\n{thinking}\n\n{text}");
            }
        }
        if text.is_empty() {
            continue;
        }

        messages.push(ParsedMessage {
            role,
            text,
            timestamp: parse_ollama_ts(&ts_raw),
            speaker: None,
        });
    }

    Ok((messages, raw_rows))
}

/// Ollama timestamps use a space separator: "2026-01-10 10:58:41.514797+02:00".
/// Normalise to RFC3339 ("T" separator) before parsing.
fn parse_ollama_ts(s: &str) -> Option<DateTime<Utc>> {
    let normalised = s.trim().replacen(' ', "T", 1);
    DateTime::parse_from_rfc3339(&normalised)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn truncate(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(max_chars).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::{parse_ollama_ts, read, truncate};
    use crate::readers::ChatProvider;
    use rusqlite::Connection;
    use std::fs;

    #[test]
    fn reads_a_chat_into_a_session_with_golden_transcript() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("ollama.db");
        let conn = Connection::open(&db).expect("open db");
        conn.execute_batch(
            "CREATE TABLE chats (id TEXT PRIMARY KEY, title TEXT, created_at TEXT, updated_at TEXT);
             CREATE TABLE messages (chat_id TEXT, role TEXT, content TEXT, thinking TEXT, created_at TEXT);
             INSERT INTO chats VALUES ('c1', 'Ollama chat', '2026-01-10T08:58:41Z', '2026-01-10T09:00:00Z');
             INSERT INTO messages VALUES ('c1', 'user', 'hello there', '', '2026-01-10T08:58:41Z');
             INSERT INTO messages VALUES ('c1', 'assistant', 'hi, how can I help', '', '2026-01-10T08:59:00Z');",
        )
        .expect("build db");
        drop(conn);

        let sessions = read(&db).expect("read ollama db");
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.provider, ChatProvider::OllamaChat);
        // Golden transcript: role labels, exactly as before the business-messaging work.
        assert_eq!(
            session.transcript(),
            "[User]\nhello there\n\n[Assistant]\nhi, how can I help"
        );
        let _ = fs::remove_dir_all(dir.path());
    }

    #[test]
    fn parses_space_separated_timestamp_with_offset() {
        let ts = parse_ollama_ts("2026-01-10 10:58:41.514797+02:00");
        assert!(
            ts.is_some(),
            "should parse Ollama space-separated timestamp"
        );
        let dt = ts.unwrap();
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-01-10 08:58:41"
        );
    }

    #[test]
    fn parses_high_precision_fractional_seconds() {
        let ts = parse_ollama_ts("2026-04-22 19:09:24.5410598+03:00");
        assert!(ts.is_some(), "should parse 7-digit fractional seconds");
    }

    #[test]
    fn returns_none_for_garbage_timestamp() {
        assert!(parse_ollama_ts("not-a-date").is_none());
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 80), "hello");
    }

    #[test]
    fn truncate_long_string_appends_ellipsis() {
        let long: String = "a".repeat(100);
        let result = truncate(&long, 80);
        assert!(result.ends_with('…'));
        assert!(result.chars().count() <= 82);
    }
}
