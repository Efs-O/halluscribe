// HalluScribe - reads a Viber `Contacts.data` into structured rows.
//
// Phase 7b of the Business Messaging ingestion plan: the *data* layer only. It
// reads the raw `ZVIBERMESSAGE` rows, skips the rows that are not usable text
// (system/call events, media, unknown state, orphans without a conversation,
// bad dates), and hands back a `ViberDb` sorted by (conversation, date, rowid).
// It reuses the SMS reader's `RawMessage` / `Conversation` / `ConversationKey`
// so the windowing and session assembly are shared; it has its own `SkipCounts`
// (Viber's skip reasons differ from iMessage's and WhatsApp's). It does NOT
// render transcripts, build `ParsedSession`s, or touch the scanner - that is
// the reader module. The caller opens the temp copy read-only (Phase 1) and
// passes the connection in; this module never opens files, never `unwrap`s on
// row data, and never logs text, names or numbers.
//
// Schema (verified against the real backup, 2026-09-18): `ZVIBERMESSAGE` is
// keyed by `Z_PK`, carries `ZCONVERSATION` (its `ZCONVERSATION.Z_PK`),
// `ZDATE` (REAL, seconds since 2001-01-01 UTC), `ZTEXT`, `ZSTATE` (a TEXT enum
// that is the direction - see below), `ZPHONENUMINDEX` (the sender, a
// `ZPHONENUMBER.Z_PK`), `ZSYSTEMTYPE` (set on system/call rows), `ZATTACHMENT`
// and `ZGALLERYTYPE` (media). `ZCONVERSATION` carries `ZGROUPID` (set on
// groups) and `ZNAME`. `ZPHONENUMBER` maps `Z_PK` -> `ZPHONE`.
//
// Direction (verified by the supervisor from metadata only, 2026-09-19):
// `ZSTATE` is never NULL. `delivered` / `send` / `pendingNotSent` are the
// user's own (sent) messages; `received` is an incoming one. Any other value is
// unknown and is counted, not guessed.

use crate::readers::apple_messages_db::{Conversation, ConversationKey, RawMessage};
use crate::readers::ReaderError;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use std::collections::{BTreeSet, HashMap};

/// iOS core-data epoch: 2001-01-01 00:00:00 UTC, in unix seconds. Viber stores
/// `ZDATE` as seconds (a `REAL`) since this instant, exactly like WhatsApp.
const IOS_EPOCH_SECS: i64 = 978_307_200;

/// Rows skipped while loading, by reason. Every skipped or unreadable row is
/// counted here - nothing is dropped silently. `no_sender` is a keep-count:
/// those rows are retained (the speaker falls back), not skipped.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SkipCounts {
    /// `ZSYSTEMTYPE IS NOT NULL`: a system or call event, not a message.
    pub system: u64,
    /// Blank `ZTEXT` with `ZATTACHMENT` or `ZGALLERYTYPE != 0` (image, video, …).
    pub media: u64,
    /// A `ZSTATE` that is not one of the known direction values.
    pub unknown_state: u64,
    /// Blank `ZTEXT`, not media and not a system row.
    pub unusable_text: u64,
    /// A message with no `ZCONVERSATION`.
    pub orphan_no_session: u64,
    /// A `ZDATE` that cannot be converted to a date.
    pub bad_date: u64,
    /// Rows that failed to read (a column type mismatch, e.g. a NULL where a
    /// non-null type is required). Counted, not swallowed.
    pub row_errors: u64,
    /// A `received` row whose `ZPHONENUMINDEX` is NULL: kept, but the speaker
    /// falls back to the conversation name (1:1) or `Unknown` (group).
    pub no_sender: u64,
}

/// The loaded Viber database.
#[derive(Debug, Clone, PartialEq)]
pub struct ViberDb {
    pub conversations: Vec<Conversation>,
    /// Sorted by (conversation, date, rowid).
    pub messages: Vec<RawMessage>,
    pub skipped: SkipCounts,
}

