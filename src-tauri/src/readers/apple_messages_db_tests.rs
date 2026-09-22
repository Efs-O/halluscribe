// HalluScribe - tests for the Messages DB layer. All fixtures are synthetic:
// an in-memory sms.db with invented guids and handles. No real data.

use super::load;
use crate::readers::apple_messages_db::{ConversationKey, MessagesDb};
use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection};

/// iOS core-data epoch: 2001-01-01 00:00:00 UTC, in unix seconds.
const IOS_EPOCH_SECS: i64 = 978_307_200;

/// The full sms.db schema the reader depends on.
const SCHEMA: &str = r#"
CREATE TABLE message (
    ROWID INTEGER PRIMARY KEY,
    guid TEXT,
    text TEXT,
    attributedBody BLOB,
    handle_id INTEGER,
    date INTEGER,
    is_from_me INTEGER,
    service TEXT,
    associated_message_type INTEGER,
    item_type INTEGER
);
CREATE TABLE handle (ROWID INTEGER PRIMARY KEY, id TEXT, service TEXT);
CREATE TABLE chat (
    ROWID INTEGER PRIMARY KEY,
    guid TEXT,
    chat_identifier TEXT,
    display_name TEXT,
    service_name TEXT
);
CREATE TABLE chat_message_join (chat_id INTEGER, message_id INTEGER);
CREATE TABLE chat_handle_join (chat_id INTEGER, handle_id INTEGER);
CREATE TABLE attachment (
    ROWID INTEGER PRIMARY KEY,
    filename TEXT,
    mime_type TEXT,
    transfer_name TEXT,
    total_bytes INTEGER
);
CREATE TABLE message_attachment_join (message_id INTEGER, attachment_id INTEGER);
"#;

fn conn() -> Connection {
    let c = Connection::open_in_memory().expect("in-memory db");
    c.execute_batch(SCHEMA).expect("create schema");
    c
}

/// A valid typedstream attributedBody blob (the BM-3 format), for short text.
fn attributed_blob(text: &str) -> Vec<u8> {
    let mut b = vec![
        0x04, 0x0B, b's', b't', b'r', b'e', b'a', b'm', b't', b'y', b'p', b'e', b'd',
    ];
    b.extend_from_slice(b"NSString");
    b.push(0x2B);
    let len = text.len();
    assert!(len < 0x80, "test only builds 1-byte-length blobs");
    b.push(len as u8);
    b.extend_from_slice(text.as_bytes());
    b
}

fn add_handle(c: &Connection, rowid: i64, id: &str) {
    c.execute(
        "INSERT INTO handle (ROWID, id, service) VALUES (?1, ?2, 'SMS')",
        params![rowid, id],
    )
    .expect("insert handle");
}

fn add_chat(c: &Connection, rowid: i64, guid: &str, display: Option<&str>) {
    c.execute(
        "INSERT INTO chat (ROWID, guid, chat_identifier, display_name, service_name)
         VALUES (?1, ?2, ?2, ?3, 'SMS')",
        params![rowid, guid, display],
    )
    .expect("insert chat");
}

fn join_chat_message(c: &Connection, chat_id: i64, message_id: i64) {
    c.execute(
        "INSERT INTO chat_message_join (chat_id, message_id) VALUES (?1, ?2)",
        params![chat_id, message_id],
    )
    .expect("join chat message");
}

fn join_chat_handle(c: &Connection, chat_id: i64, handle_id: i64) {
    c.execute(
        "INSERT INTO chat_handle_join (chat_id, handle_id) VALUES (?1, ?2)",
        params![chat_id, handle_id],
    )
    .expect("join chat handle");
}

/// A message row with sensible defaults; set the fields you care about.
struct Msg {
    rowid: i64,
    guid: &'static str,
    text: Option<&'static str>,
    attributed: Option<Vec<u8>>,
    handle_id: i64,
    date: i64,
    is_from_me: bool,
    associated: i64,
    item_type: i64,
}

impl Msg {
    fn new(rowid: i64, guid: &'static str, date: i64) -> Self {
        Msg {
            rowid,
            guid,
            text: None,
            attributed: None,
            handle_id: 0,
            date,
            is_from_me: false,
            associated: 0,
            item_type: 0,
        }
    }
}

fn add_message(c: &Connection, m: &Msg) {
    c.execute(
        "INSERT INTO message (ROWID, guid, text, attributedBody, handle_id, date, is_from_me,
                              service, associated_message_type, item_type)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'SMS', ?8, ?9)",
        params![
            m.rowid,
            m.guid,
            m.text,
            m.attributed,
            m.handle_id,
            m.date,
            m.is_from_me as i64,
            m.associated,
            m.item_type,
        ],
    )
    .expect("insert message");
}

