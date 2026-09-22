// HalluScribe - tests for the Viber DB layer. All fixtures are synthetic:
// an in-memory Contacts.data with invented phone numbers. No real data.

use super::load;
use crate::readers::apple_messages_db::ConversationKey;
use chrono::{Datelike, TimeZone, Timelike, Utc};
use rusqlite::{params, Connection};

/// iOS core-data epoch: 2001-01-01 00:00:00 UTC, in unix seconds.
const IOS_EPOCH_SECS: i64 = 978_307_200;

/// The full Contacts.data schema the reader depends on.
const SCHEMA: &str = r#"
CREATE TABLE ZVIBERMESSAGE (
    Z_PK INTEGER PRIMARY KEY,
    ZCONVERSATION INTEGER,
    ZDATE REAL,
    ZTEXT TEXT,
    ZSTATE TEXT,
    ZPHONENUMINDEX INTEGER,
    ZSYSTEMTYPE TEXT,
    ZATTACHMENT INTEGER,
    ZGALLERYTYPE INTEGER
);
CREATE TABLE ZCONVERSATION (
    Z_PK INTEGER PRIMARY KEY,
    ZGROUPID TEXT,
    ZNAME TEXT
);
CREATE TABLE ZPHONENUMBER (
    Z_PK INTEGER PRIMARY KEY,
    ZPHONE TEXT
);
"#;

fn conn() -> Connection {
    let c = Connection::open_in_memory().expect("in-memory db");
    c.execute_batch(SCHEMA).expect("create schema");
    c
}

/// A valid Viber `ZDATE`: `unix_seconds - IOS_EPOCH_SECS` as a float.
fn viber_date(unix_secs: i64) -> f64 {
    (unix_secs - IOS_EPOCH_SECS) as f64
}

fn add_conversation(c: &Connection, pk: i64, group_id: Option<&str>, name: Option<&str>) {
    c.execute(
        "INSERT INTO ZCONVERSATION (Z_PK, ZGROUPID, ZNAME) VALUES (?1, ?2, ?3)",
        params![pk, group_id, name],
    )
    .expect("insert conversation");
}

fn add_phone(c: &Connection, pk: i64, phone: &str) {
    c.execute(
        "INSERT INTO ZPHONENUMBER (Z_PK, ZPHONE) VALUES (?1, ?2)",
        params![pk, phone],
    )
    .expect("insert phone");
}

// A fixture builder: the data params mirror the `ZVIBERMESSAGE` column set, so
// bundling them would obscure the fixture. Named fields are the real shape.
#[allow(clippy::too_many_arguments)]
fn add_message(
    c: &Connection,
    pk: i64,
    conversation: Option<i64>,
    unix_secs: i64,
    state: &str,
    phone_num_index: Option<i64>,
    system_type: Option<&str>,
    attachment: i64,
    gallery_type: i64,
    text: Option<&str>,
) {
    c.execute(
        "INSERT INTO ZVIBERMESSAGE
         (Z_PK, ZCONVERSATION, ZDATE, ZTEXT, ZSTATE, ZPHONENUMINDEX, ZSYSTEMTYPE, ZATTACHMENT, ZGALLERYTYPE)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![pk, conversation, viber_date(unix_secs), text, state, phone_num_index, system_type, attachment, gallery_type],
    )
    .expect("insert message");
}