/// Read the Viber tables from an open connection.
pub fn load(conn: &Connection) -> Result<ViberDb, ReaderError> {
    check_schema(conn)?;

    let conversations_meta = load_conversations(conn).map_err(db_err)?;
    let phones = load_phones(conn).map_err(db_err)?;
    let (mut messages, mut skipped) = load_messages(conn, &phones).map_err(db_err)?;
    // A message whose ZCONVERSATION row is gone (seen on a real backup) is an
    // orphan too: count it like one with no conversation at all, so
    // `build_conversations` only ever sees conversations that exist.
    let before = messages.len();
    messages.retain(|m| match m.conversation {
        ConversationKey::Chat(pk) => conversations_meta.contains_key(&pk),
        _ => true,
    });
    skipped.orphan_no_session += (before - messages.len()) as u64;
    let conversations = build_conversations(&messages, &conversations_meta);

    Ok(ViberDb {
        conversations,
        messages,
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
        ("ZVIBERMESSAGE", "Z_PK"),
        ("ZVIBERMESSAGE", "ZCONVERSATION"),
        ("ZVIBERMESSAGE", "ZDATE"),
        ("ZVIBERMESSAGE", "ZTEXT"),
        ("ZVIBERMESSAGE", "ZSTATE"),
        ("ZVIBERMESSAGE", "ZPHONENUMINDEX"),
        ("ZVIBERMESSAGE", "ZSYSTEMTYPE"),
        ("ZVIBERMESSAGE", "ZATTACHMENT"),
        ("ZVIBERMESSAGE", "ZGALLERYTYPE"),
        ("ZCONVERSATION", "Z_PK"),
        ("ZCONVERSATION", "ZNAME"),
        ("ZPHONENUMBER", "Z_PK"),
        ("ZPHONENUMBER", "ZPHONE"),
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
                "Viber schema not supported: missing {table}.{column}"
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

/// One `ZCONVERSATION` row: its display name. (Group-ness is not stored here:
/// the shared renderer detects a group from the participant count, exactly as
/// the WhatsApp reader does, so `ZGROUPID` is not read.)
struct ConversationRow {
    name: Option<String>,
}

/// `ZCONVERSATION.Z_PK` -> conversation row.
fn load_conversations(conn: &Connection) -> rusqlite::Result<HashMap<i64, ConversationRow>> {
    let mut out: HashMap<i64, ConversationRow> = HashMap::new();
    let mut stmt = conn.prepare("SELECT Z_PK, ZNAME FROM ZCONVERSATION")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
    })?;
    for row in rows {
        let (pk, name) = row?;
        out.insert(
            pk,
            ConversationRow {
                name: name.filter(|n| !n.trim().is_empty()),
            },
        );
    }
    Ok(out)
}

/// `ZPHONENUMBER.Z_PK` -> the phone number (the sender's number, for an
/// incoming message's `ZPHONENUMINDEX`).
fn load_phones(conn: &Connection) -> rusqlite::Result<HashMap<i64, String>> {
    let mut out: HashMap<i64, String> = HashMap::new();
    let mut stmt = conn.prepare("SELECT Z_PK, ZPHONE FROM ZPHONENUMBER")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
    })?;
    for row in rows {
        let (pk, phone) = row?;
        if let Some(phone) = phone.filter(|p| !p.trim().is_empty()) {
            out.insert(pk, phone);
        }
    }
    Ok(out)
}

/// A single `ZVIBERMESSAGE` row.
struct MessageRow {
    rowid: i64,
    conversation: Option<i64>,
    date: f64,
    text: Option<String>,
    state: Option<String>,
    phone_num_index: Option<i64>,
    system_type: Option<String>,
    attachment: Option<i64>,
    gallery_type: Option<i64>,
}