fn add_attachment(c: &Connection, rowid: i64, transfer: &str, mime: &str, bytes: i64) {
    c.execute(
        "INSERT INTO attachment (ROWID, filename, mime_type, transfer_name, total_bytes)
         VALUES (?1, '/private/var/xx', ?2, ?3, ?4)",
        params![rowid, mime, transfer, bytes],
    )
    .expect("insert attachment");
}

fn join_message_attachment(c: &Connection, message_id: i64, attachment_id: i64) {
    c.execute(
        "INSERT INTO message_attachment_join (message_id, attachment_id) VALUES (?1, ?2)",
        params![message_id, attachment_id],
    )
    .expect("join attachment");
}

/// `unix_secs` -> seconds since 2001-01-01.
fn ios_secs(unix_secs: i64) -> i64 {
    unix_secs - IOS_EPOCH_SECS
}

/// `unix_secs` -> nanoseconds since 2001-01-01.
fn ios_ns(unix_secs: i64) -> i64 {
    (unix_secs - IOS_EPOCH_SECS) * 1_000_000_000
}

fn utc(y: i32, mo: u32, d: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(y, mo, d, 0, 0, 0)
        .single()
        .expect("valid date")
}

#[test]
fn a_missing_column_names_the_first_missing_one() {
    let c = conn();
    c.execute("ALTER TABLE message DROP COLUMN item_type", [])
        .expect("drop column");
    let err = load(&c).expect_err("must fail schema check");
    assert!(
        matches!(&err, crate::readers::ReaderError::Database(m) if m
            == "Messages schema not supported: missing message.item_type"),
        "unexpected error: {err:?}"
    );
}

#[test]
fn text_attribution_and_unusable_are_counted() {
    let c = conn();
    add_handle(&c, 1, "+10000000001");
    add_chat(&c, 1, "chat-1", None);
    join_chat_handle(&c, 1, 1);

    // 1: plain text.
    let m1 = Msg {
        text: Some("hello"),
        ..Msg::new(1, "g1", ios_secs(1_577_836_800))
    };
    add_message(&c, &m1);
    join_chat_message(&c, 1, 1);
    // 2: attributedBody only.
    let m2 = Msg {
        attributed: Some(attributed_blob("world")),
        ..Msg::new(2, "g2", ios_secs(1_577_836_801))
    };
    add_message(&c, &m2);
    join_chat_message(&c, 1, 2);
    // 3: no text, no attributedBody, no attachments -> unusable.
    let m3 = Msg::new(3, "g3", ios_secs(1_577_836_802));
    add_message(&c, &m3);
    join_chat_message(&c, 1, 3);

    let db = load(&c).expect("load");
    assert_eq!(db.messages.len(), 2);
    assert_eq!(db.messages[0].text.as_deref(), Some("hello"));
    assert_eq!(db.messages[1].text.as_deref(), Some("world"));
    assert_eq!(db.skipped.unusable_text, 1);
}

#[test]
fn an_attachment_only_message_is_kept_without_text() {
    let c = conn();
    add_handle(&c, 1, "+10000000001");
    add_chat(&c, 1, "chat-1", None);
    join_chat_handle(&c, 1, 1);
    let m = Msg::new(1, "g1", ios_secs(1_577_836_800));
    add_message(&c, &m);
    join_chat_message(&c, 1, 1);
    add_attachment(&c, 1, "IMG_0001.jpg", "image/jpeg", 1234);
    join_message_attachment(&c, 1, 1);

    let db = load(&c).expect("load");
    assert_eq!(db.messages.len(), 1);
    assert!(db.messages[0].text.is_none());
    assert_eq!(db.messages[0].attachments.len(), 1);
    assert_eq!(
        db.messages[0].attachments[0].transfer_name.as_deref(),
        Some("IMG_0001.jpg")
    );
    assert_eq!(db.messages[0].attachments[0].total_bytes, Some(1234));
    assert_eq!(db.skipped.unusable_text, 0);
}

