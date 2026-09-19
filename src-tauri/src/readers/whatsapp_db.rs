// HalluScribe - reads a WhatsApp `ChatStorage.sqlite` into structured rows.
//
// Phase 7a of the Business Messaging ingestion plan: the *data* layer only. It
// reads the raw `ZWAMESSAGE` rows, skips the rows that are not usable text
// (media, group events, blank text, orphans without a session, bad dates), and
// hands back a `WhatsAppDb` sorted by (conversation, date, rowid). It reuses
// the SMS reader's `RawMessage` / `Conversation` / `ConversationKey` so the
// windowing and session assembly are shared; it has its own `SkipCounts`
// (WhatsApp's skip reasons differ from iMessage's). It does NOT render
// transcripts, build `ParsedSession`s, or touch the scanner - that is the
// reader module. The caller opens the temp copy read-only (Phase 1) and passes
// the connection in; this module never opens files, never `unwrap`s on row
// data, and never logs text, JIDs, names or numbers.
//
// Schema (verified against the real backup, 2026-09-18): `ZWAMESSAGE` is keyed
// by `Z_PK`, carries `ZCHATSESSION` (its `ZWACHATSESSION.Z_PK`), `ZISFROMME`,
// `ZMESSAGEDATE` (REAL, seconds since 2001-01-01 UTC), `ZTEXT`, `ZFROMJID`,
// `ZMESSAGETYPE` (0 = text) and `ZGROUPEVENTTYPE` (0 = none). `ZWAPROFILEPUSHNAME`
// is a `JID -> push_name` map (WhatsApp's own name for a contact); it is the
// first name source, ahead of the address book.

use crate::readers::apple_messages_db::{Conversation, ConversationKey, RawMessage};
use crate::readers::ReaderError;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use std::collections::{BTreeSet, HashMap};

/// iOS core-data epoch: 2001-01-01 00:00:00 UTC, in unix seconds. WhatsApp
/// stores `ZMESSAGEDATE` as seconds (a `REAL`) since this instant.
const IOS_EPOCH_SECS: i64 = 978_307_200;

/// Rows skipped while loading, by reason. Every skipped or unreadable row is
/// counted here - nothing is dropped silently.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SkipCounts {
    /// `ZMESSAGETYPE != 0` with no usable text (image, video, audio, document, …).
    pub media: u64,
    /// `ZGROUPEVENTTYPE != 0` with no usable text (member added, subject changed, …).
    pub group_events: u64,
    /// A `ZMESSAGETYPE == 0` (text) row whose `ZTEXT` is blank.
    pub unusable_text: u64,
    /// A message with no `ZCHATSESSION`.
    pub orphan_no_session: u64,
    /// A `ZMESSAGEDATE` that cannot be converted to a date.
    pub bad_date: u64,
    /// Rows that failed to read (a column type mismatch, e.g. a NULL where a
    /// non-null type is required). Counted, not swallowed.
    pub row_errors: u64,
}

/// The loaded WhatsApp database.
#[derive(Debug, Clone, PartialEq)]
pub struct WhatsAppDb {
    pub conversations: Vec<Conversation>,
    /// Sorted by (conversation, date, rowid).
    pub messages: Vec<RawMessage>,
    /// `ZWAPROFILEPUSHNAME`: JID -> WhatsApp's own push name for that contact.
    /// The first name source, ahead of the address book (Phase 7a contact rule).
    pub push_names: HashMap<String, String>,
    pub skipped: SkipCounts,
}

/// Read the WhatsApp tables from an open connection.
pub fn load(conn: &Connection) -> Result<WhatsAppDb, ReaderError> {
    check_schema(conn)?;

    let sessions = load_sessions(conn).map_err(db_err)?;
    let (messages, skipped) = load_messages(conn).map_err(db_err)?;
    let conversations = build_conversations(conn, &messages, &sessions).map_err(db_err)?;
    let push_names = load_push_names(conn).map_err(db_err)?;

    Ok(WhatsAppDb {
        conversations,
        messages,
        push_names,
        skipped,
    })
}

/// Map a SQLite error to a `ReaderError::Database`.
fn db_err(error: rusqlite::Error) -> ReaderError {
    ReaderError::Database(error.to_string())
}