/// Read and resolve every message row, skipping the non-text rows and counting
/// each skip. Returns the kept messages (unsorted) plus the skip counts.
fn load_messages(
    conn: &Connection,
    phones: &HashMap<i64, String>,
) -> rusqlite::Result<(Vec<RawMessage>, SkipCounts)> {
    let mut messages: Vec<RawMessage> = Vec::new();
    let mut skipped = SkipCounts::default();

    let mut stmt = conn.prepare(
        "SELECT Z_PK, ZCONVERSATION, ZDATE, ZTEXT, ZSTATE, ZPHONENUMINDEX,
                ZSYSTEMTYPE, ZATTACHMENT, ZGALLERYTYPE
         FROM ZVIBERMESSAGE",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(MessageRow {
            rowid: r.get::<_, i64>(0)?,
            conversation: r.get::<_, Option<i64>>(1)?,
            date: r.get::<_, f64>(2)?,
            text: r.get::<_, Option<String>>(3)?,
            state: r.get::<_, Option<String>>(4)?,
            phone_num_index: r.get::<_, Option<i64>>(5)?,
            system_type: r.get::<_, Option<String>>(6)?,
            attachment: r.get::<_, Option<i64>>(7)?,
            gallery_type: r.get::<_, Option<i64>>(8)?,
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
        let Some(date_utc) = viber_date_to_utc(row.date) else {
            skipped.bad_date += 1;
            continue;
        };

        // Every Viber message belongs to a conversation; a missing one is an
        // orphan.
        let Some(conversation) = row.conversation else {
            skipped.orphan_no_session += 1;
            continue;
        };

        // System and call events are not messages.
        if row.system_type.is_some() {
            skipped.system += 1;
            continue;
        }

        // Direction from `ZSTATE` (never NULL in practice, read defensively).
        let state = row.state.as_deref().unwrap_or("");
        let is_from_me = matches!(state, "delivered" | "send" | "pendingNotSent");
        let is_incoming = state == "received";
        if !is_from_me && !is_incoming {
            skipped.unknown_state += 1;
            continue;
        }

        // A row is kept only if it has usable text. A media row (blank text
        // with an attachment or gallery type) is counted.
        let Some(text) = row.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) else {
            if row.attachment.unwrap_or(0) != 0 || row.gallery_type.unwrap_or(0) != 0 {
                skipped.media += 1;
            } else {
                skipped.unusable_text += 1;
            }
            continue;
        };

        // The sender's phone: `None` for my own messages; for an incoming one,
        // `ZPHONENUMINDEX` -> `ZPHONENUMBER.ZPHONE`. A received row with no
        // sender index is kept (the speaker falls back) and counted.
        let sender_handle = if is_from_me {
            None
        } else {
            match row
                .phone_num_index
                .and_then(|idx| phones.get(&idx).cloned())
            {
                Some(phone) => Some(phone),
                None => {
                    skipped.no_sender += 1;
                    None
                }
            }
        };

        messages.push(RawMessage {
            rowid: row.rowid,
            // Viber has no per-message guid; the rowid is the stable id.
            guid: format!("viber-{}", row.rowid),
            conversation: ConversationKey::Chat(conversation),
            date_utc,
            is_from_me,
            sender_handle,
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

/// Build one `Conversation` for each conversation present in the messages. The
/// participants are the distinct incoming sender phones (a 1:1 chat has one, a
/// group has several); only the display name comes from `ZCONVERSATION`.
fn build_conversations(
    messages: &[RawMessage],
    meta: &HashMap<i64, ConversationRow>,
) -> Vec<Conversation> {
    // Distinct incoming sender phones per conversation, in first-seen order.
    let mut participants: HashMap<i64, Vec<String>> = HashMap::new();
    for m in messages {
        if m.is_from_me {
            continue;
        }
        let ConversationKey::Chat(pk) = &m.conversation else {
            continue;
        };
        if let Some(phone) = &m.sender_handle {
            let list = participants.entry(*pk).or_default();
            if !list.contains(phone) {
                list.push(phone.clone());
            }
        }
    }

    let keys: BTreeSet<i64> = messages
        .iter()
        .filter_map(|m| match m.conversation {
            ConversationKey::Chat(pk) => Some(pk),
            _ => None,
        })
        .collect();

    let mut conversations: Vec<Conversation> = Vec::with_capacity(keys.len());
    for pk in keys {
        let row = &meta[&pk];
        let part = participants.get(&pk).cloned().unwrap_or_default();
        conversations.push(Conversation {
            key: ConversationKey::Chat(pk),
            chat_guid: None,
            display_name: row.name.clone(),
            service: None,
            participants: part,
        });
    }
    conversations
}

/// Convert a Viber `ZDATE` (seconds since 2001-01-01 UTC, stored as a `REAL`)
/// to a `DateTime<Utc>`. Returns `None` for an unconvertible value.
fn viber_date_to_utc(value: f64) -> Option<DateTime<Utc>> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let secs = value as i64;
    let nanos = ((value - value.trunc()) * 1_000_000_000.0) as u32;
    Utc.timestamp_opt(IOS_EPOCH_SECS + secs, nanos).single()
}

#[cfg(test)]
#[path = "viber_db_tests.rs"]
mod tests;
