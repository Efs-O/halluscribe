// HalluScribe - reads an iPhone `sms.db` (Messages) into structured rows.
//
// Phase 5a of the Business Messaging ingestion plan: this is the *data* layer
// only. It reads the raw `message`/`chat`/`handle`/`attachment` rows, skips the
// rows that are not real messages (tapbacks, group events, unusable text,
// orphans without a handle, bad dates), and hands back a `MessagesDb` sorted by
// (conversation, date, rowid). It does NOT render transcripts, build
// `ParsedSession`s, touch the scanner or settings - that is Phase 5b.
//
// The caller opens the temp copy read-only (Phase 1) and passes the connection
// in; this module never opens files. It streams rows with `query_map`, never
// `unwrap`s on row data, and never logs text, handles or names.

use crate::readers::ReaderError;
use chrono::{DateTime, Utc};
use rows::{ios_date_to_utc, load_attachments, resolve_text, ATTACHMENTS_SQL};
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};

/// The conversation a message belongs to: a `chat` row, or (for the ~10% of
/// messages with no `chat_message_join` row) a single `handle`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConversationKey {
    /// `chat.ROWID`.
    Chat(i64),
    /// `handle.ROWID`, for a message with no chat join.
    OrphanHandle(i64),
}

/// One attachment on a message. `transfer_name` is the user-visible filename;
/// the raw `attachment.filename` is a device path and is deliberately not kept.
#[derive(Debug, Clone, PartialEq)]
pub struct RawAttachment {
    pub transfer_name: Option<String>,
    pub mime_type: Option<String>,
    pub total_bytes: Option<i64>,
}

/// One real (non-tapback, non-group-event) message.
#[derive(Debug, Clone, PartialEq)]
pub struct RawMessage {
    pub rowid: i64,
    pub guid: String,
    pub conversation: ConversationKey,
    pub date_utc: DateTime<Utc>,
    pub is_from_me: bool,
    /// The sender's `handle.id` when resolvable; `None` for my own messages and
    /// for cases Phase 5b renders as unresolved.
    pub sender_handle: Option<String>,
    pub service: Option<String>,
    pub text: Option<String>,
    pub attachments: Vec<RawAttachment>,
}

/// One conversation: a `chat` or a single orphan `handle`.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversation {
    pub key: ConversationKey,
    pub chat_guid: Option<String>,
    pub display_name: Option<String>,
    pub service: Option<String>,
    /// `handle.id` of every participant, via `chat_handle_join`. For an
    /// `OrphanHandle` this is that one handle.
    pub participants: Vec<String>,
}

/// Rows skipped while loading, by reason. Every skipped or unreadable row is
/// counted here - nothing is dropped silently.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SkipCounts {
    pub tapbacks: u64,
    pub group_events: u64,
    pub unusable_text: u64,
    pub orphan_no_handle: u64,
    pub bad_date: u64,
    /// Rows that failed to read (a column type mismatch, e.g. a NULL where a
    /// non-null type is required). Counted, not swallowed.
    pub row_errors: u64,
    /// Extra `chat_message_join` rows for a message already loaded (a message
    /// joined to more than one chat is kept once, in its first chat).
    pub duplicate_joins: u64,
    /// Attachment rows that failed to read. The message is kept without them.
    pub attachment_errors: u64,
}

/// The loaded Messages database.
#[derive(Debug, Clone, PartialEq)]
pub struct MessagesDb {
    pub conversations: Vec<Conversation>,
    /// Sorted by (conversation, date, rowid).
    pub messages: Vec<RawMessage>,
    pub skipped: SkipCounts,
}

