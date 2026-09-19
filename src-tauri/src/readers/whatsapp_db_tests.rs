// HalluScribe - tests for the WhatsApp DB layer. All fixtures are synthetic:
// an in-memory ChatStorage.sqlite with invented JIDs. No real data.

use super::load;
use crate::readers::apple_messages_db::ConversationKey;
use chrono::{Datelike, TimeZone, Timelike, Utc};
use rusqlite::{params, Connection};

/// iOS core-data epoch: 2001-01-01 00:00:00 UTC, in unix seconds.
const IOS_EPOCH_SECS: i64 = 978_307_200;

/// The full ChatStorage.sqlite schema the reader depends on.
const SCHEMA: &str = r#"
CREATE TABLE ZWAMESSAGE (
    Z_PK INTEGER PRIMARY KEY,
    ZCHATSESSION INTEGER,
    ZISFROMME INTEGER,
    ZMESSAGEDATE REAL,
    ZMESSAGETYPE INTEGER,
    ZGROUPEVENTTYPE INTEGER,
    ZTEXT TEXT,
    ZPUSHNAME TEXT,
    ZFROMJID TEXT
);
CREATE TABLE ZWACHATSESSION (
    Z_PK INTEGER PRIMARY KEY,
    ZCONTACTJID TEXT,
    ZPARTNERNAME TEXT,
    ZSESSIONTYPE INTEGER
);
CREATE TABLE ZWAGROUPMEMBER (
    Z_PK INTEGER PRIMARY KEY,
    ZCHATSESSION INTEGER,
    ZMEMBERJID TEXT,
    ZCONTACTNAME TEXT
);
CREATE TABLE ZWAPROFILEPUSHNAME (Z_PK INTEGER PRIMARY KEY, ZJID TEXT, ZPUSHNAME TEXT);
"#;

fn conn() -> Connection {
    let c = Connection::open_in_memory().expect("in-memory db");
    c.execute_batch(SCHEMA).expect("create schema");
    c
}

/// A valid WhatsApp `ZMESSAGEDATE`: `unix_seconds - IOS_EPOCH_SECS` as a float.
fn wa_date(unix_secs: i64) -> f64 {
    (unix_secs - IOS_EPOCH_SECS) as f64
}

fn add_session(c: &Connection, pk: i64, jid: &str, partner: Option<&str>, is_group: bool) {
    c.execute(
        "INSERT INTO ZWACHATSESSION (Z_PK, ZCONTACTJID, ZPARTNERNAME, ZSESSIONTYPE)
         VALUES (?1, ?2, ?3, ?4)",
        params![pk, jid, partner, if is_group { 1 } else { 0 }],
    )
    .expect("insert session");
}

// A fixture builder: the 8 data params mirror the `ZWAMESSAGE` column set, so
// bundling them would obscure the fixture. Named fields are the real shape.
#[allow(clippy::too_many_arguments)]
fn add_message(
    c: &Connection,
    pk: i64,
    session: Option<i64>,
    from_me: bool,
    unix_secs: i64,
    msg_type: i64,
    group_event: i64,
    text: Option<&str>,
    from_jid: Option<&str>,
) {
    c.execute(
        "INSERT INTO ZWAMESSAGE
         (Z_PK, ZCHATSESSION, ZISFROMME, ZMESSAGEDATE, ZMESSAGETYPE, ZGROUPEVENTTYPE, ZTEXT, ZPUSHNAME, ZFROMJID)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8)",
        params![pk, session, if from_me { 1 } else { 0 }, wa_date(unix_secs), msg_type, group_event, text, from_jid],
    )
    .expect("insert message");
}

#[test]
fn text_rows_are_loaded_and_sorted() {
    let c = conn();
    add_session(&c, 1, "111@s.whatsapp.net", Some("Nikos P"), false);
    // Out of date order on purpose: the reader must sort by date.
    add_message(
        &c,
        10,
        Some(1),
        false,
        1_700_000_000,
        0,
        0,
        Some("first"),
        Some("111@s.whatsapp.net"),
    );
    add_message(
        &c,
        11,
        Some(1),
        true,
        1_700_000_010,
        0,
        0,
        Some("second"),
        None,
    );

    let db = load(&c).expect("load");
    assert_eq!(db.messages.len(), 2);
    assert_eq!(db.messages[0].text.as_deref(), Some("first"));
    assert!(!db.messages[0].is_from_me);
    assert_eq!(db.messages[1].text.as_deref(), Some("second"));
    assert!(db.messages[1].is_from_me);
    // Sorted by date.
    assert!(db.messages[0].date_utc < db.messages[1].date_utc);
    // The sender JID is kept for the incoming message.
    assert_eq!(
        db.messages[0].sender_handle.as_deref(),
        Some("111@s.whatsapp.net")
    );
    assert_eq!(db.messages[1].sender_handle, None);
}