/// Every table and column the reader depends on, in check order. The first
/// missing one is named in the error.
fn required_schema() -> Vec<(&'static str, &'static str)> {
    [
        ("ZWAMESSAGE", "Z_PK"),
        ("ZWAMESSAGE", "ZCHATSESSION"),
        ("ZWAMESSAGE", "ZISFROMME"),
        ("ZWAMESSAGE", "ZMESSAGEDATE"),
        ("ZWAMESSAGE", "ZTEXT"),
        ("ZWAMESSAGE", "ZFROMJID"),
        ("ZWAMESSAGE", "ZMESSAGETYPE"),
        ("ZWAMESSAGE", "ZGROUPEVENTTYPE"),
        ("ZWACHATSESSION", "Z_PK"),
        ("ZWACHATSESSION", "ZCONTACTJID"),
        ("ZWACHATSESSION", "ZPARTNERNAME"),
        ("ZWACHATSESSION", "ZSESSIONTYPE"),
        ("ZWAGROUPMEMBER", "ZCHATSESSION"),
        ("ZWAGROUPMEMBER", "ZMEMBERJID"),
        ("ZWAPROFILEPUSHNAME", "ZJID"),
        ("ZWAPROFILEPUSHNAME", "ZPUSHNAME"),
    ]
    .to_vec()
}

/// Verify the schema. Returns a `Database` error naming the first missing
/// table or column.
fn check_schema(conn: &Connection) -> Result<(), ReaderError> {
    for (table, column) in required_schema() {
        let has = table_has_column(conn, table, column).map_err(db_err)?;
        if !has {
            return Err(ReaderError::Database(format!(
                "WhatsApp schema not supported: missing {table}.{column}"
            )));
        }
    }
    Ok(())
}

/// `true` if `table` exists and has a column named `column`.
fn table_has_column(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let table = table.replace('\'', "''");
    let mut stmt = conn.prepare(&format!("PRAGMA table_info('{table}')"))?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
    for name in rows {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// One `ZWACHATSESSION` row: the contact JID, the partner name, and whether it
/// is a group (`ZSESSIONTYPE == 1`).
struct SessionRow {
    contact_jid: Option<String>,
    partner_name: Option<String>,
    is_group: bool,
}

/// `ZWACHATSESSION.Z_PK` -> session row.
fn load_sessions(conn: &Connection) -> rusqlite::Result<HashMap<i64, SessionRow>> {
    let mut sessions: HashMap<i64, SessionRow> = HashMap::new();
    let mut stmt =
        conn.prepare("SELECT Z_PK, ZCONTACTJID, ZPARTNERNAME, ZSESSIONTYPE FROM ZWACHATSESSION")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    for row in rows {
        let (pk, contact_jid, partner_name, session_type) = row?;
        sessions.insert(
            pk,
            SessionRow {
                contact_jid,
                partner_name,
                is_group: session_type == 1,
            },
        );
    }
    Ok(sessions)
}

/// A single `ZWAMESSAGE` row.
struct MessageRow {
    rowid: i64,
    chat_session: Option<i64>,
    is_from_me: bool,
    date: f64,
    message_type: i64,
    group_event_type: i64,
    text: Option<String>,
    from_jid: Option<String>,
}

/// Read and resolve every message row, skipping the non-text rows and counting
/// each skip. Returns the kept messages (unsorted) plus the skip counts.
fn load_messages(conn: &Connection) -> rusqlite::Result<(Vec<RawMessage>, SkipCounts)> {
    let mut messages: Vec<RawMessage> = Vec::new();
    let mut skipped = SkipCounts::default();

    let mut stmt = conn.prepare(
        "SELECT Z_PK, ZCHATSESSION, ZISFROMME, ZMESSAGEDATE, ZMESSAGETYPE, ZGROUPEVENTTYPE,
                ZTEXT, ZFROMJID
         FROM ZWAMESSAGE",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(MessageRow {
            rowid: r.get::<_, i64>(0)?,
            chat_session: r.get::<_, Option<i64>>(1)?,
            is_from_me: r.get::<_, i64>(2)? != 0,
            date: r.get::<_, f64>(3)?,
            message_type: r.get::<_, i64>(4)?,
            group_event_type: r.get::<_, i64>(5)?,
            text: r.get::<_, Option<String>>(6)?,
            from_jid: r.get::<_, Option<String>>(7)?,
        })
    })?;

    for row in rows {
        let row = match row {
            Ok(r) => r,
            // A row that fails to read (e.g. a NULL where a non-null type is
            // required) is counted, not swallowed.
            Err(_) => {
                skipped.row_errors += 1;
                continue;
            }
        };

        // Date: seconds (REAL) since 2001-01-01 UTC.
        let Some(date_utc) = wa_date_to_utc(row.date) else {
            skipped.bad_date += 1;
            continue;
        };

        // Every WhatsApp message belongs to a session; a missing one is an orphan.
        let Some(chat_session) = row.chat_session else {
            skipped.orphan_no_session += 1;
            continue;
        };

        // A row is kept only if it has usable text. A media or group-event row
        // with a caption keeps the caption; one without text is counted.
        let Some(text) = row.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) else {
            if row.group_event_type != 0 {
                skipped.group_events += 1;
            } else if row.message_type != 0 {
                skipped.media += 1;
            } else {
                skipped.unusable_text += 1;
            }
            continue;
        };

        // The sender's JID: `None` for my own messages; otherwise the message's
        // `ZFROMJID` (a 1:1 chat's sender is also the session's contact JID,
        // which the reader falls back to).
        let sender_jid = if row.is_from_me {
            None
        } else {
            row.from_jid.clone()
        };

        messages.push(RawMessage {
            rowid: row.rowid,
            // WhatsApp has no per-message guid; the rowid is the stable id.
            guid: format!("wa-{}", row.rowid),
            conversation: ConversationKey::Chat(chat_session),
            date_utc,
            is_from_me: row.is_from_me,
            sender_handle: sender_jid,
            service: None,
            text: Some(text.to_string()),
            attachments: Vec::new(),
        });
    }

    // Sort by (conversation, date, rowid).
    messages.sort_by(|a, b| {
        a.conversation
            .cmp(&b.conversation)
            .then_with(|| a.date_utc.cmp(&b.date_utc))
            .then_with(|| a.rowid.cmp(&b.rowid))
    });

    Ok((messages, skipped))
}