#[test]
fn text_rows_are_loaded_and_sorted() {
    let c = conn();
    add_conversation(&c, 1, None, Some("Nikos P"));
    add_phone(&c, 100, "+306912345678");
    // Out of date order on purpose: the reader must sort by date.
    add_message(
        &c,
        10,
        Some(1),
        1_700_000_000,
        "received",
        Some(100),
        None,
        0,
        0,
        Some("first"),
    );
    add_message(
        &c,
        11,
        Some(1),
        1_700_000_010,
        "delivered",
        None,
        None,
        0,
        0,
        Some("second"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.messages.len(), 2);
    assert_eq!(db.messages[0].text.as_deref(), Some("first"));
    assert!(!db.messages[0].is_from_me);
    assert_eq!(db.messages[1].text.as_deref(), Some("second"));
    assert!(db.messages[1].is_from_me);
    // Sorted by date.
    assert!(db.messages[0].date_utc < db.messages[1].date_utc);
    // The sender phone is kept for the incoming message.
    assert_eq!(
        db.messages[0].sender_handle.as_deref(),
        Some("+306912345678")
    );
    assert_eq!(db.messages[1].sender_handle, None);
}

#[test]
fn the_date_is_seconds_since_ios_epoch() {
    let c = conn();
    add_conversation(&c, 1, None, None);
    add_phone(&c, 100, "+306912345678");
    // A known instant: 2024-01-01 00:00:00 UTC = unix 1704067200.
    add_message(
        &c,
        1,
        Some(1),
        1_704_067_200,
        "received",
        Some(100),
        None,
        0,
        0,
        Some("hi"),
    );

    let db = load(&c).expect("load");
    let dt = db.messages[0].date_utc;
    assert_eq!(dt.year(), 2024);
    assert_eq!(dt.month(), 1);
    assert_eq!(dt.day(), 1);
    assert_eq!(dt.hour(), 0);
}

#[test]
fn a_system_row_is_counted_as_system() {
    let c = conn();
    add_conversation(&c, 1, None, None);
    add_phone(&c, 100, "+306912345678");
    // ZSYSTEMTYPE set = a call event, even with a sender index.
    add_message(
        &c,
        1,
        Some(1),
        1_700_000_000,
        "send",
        Some(100),
        Some("call"),
        0,
        0,
        Some("call log"),
    );
    add_message(
        &c,
        2,
        Some(1),
        1_700_000_001,
        "received",
        Some(100),
        None,
        0,
        0,
        Some("ok"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.system, 1);
    assert_eq!(db.messages.len(), 1);
}

#[test]
fn a_media_row_without_text_is_counted_as_media() {
    let c = conn();
    add_conversation(&c, 1, None, None);
    add_phone(&c, 100, "+306912345678");
    // Blank text + ZATTACHMENT set = an image.
    add_message(
        &c,
        1,
        Some(1),
        1_700_000_000,
        "received",
        Some(100),
        None,
        5,
        0,
        None,
    );
    add_message(
        &c,
        2,
        Some(1),
        1_700_000_001,
        "received",
        Some(100),
        None,
        0,
        0,
        Some("ok"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.media, 1);
    assert_eq!(db.messages.len(), 1);
}

#[test]
fn a_gallery_row_without_text_is_counted_as_media() {
    let c = conn();
    add_conversation(&c, 1, None, None);
    add_phone(&c, 100, "+306912345678");
    // Blank text + ZGALLERYTYPE set = a gallery item.
    add_message(
        &c,
        1,
        Some(1),
        1_700_000_000,
        "received",
        Some(100),
        None,
        0,
        2,
        None,
    );
    add_message(
        &c,
        2,
        Some(1),
        1_700_000_001,
        "received",
        Some(100),
        None,
        0,
        0,
        Some("ok"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.media, 1);
    assert_eq!(db.messages.len(), 1);
}

#[test]
fn a_unknown_state_is_counted_and_skipped() {
    let c = conn();
    add_conversation(&c, 1, None, None);
    add_phone(&c, 100, "+306912345678");
    // A ZSTATE that is not a known direction value.
    add_message(
        &c,
        1,
        Some(1),
        1_700_000_000,
        "someFutureState",
        Some(100),
        None,
        0,
        0,
        Some("mystery"),
    );
    add_message(
        &c,
        2,
        Some(1),
        1_700_000_001,
        "received",
        Some(100),
        None,
        0,
        0,
        Some("ok"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.unknown_state, 1);
    assert_eq!(db.messages.len(), 1);
}

#[test]
fn a_blank_text_row_is_counted_as_unusable_text() {
    let c = conn();
    add_conversation(&c, 1, None, None);
    add_phone(&c, 100, "+306912345678");
    add_message(
        &c,
        1,
        Some(1),
        1_700_000_000,
        "received",
        Some(100),
        None,
        0,
        0,
        Some("   "),
    );
    add_message(
        &c,
        2,
        Some(1),
        1_700_000_001,
        "received",
        Some(100),
        None,
        0,
        0,
        Some("real"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.unusable_text, 1);
    assert_eq!(db.messages.len(), 1);
}

#[test]
fn an_orphan_message_without_a_conversation_is_counted() {
    let c = conn();
    add_message(
        &c,
        1,
        None,
        1_700_000_000,
        "received",
        Some(100),
        None,
        0,
        0,
        Some("orphan"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.orphan_no_session, 1);
    assert!(db.messages.is_empty());
}

#[test]
fn a_message_whose_conversation_row_is_gone_is_an_orphan_not_a_panic() {
    // Seen on a real backup: ZCONVERSATION points at a deleted conversation.
    let c = conn();
    add_conversation(&c, 1, None, Some("kept"));
    add_message(
        &c,
        1,
        Some(1),
        1_700_000_000,
        "received",
        None,
        None,
        0,
        0,
        Some("hi"),
    );
    add_message(
        &c,
        2,
        Some(99),
        1_700_000_001,
        "received",
        None,
        None,
        0,
        0,
        Some("gone"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.orphan_no_session, 1);
    assert_eq!(db.messages.len(), 1);
    assert_eq!(db.conversations.len(), 1);
}

#[test]
fn a_bad_date_is_counted() {
    let c = conn();
    add_conversation(&c, 1, None, None);
    // A zero date is unconvertible.
    c.execute(
        "INSERT INTO ZVIBERMESSAGE (Z_PK, ZCONVERSATION, ZDATE, ZTEXT, ZSTATE, ZPHONENUMINDEX, ZSYSTEMTYPE, ZATTACHMENT, ZGALLERYTYPE)
         VALUES (1, 1, 0.0, 'bad date', 'received', 100, NULL, 0, 0)",
        [],
    )
    .expect("insert");

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.bad_date, 1);
    assert!(db.messages.is_empty());
}

#[test]
fn a_received_row_with_no_sender_is_kept_and_counted() {
    let c = conn();
    add_conversation(&c, 1, None, Some("Nikos P"));
    // A received row whose ZPHONENUMINDEX is NULL: kept, counted as no_sender.
    add_message(
        &c,
        1,
        Some(1),
        1_700_000_000,
        "received",
        None,
        None,
        0,
        0,
        Some("hi"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.no_sender, 1);
    assert_eq!(db.messages.len(), 1);
    assert_eq!(db.messages[0].sender_handle, None);
    assert!(!db.messages[0].is_from_me);
}

#[test]
fn a_sent_state_is_from_me() {
    let c = conn();
    add_conversation(&c, 1, None, None);
    for (i, state) in ["delivered", "send", "pendingNotSent"].iter().enumerate() {
        add_message(
            &c,
            i as i64 + 1,
            Some(1),
            1_700_000_000 + i as i64,
            state,
            None,
            None,
            0,
            0,
            Some("me"),
        );
    }

    let db = load(&c).expect("load");
    assert_eq!(db.messages.len(), 3);
    assert!(db.messages.iter().all(|m| m.is_from_me));
}

#[test]
fn the_conversation_for_a_one_to_one_carries_the_sender_phone() {
    let c = conn();
    add_conversation(&c, 1, None, Some("Nikos P"));
    add_phone(&c, 100, "+306912345678");
    add_message(
        &c,
        1,
        Some(1),
        1_700_000_000,
        "received",
        Some(100),
        None,
        0,
        0,
        Some("hi"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.conversations.len(), 1);
    let conv = &db.conversations[0];
    assert_eq!(conv.key, ConversationKey::Chat(1));
    assert_eq!(conv.display_name.as_deref(), Some("Nikos P"));
    assert_eq!(conv.participants, vec!["+306912345678".to_string()]);
}

#[test]
fn a_group_conversation_carries_its_distinct_sender_phones() {
    let c = conn();
    add_conversation(&c, 1, Some("12345"), Some("Team"));
    add_phone(&c, 100, "+306912345678");
    add_phone(&c, 101, "+306987654321");
    // Two distinct senders, one repeated.
    add_message(
        &c,
        1,
        Some(1),
        1_700_000_000,
        "received",
        Some(100),
        None,
        0,
        0,
        Some("hi"),
    );
    add_message(
        &c,
        2,
        Some(1),
        1_700_000_001,
        "received",
        Some(101),
        None,
        0,
        0,
        Some("hello"),
    );
    add_message(
        &c,
        3,
        Some(1),
        1_700_000_002,
        "received",
        Some(100),
        None,
        0,
        0,
        Some("again"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.conversations.len(), 1);
    assert_eq!(
        db.conversations[0].participants,
        vec!["+306912345678".to_string(), "+306987654321".to_string()]
    );
}

#[test]
fn a_missing_required_column_is_a_database_error() {
    let c = Connection::open_in_memory().expect("in-memory db");
    // A schema missing ZVIBERMESSAGE.ZSTATE.
    c.execute_batch(
        r#"
        CREATE TABLE ZVIBERMESSAGE (Z_PK INTEGER PRIMARY KEY, ZCONVERSATION INTEGER, ZDATE REAL, ZTEXT TEXT, ZPHONENUMINDEX INTEGER, ZSYSTEMTYPE TEXT, ZATTACHMENT INTEGER, ZGALLERYTYPE INTEGER);
        CREATE TABLE ZCONVERSATION (Z_PK INTEGER PRIMARY KEY, ZGROUPID TEXT, ZNAME TEXT);
        CREATE TABLE ZPHONENUMBER (Z_PK INTEGER PRIMARY KEY, ZPHONE TEXT);
        "#,
    )
    .expect("create schema");

    let err = load(&c).expect_err("should fail");
    match err {
        crate::readers::ReaderError::Database(msg) => {
            assert!(msg.contains("ZVIBERMESSAGE.ZSTATE"), "got: {msg}")
        }
        other => panic!("expected Database error, got {other:?}"),
    }
}

/// A helper the date test relies on: the epoch constant is the iOS one.
#[test]
fn the_ios_epoch_is_2001() {
    let dt = Utc
        .timestamp_opt(IOS_EPOCH_SECS, 0)
        .single()
        .expect("valid");
    assert_eq!(dt.year(), 2001);
}