#[test]
fn the_date_is_seconds_since_ios_epoch() {
    let c = conn();
    add_session(&c, 1, "111@s.whatsapp.net", None, false);
    // A known instant: 2024-01-01 00:00:00 UTC = unix 1704067200.
    add_message(
        &c,
        1,
        Some(1),
        false,
        1_704_067_200,
        0,
        0,
        Some("hi"),
        Some("111@s.whatsapp.net"),
    );

    let db = load(&c).expect("load");
    let dt = db.messages[0].date_utc;
    assert_eq!(dt.year(), 2024);
    assert_eq!(dt.month(), 1);
    assert_eq!(dt.day(), 1);
    assert_eq!(dt.hour(), 0);
}

#[test]
fn a_media_row_without_text_is_counted_as_media() {
    let c = conn();
    add_session(&c, 1, "111@s.whatsapp.net", None, false);
    // ZMESSAGETYPE 1 = image, no text.
    add_message(
        &c,
        1,
        Some(1),
        false,
        1_700_000_000,
        1,
        0,
        None,
        Some("111@s.whatsapp.net"),
    );
    // A text row so the db is non-empty.
    add_message(
        &c,
        2,
        Some(1),
        false,
        1_700_000_001,
        0,
        0,
        Some("ok"),
        Some("111@s.whatsapp.net"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.media, 1);
    assert_eq!(db.messages.len(), 1);
}

#[test]
fn a_group_event_row_without_text_is_counted_as_group_event() {
    let c = conn();
    add_session(&c, 1, "222@g.us", Some("Team"), true);
    // ZGROUPEVENTTYPE 2 = member added, no text.
    add_message(
        &c,
        1,
        Some(1),
        false,
        1_700_000_000,
        0,
        2,
        None,
        Some("333@s.whatsapp.net"),
    );
    add_message(
        &c,
        2,
        Some(1),
        false,
        1_700_000_001,
        0,
        0,
        Some("hello"),
        Some("333@s.whatsapp.net"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.group_events, 1);
    assert_eq!(db.messages.len(), 1);
}

#[test]
fn a_blank_text_row_is_counted_as_unusable_text() {
    let c = conn();
    add_session(&c, 1, "111@s.whatsapp.net", None, false);
    add_message(
        &c,
        1,
        Some(1),
        false,
        1_700_000_000,
        0,
        0,
        Some("   "),
        Some("111@s.whatsapp.net"),
    );
    add_message(
        &c,
        2,
        Some(1),
        false,
        1_700_000_001,
        0,
        0,
        Some("real"),
        Some("111@s.whatsapp.net"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.unusable_text, 1);
    assert_eq!(db.messages.len(), 1);
}

#[test]
fn a_media_row_with_a_caption_keeps_the_caption() {
    let c = conn();
    add_session(&c, 1, "111@s.whatsapp.net", None, false);
    // An image (type 1) that carries a caption: the caption is the message.
    add_message(
        &c,
        1,
        Some(1),
        false,
        1_700_000_000,
        1,
        0,
        Some("look at this"),
        Some("111@s.whatsapp.net"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.media, 0);
    assert_eq!(db.messages.len(), 1);
    assert_eq!(db.messages[0].text.as_deref(), Some("look at this"));
}

#[test]
fn an_orphan_message_without_a_session_is_counted() {
    let c = conn();
    add_message(
        &c,
        1,
        None,
        false,
        1_700_000_000,
        0,
        0,
        Some("orphan"),
        Some("111@s.whatsapp.net"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.orphan_no_session, 1);
    assert!(db.messages.is_empty());
}

#[test]
fn a_bad_date_is_counted() {
    let c = conn();
    add_session(&c, 1, "111@s.whatsapp.net", None, false);
    // A non-finite / zero date is unconvertible.
    c.execute(
        "INSERT INTO ZWAMESSAGE (Z_PK, ZCHATSESSION, ZISFROMME, ZMESSAGEDATE, ZMESSAGETYPE, ZGROUPEVENTTYPE, ZTEXT, ZPUSHNAME, ZFROMJID)
         VALUES (1, 1, 0, 0.0, 0, 0, 'bad date', NULL, '111@s.whatsapp.net')",
        [],
    )
    .expect("insert");

    let db = load(&c).expect("load");
    assert_eq!(db.skipped.bad_date, 1);
    assert!(db.messages.is_empty());
}

#[test]
fn the_conversation_for_a_one_to_one_carries_the_contact_jid() {
    let c = conn();
    add_session(&c, 1, "111@s.whatsapp.net", Some("Nikos P"), false);
    add_message(
        &c,
        1,
        Some(1),
        false,
        1_700_000_000,
        0,
        0,
        Some("hi"),
        Some("111@s.whatsapp.net"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.conversations.len(), 1);
    let conv = &db.conversations[0];
    assert_eq!(conv.key, ConversationKey::Chat(1));
    assert_eq!(conv.chat_guid.as_deref(), Some("111@s.whatsapp.net"));
    assert_eq!(conv.display_name.as_deref(), Some("Nikos P"));
    assert_eq!(conv.participants, vec!["111@s.whatsapp.net".to_string()]);
}

#[test]
fn a_group_conversation_carries_its_member_jids() {
    let c = conn();
    add_session(&c, 1, "222@g.us", Some("Team"), true);
    c.execute(
        "INSERT INTO ZWAGROUPMEMBER (Z_PK, ZCHATSESSION, ZMEMBERJID, ZCONTACTNAME)
         VALUES (1, 1, '333@s.whatsapp.net', 'Alice'), (2, 1, '444@s.whatsapp.net', 'Bob')",
        [],
    )
    .expect("insert members");
    add_message(
        &c,
        1,
        Some(1),
        false,
        1_700_000_000,
        0,
        0,
        Some("hi"),
        Some("333@s.whatsapp.net"),
    );

    let db = load(&c).expect("load");
    assert_eq!(db.conversations.len(), 1);
    assert_eq!(
        db.conversations[0].participants,
        vec![
            "333@s.whatsapp.net".to_string(),
            "444@s.whatsapp.net".to_string()
        ]
    );
}

#[test]
fn push_names_are_loaded() {
    let c = conn();
    add_session(&c, 1, "111@s.whatsapp.net", None, false);
    add_message(
        &c,
        1,
        Some(1),
        false,
        1_700_000_000,
        0,
        0,
        Some("hi"),
        Some("111@s.whatsapp.net"),
    );
    c.execute(
        "INSERT INTO ZWAPROFILEPUSHNAME (Z_PK, ZJID, ZPUSHNAME) VALUES (1, '111@s.whatsapp.net', 'Nikos P')",
        [],
    )
    .expect("insert push name");

    let db = load(&c).expect("load");
    assert_eq!(
        db.push_names.get("111@s.whatsapp.net").map(String::as_str),
        Some("Nikos P")
    );
}

#[test]
fn a_missing_required_column_is_a_database_error() {
    let c = Connection::open_in_memory().expect("in-memory db");
    // A schema missing ZWAMESSAGE.ZTEXT.
    c.execute_batch(
        r#"
        CREATE TABLE ZWAMESSAGE (Z_PK INTEGER PRIMARY KEY, ZCHATSESSION INTEGER, ZISFROMME INTEGER, ZMESSAGEDATE REAL, ZMESSAGETYPE INTEGER, ZGROUPEVENTTYPE INTEGER, ZFROMJID TEXT);
        CREATE TABLE ZWACHATSESSION (Z_PK INTEGER PRIMARY KEY, ZCONTACTJID TEXT, ZPARTNERNAME TEXT, ZSESSIONTYPE INTEGER);
        CREATE TABLE ZWAGROUPMEMBER (Z_PK INTEGER PRIMARY KEY, ZCHATSESSION INTEGER, ZMEMBERJID TEXT, ZCONTACTNAME TEXT);
        CREATE TABLE ZWAPROFILEPUSHNAME (Z_PK INTEGER PRIMARY KEY, ZJID TEXT, ZPUSHNAME TEXT);
        "#,
    )
    .expect("create schema");

    let err = load(&c).expect_err("should fail");
    match err {
        crate::readers::ReaderError::Database(msg) => {
            assert!(msg.contains("ZWAMESSAGE.ZTEXT"), "got: {msg}")
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