/// Build one `Conversation` for each session present in the messages.
fn build_conversations(
    conn: &Connection,
    messages: &[RawMessage],
    sessions: &HashMap<i64, SessionRow>,
) -> rusqlite::Result<Vec<Conversation>> {
    let keys: BTreeSet<i64> = messages
        .iter()
        .filter_map(|m| match m.conversation {
            ConversationKey::Chat(pk) => Some(pk),
            _ => None,
        })
        .collect();

    let mut conversations: Vec<Conversation> = Vec::with_capacity(keys.len());
    for pk in keys {
        let row = &sessions[&pk];
        let participants = if row.is_group {
            group_members(conn, pk)?
        } else {
            row.contact_jid.clone().into_iter().collect()
        };
        conversations.push(Conversation {
            key: ConversationKey::Chat(pk),
            chat_guid: row.contact_jid.clone(),
            display_name: row.partner_name.clone(),
            service: None,
            participants,
        });
    }
    Ok(conversations)
}

/// The member JIDs of a group session, via `ZWAGROUPMEMBER`.
fn group_members(conn: &Connection, chat_session: i64) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT ZMEMBERJID FROM ZWAGROUPMEMBER WHERE ZCHATSESSION = ?1")?;
    let rows = stmt.query_map([chat_session], |r| r.get::<_, Option<String>>(0))?;
    let mut out = Vec::new();
    for row in rows {
        if let Some(jid) = row? {
            if !jid.is_empty() {
                out.push(jid);
            }
        }
    }
    Ok(out)
}

/// `ZWAPROFILEPUSHNAME`: JID -> WhatsApp's own push name for that contact.
/// A blank name is dropped (nothing to show); a JID with two different names
/// keeps the first (the table is keyed by JID, so this is rare).
fn load_push_names(conn: &Connection) -> rusqlite::Result<HashMap<String, String>> {
    let mut push_names: HashMap<String, String> = HashMap::new();
    let mut stmt = conn.prepare("SELECT ZJID, ZPUSHNAME FROM ZWAPROFILEPUSHNAME")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, Option<String>>(1)?,
        ))
    })?;
    for row in rows {
        let (jid, name) = row?;
        let Some(jid) = jid.filter(|j| !j.trim().is_empty()) else {
            continue;
        };
        let Some(name) = name.filter(|n| !n.trim().is_empty()) else {
            continue;
        };
        push_names
            .entry(jid.trim().to_string())
            .or_insert_with(|| name.trim().to_string());
    }
    Ok(push_names)
}

/// Convert a WhatsApp `ZMESSAGEDATE` (seconds since 2001-01-01 UTC, stored as
/// a `REAL`) to a `DateTime<Utc>`. Returns `None` for an unconvertible value.
fn wa_date_to_utc(value: f64) -> Option<DateTime<Utc>> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let secs = value as i64;
    let nanos = ((value - value.trunc()) * 1_000_000_000.0) as u32;
    Utc.timestamp_opt(IOS_EPOCH_SECS + secs, nanos).single()
}

#[cfg(test)]
#[path = "whatsapp_db_tests.rs"]
mod tests;