#[test]
fn tapbacks_and_group_events_are_skipped_and_counted() {
    let c = conn();
    add_handle(&c, 1, "+10000000001");
    add_chat(&c, 1, "chat-1", None);
    join_chat_handle(&c, 1, 1);

    // A tapback (associated_message_type != 0).
    let tap = Msg {
        text: Some("reacted"),
        associated: 1,
        ..Msg::new(1, "t1", ios_secs(1_577_836_800))
    };
    add_message(&c, &tap);
    join_chat_message(&c, 1, 1);
    // A group event (item_type != 0).
    let grp = Msg {
        text: Some("renamed"),
        item_type: 1,
        ..Msg::new(2, "t2", ios_secs(1_577_836_801))
    };
    add_message(&c, &grp);
    join_chat_message(&c, 1, 2);
    // A normal message, kept.
    let ok = Msg {
        text: Some("hi"),
        ..Msg::new(3, "t3", ios_secs(1_577_836_802))
    };
    add_message(&c, &ok);
    join_chat_message(&c, 1, 3);

    let db = load(&c).expect("load");
    assert_eq!(db.messages.len(), 1);
    assert_eq!(db.messages[0].guid, "t3");
    assert_eq!(db.skipped.tapbacks, 1);
    assert_eq!(db.skipped.group_events, 1);
}

#[test]
fn nanosecond_and_second_dates_resolve_to_the_same_instant() {
    let c = conn();
    add_handle(&c, 1, "+10000000001");
    add_chat(&c, 1, "chat-1", None);
    join_chat_handle(&c, 1, 1);
    let unix = 1_577_836_800; // 2020-01-01 00:00:00 UTC
    let m_s = Msg {
        text: Some("s"),
        ..Msg::new(1, "g1", ios_secs(unix))
    };
    add_message(&c, &m_s);
    join_chat_message(&c, 1, 1);
    let m_ns = Msg {
        text: Some("ns"),
        ..Msg::new(2, "g2", ios_ns(unix))
    };
    add_message(&c, &m_ns);
    join_chat_message(&c, 1, 2);

    let db = load(&c).expect("load");
    let expected = utc(2020, 1, 1);
    assert_eq!(db.messages[0].date_utc, expected);
    assert_eq!(db.messages[1].date_utc, expected);
    assert_eq!(db.skipped.bad_date, 0);
}

#[test]
fn sender_is_resolved_per_chat_shape() {
    let c = conn();
    // 1:1 chat: one participant, message carries no handle.
    add_handle(&c, 1, "+10000000001");
    add_chat(&c, 1, "one2one", None);
    join_chat_handle(&c, 1, 1);
    let m1 = Msg {
        text: Some("hi"),
        ..Msg::new(1, "g1", ios_secs(1_577_836_800))
    };
    add_message(&c, &m1);
    join_chat_message(&c, 1, 1);

    // Group chat: two participants, message carries the sender's handle.
    add_handle(&c, 2, "+10000000002");
    add_handle(&c, 3, "+10000000003");
    add_chat(&c, 2, "group", Some("The Team"));
    join_chat_handle(&c, 2, 2);
    join_chat_handle(&c, 2, 3);
    let m2 = Msg {
        text: Some("from 2"),
        handle_id: 2,
        ..Msg::new(2, "g2", ios_secs(1_577_836_801))
    };
    add_message(&c, &m2);
    join_chat_message(&c, 2, 2);

    // My own message: sender is None.
    let m3 = Msg {
        text: Some("me"),
        is_from_me: true,
        handle_id: 1,
        ..Msg::new(3, "g3", ios_secs(1_577_836_802))
    };
    add_message(&c, &m3);
    join_chat_message(&c, 1, 3);

    let db = load(&c).expect("load");
    let by_guid: std::collections::HashMap<&str, &crate::readers::apple_messages_db::RawMessage> =
        db.messages.iter().map(|m| (m.guid.as_str(), m)).collect();
    // 1:1, no handle: the single participant.
    assert_eq!(by_guid["g1"].sender_handle.as_deref(), Some("+10000000001"));
    // Group: the message's own handle.
    assert_eq!(by_guid["g2"].sender_handle.as_deref(), Some("+10000000002"));
    // From me: None.
    assert_eq!(by_guid["g3"].sender_handle, None);
}

#[test]
fn an_orphan_with_a_handle_gets_its_own_conversation() {
    let c = conn();
    add_handle(&c, 7, "+10000000007");
    // No chat, no chat_message_join; handle_id points at handle ROWID 7.
    let m = Msg {
        text: Some("orphan"),
        handle_id: 7,
        ..Msg::new(1, "g1", ios_secs(1_577_836_800))
    };
    add_message(&c, &m);

    let db = load(&c).expect("load");
    assert_eq!(db.messages.len(), 1);
    assert_eq!(
        db.messages[0].conversation,
        ConversationKey::OrphanHandle(7)
    );
    // The conversation is built with that one participant.
    let conv = db
        .conversations
        .iter()
        .find(|cv| cv.key == ConversationKey::OrphanHandle(7))
        .expect("orphan conversation");
    assert_eq!(conv.participants, vec!["+10000000007".to_string()]);
}