/// Read the Messages tables from an open connection.
pub fn load(conn: &Connection) -> Result<MessagesDb, ReaderError> {
    check_schema(conn)?;

    let handles = load_handles(conn).map_err(db_err)?;
    let (messages, skipped) = load_messages(conn, &handles).map_err(db_err)?;
    let conversations = build_conversations(conn, &messages).map_err(db_err)?;

    Ok(MessagesDb {
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
        ("message", "ROWID"),
        ("message", "guid"),
        ("message", "text"),
        ("message", "attributedBody"),
        ("message", "handle_id"),
        ("message", "date"),
        ("message", "is_from_me"),
        ("message", "service"),
        ("message", "associated_message_type"),
        ("message", "item_type"),
        ("handle", "ROWID"),
        ("handle", "id"),
        ("handle", "service"),
        ("chat", "ROWID"),
        ("chat", "guid"),
        ("chat", "chat_identifier"),
        ("chat", "display_name"),
        ("chat", "service_name"),
        ("chat_message_join", "chat_id"),
        ("chat_message_join", "message_id"),
        ("chat_handle_join", "chat_id"),
        ("chat_handle_join", "handle_id"),
        ("attachment", "ROWID"),
        ("attachment", "filename"),
        ("attachment", "mime_type"),
        ("attachment", "transfer_name"),
        ("attachment", "total_bytes"),
        ("message_attachment_join", "message_id"),
        ("message_attachment_join", "attachment_id"),
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
                "Messages schema not supported: missing {table}.{column}"
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

/// `handle.ROWID` -> `handle.id`.
fn load_handles(conn: &Connection) -> rusqlite::Result<BTreeMap<i64, String>> {
    let mut handles: BTreeMap<i64, String> = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT ROWID, id FROM handle")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (rowid, id) = row?;
        handles.insert(rowid, id);
    }
    Ok(handles)
}

/// A single message row plus the data needed to resolve it.
struct MessageRow {
    rowid: i64,
    guid: String,
    chat_id: Option<i64>,
    handle_id: i64,
    date: i64,
    is_from_me: bool,
    service: Option<String>,
    text: Option<String>,
    attributed_body: Option<Vec<u8>>,
    associated_message_type: i64,
    item_type: i64,
}

/// Read and resolve every message row, skipping the non-message rows and
/// counting each skip. Returns the kept messages (unsorted) plus the skip
/// counts.
fn load_messages(
    conn: &Connection,
    handles: &BTreeMap<i64, String>,
) -> rusqlite::Result<(Vec<RawMessage>, SkipCounts)> {
    let mut messages: Vec<RawMessage> = Vec::new();
    let mut skipped = SkipCounts::default();

    let mut stmt = conn.prepare(
        "SELECT m.ROWID, m.guid, j.chat_id, m.handle_id, m.date, m.is_from_me, m.service,
                m.text, m.attributedBody, m.associated_message_type, m.item_type
         FROM message m
         LEFT JOIN chat_message_join j ON j.message_id = m.ROWID
         ORDER BY m.ROWID, j.chat_id",
    )?;
    let mut attachment_stmt = conn.prepare(ATTACHMENTS_SQL)?;
    let mut last_rowid: Option<i64> = None;
    let rows = stmt.query_map([], |r| {
        Ok(MessageRow {
            rowid: r.get::<_, i64>(0)?,
            guid: r.get::<_, String>(1)?,
            chat_id: r.get::<_, Option<i64>>(2)?,
            handle_id: r.get::<_, i64>(3)?,
            date: r.get::<_, i64>(4)?,
            is_from_me: r.get::<_, i64>(5)? != 0,
            service: r.get::<_, Option<String>>(6)?,
            text: r.get::<_, Option<String>>(7)?,
            attributed_body: r.get::<_, Option<Vec<u8>>>(8)?,
            associated_message_type: r.get::<_, i64>(9)?,
            item_type: r.get::<_, i64>(10)?,
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

        // A message joined to several chats comes back once per join; keep
        // the first (lowest chat_id) and count the rest.
        if last_rowid == Some(row.rowid) {
            skipped.duplicate_joins += 1;
            continue;
        }
        last_rowid = Some(row.rowid);

        // Tapbacks and group events are not messages.
        if row.associated_message_type != 0 {
            skipped.tapbacks += 1;
            continue;
        }
        if row.item_type != 0 {
            skipped.group_events += 1;
            continue;
        }

        // Resolve the conversation key.
        let conversation = match row.chat_id {
            Some(chat_id) => ConversationKey::Chat(chat_id),
            None if row.handle_id > 0 => ConversationKey::OrphanHandle(row.handle_id),
            None => {
                skipped.orphan_no_handle += 1;
                continue;
            }
        };

        // Date: nanoseconds or seconds since 2001-01-01 UTC.
        let Some(date_utc) = ios_date_to_utc(row.date) else {
            skipped.bad_date += 1;
            continue;
        };

        // Text: the `text` column, else the decoded attributedBody.
        let text = resolve_text(&row.text, &row.attributed_body);
        let attachments = load_attachments(&mut attachment_stmt, row.rowid, &mut skipped);

        // No text and no attachments: unusable.
        if text.is_none() && attachments.is_empty() {
            skipped.unusable_text += 1;
            continue;
        }

        let sender_handle =
            resolve_sender_handle(&conversation, row.is_from_me, row.handle_id, handles, conn)?;

        messages.push(RawMessage {
            rowid: row.rowid,
            guid: row.guid,
            conversation,
            date_utc,
            is_from_me: row.is_from_me,
            sender_handle,
            service: row.service,
            text,
            attachments,
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

/// The `handle.id`s of a chat's participants, via `chat_handle_join`.
fn chat_participants(conn: &Connection, chat_id: i64) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT h.id FROM chat_handle_join j JOIN handle h ON h.ROWID = j.handle_id
         WHERE j.chat_id = ?1",
    )?;
    let rows = stmt.query_map([chat_id], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Build one `Conversation` for each conversation key present in the messages.
fn build_conversations(
    conn: &Connection,
    messages: &[RawMessage],
) -> rusqlite::Result<Vec<Conversation>> {
    let keys: BTreeSet<ConversationKey> = messages.iter().map(|m| m.conversation.clone()).collect();

    let mut conversations: Vec<Conversation> = Vec::with_capacity(keys.len());
    for key in keys {
        conversations.push(match key {
            ConversationKey::Chat(chat_id) => {
                let (chat_guid, display_name, service) = chat_details(conn, chat_id);
                let participants = chat_participants(conn, chat_id)?;
                Conversation {
                    key: ConversationKey::Chat(chat_id),
                    chat_guid,
                    display_name,
                    service,
                    participants,
                }
            }
            ConversationKey::OrphanHandle(handle_rowid) => {
                let participants = handle_lookup(conn, handle_rowid).into_iter().collect();
                Conversation {
                    key: ConversationKey::OrphanHandle(handle_rowid),
                    chat_guid: None,
                    display_name: None,
                    service: None,
                    participants,
                }
            }
        });
    }
    Ok(conversations)
}

/// `(guid, display_name, service_name)` for a chat row, or `None` fields if the
/// chat row is absent.
fn chat_details(
    conn: &Connection,
    chat_id: i64,
) -> (Option<String>, Option<String>, Option<String>) {
    conn.query_row(
        "SELECT guid, display_name, service_name FROM chat WHERE ROWID = ?1",
        [chat_id],
        |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        },
    )
    .unwrap_or((None, None, None))
}

/// The `handle.id` for a `handle.ROWID`, or `None` if absent.
fn handle_lookup(conn: &Connection, handle_id: i64) -> Option<String> {
    conn.query_row("SELECT id FROM handle WHERE ROWID = ?1", [handle_id], |r| {
        r.get::<_, String>(0)
    })
    .ok()
}

/// The sender's `handle.id`: `None` for my own messages; for a group chat, the
/// message's `handle_id`; for a 1:1 chat with no `handle_id`, the chat's single
/// participant; otherwise `None` (Phase 5b renders it unresolved).
fn resolve_sender_handle(
    conversation: &ConversationKey,
    is_from_me: bool,
    handle_id: i64,
    handles: &BTreeMap<i64, String>,
    conn: &Connection,
) -> rusqlite::Result<Option<String>> {
    if is_from_me {
        return Ok(None);
    }
    if handle_id > 0 {
        // A group chat (or any message that carries its own handle).
        return Ok(handles.get(&handle_id).cloned());
    }
    // A 1:1 chat with no handle: the sender is the chat's single participant.
    if let ConversationKey::Chat(chat_id) = conversation {
        let participants = chat_participants(conn, *chat_id)?;
        if participants.len() == 1 {
            return Ok(participants.into_iter().next());
        }
    }
    Ok(None)
}

#[path = "apple_messages_db_rows.rs"]
mod rows;

#[cfg(test)]
#[path = "apple_messages_db_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "apple_messages_db_join_tests.rs"]
mod join_tests;