#[test]
fn an_orphan_without_a_handle_is_counted() {
    let c = conn();
    // No chat, no chat_message_join, handle_id = 0.
    let m = Msg {
        text: Some("lost"),
        ..Msg::new(1, "g1", ios_secs(1_577_836_800))
    };
    add_message(&c, &m);

    let db = load(&c).expect("load");
    assert_eq!(db.messages.len(), 0);
    assert_eq!(db.skipped.orphan_no_handle, 1);
}

#[test]
fn messages_are_sorted_by_conversation_date_and_rowid() {
    let c = conn();
    add_handle(&c, 1, "+10000000001");
    add_handle(&c, 2, "+10000000002");
    add_chat(&c, 1, "chat-1", None);
    add_chat(&c, 2, "chat-2", None);
    join_chat_handle(&c, 1, 1);
    join_chat_handle(&c, 2, 2);

    // Insert out of order: chat 2 first, then chat 1 with two dates.
    let a = Msg {
        text: Some("c2"),
        ..Msg::new(10, "a", ios_secs(1_577_836_800))
    };
    add_message(&c, &a);
    join_chat_message(&c, 2, 10);
    let b = Msg {
        text: Some("c1-later"),
        ..Msg::new(11, "b", ios_secs(1_577_836_900))
    };
    add_message(&c, &b);
    join_chat_message(&c, 1, 11);
    let d = Msg {
        text: Some("c1-earlier"),
        ..Msg::new(12, "d", ios_secs(1_577_836_850))
    };
    add_message(&c, &d);
    join_chat_message(&c, 1, 12);

    let db = load(&c).expect("load");
    let order: Vec<(&str, &ConversationKey)> = db
        .messages
        .iter()
        .map(|m| (m.guid.as_str(), &m.conversation))
        .collect();
    // chat 1 (key Chat(1)) sorts before chat 2 (Chat(2)); within chat 1, by date.
    assert_eq!(
        order,
        vec![
            ("d", &ConversationKey::Chat(1)),
            ("b", &ConversationKey::Chat(1)),
            ("a", &ConversationKey::Chat(2)),
        ]
    );
}

/// A `MessagesDb` is the loaded, sorted, counted structure.
#[test]
fn the_loaded_db_exposes_conversations_messages_and_skips() {
    let c = conn();
    add_handle(&c, 1, "+10000000001");
    add_chat(&c, 1, "chat-1", Some("Client"));
    join_chat_handle(&c, 1, 1);
    let m = Msg {
        text: Some("hi"),
        ..Msg::new(1, "g1", ios_secs(1_577_836_800))
    };
    add_message(&c, &m);
    join_chat_message(&c, 1, 1);

    let db: MessagesDb = load(&c).expect("load");
    assert_eq!(db.conversations.len(), 1);
    assert_eq!(db.conversations[0].display_name.as_deref(), Some("Client"));
    assert_eq!(db.conversations[0].chat_guid.as_deref(), Some("chat-1"));
    assert_eq!(db.messages.len(), 1);
    assert_eq!(
        db.skipped,
        crate::readers::apple_messages_db::SkipCounts::default()
    );
}

#[test]
fn a_row_that_fails_to_read_is_counted_not_swallowed() {
    let c = conn();
    add_handle(&c, 1, "+10000000001");
    add_chat(&c, 1, "chat-1", None);
    join_chat_handle(&c, 1, 1);

    // A valid row, so we can prove the others still load.
    let ok = Msg {
        text: Some("fine"),
        ..Msg::new(1, "g1", ios_secs(1_577_836_800))
    };
    add_message(&c, &ok);
    join_chat_message(&c, 1, 1);

    // A row whose guid is NULL: `get::<String>` on it fails, so it is a row
    // error, counted - not silently dropped.
    c.execute(
        "INSERT INTO message (ROWID, guid, text, attributedBody, handle_id, date, is_from_me,
                              service, associated_message_type, item_type)
         VALUES (2, NULL, 'bad', NULL, 0, ?1, 0, 'SMS', 0, 0)",
        [ios_secs(1_577_836_801)],
    )
    .expect("insert bad row");
    join_chat_message(&c, 1, 2);

    let db = load(&c).expect("load");
    // The valid row loaded; the NULL-guid row did not.
    assert_eq!(db.messages.len(), 1);
    assert_eq!(db.messages[0].guid, "g1");
    // The failed row is counted, and no other skip reason fired.
    assert_eq!(db.skipped.row_errors, 1);
    assert_eq!(db.skipped.tapbacks, 0);
    assert_eq!(db.skipped.group_events, 0);
    assert_eq!(db.skipped.unusable_text, 0);
    assert_eq!(db.skipped.orphan_no_handle, 0);
    assert_eq!(db.skipped.bad_date, 0);
}
