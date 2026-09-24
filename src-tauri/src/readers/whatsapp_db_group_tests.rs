// HalluScribe - tests for the WhatsApp group sender (`ZGROUPMEMBER`). All
// fixtures are synthetic: an in-memory ChatStorage.sqlite with invented JIDs.

use super::load;
use rusqlite::{params, Connection};

/// iOS core-data epoch: 2001-01-01 00:00:00 UTC, in unix seconds.
const IOS_EPOCH_SECS: i64 = 978_307_200;

/// The ChatStorage.sqlite schema with `ZWAMESSAGE.ZGROUPMEMBER`, as on iOS.
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
    ZFROMJID TEXT,
    ZGROUPMEMBER INTEGER
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
INSERT INTO ZWACHATSESSION VALUES (1, '222@g.us', 'Family', 1);
INSERT INTO ZWAGROUPMEMBER VALUES (7, 1, '306912345678@s.whatsapp.net', NULL);
"#;

fn conn() -> Connection {
    let c = Connection::open_in_memory().expect("in-memory db");
    c.execute_batch(SCHEMA).expect("create schema");
    c
}

fn add_group_message(c: &Connection, pk: i64, from_jid: Option<&str>, member: Option<i64>) {
    c.execute(
        "INSERT INTO ZWAMESSAGE
         (Z_PK, ZCHATSESSION, ZISFROMME, ZMESSAGEDATE, ZMESSAGETYPE, ZGROUPEVENTTYPE,
          ZTEXT, ZPUSHNAME, ZFROMJID, ZGROUPMEMBER)
         VALUES (?1, 1, 0, ?2, 0, 0, 'hello', NULL, ?3, ?4)",
        params![
            pk,
            (1_700_000_000 - IOS_EPOCH_SECS + pk) as f64,
            from_jid,
            member
        ],
    )
    .expect("insert message");
}

#[test]
fn a_group_message_is_credited_to_its_member_not_the_group() {
    let c = conn();
    add_group_message(&c, 10, Some("222@g.us"), Some(7));
    let db = load(&c).expect("load");
    assert_eq!(db.messages.len(), 1);
    assert_eq!(
        db.messages[0].sender_handle.as_deref(),
        Some("306912345678@s.whatsapp.net")
    );
}

#[test]
fn a_group_jid_without_a_member_is_unknown() {
    let c = conn();
    add_group_message(&c, 10, Some("222@g.us"), None);
    // A dangling member id resolves to nothing, too.
    add_group_message(&c, 11, Some("222@g.us"), Some(99));
    let db = load(&c).expect("load");
    assert_eq!(db.messages.len(), 2);
    assert!(db.messages.iter().all(|m| m.sender_handle.is_none()));
}

#[test]
fn a_person_jid_in_zfromjid_is_still_used_without_a_member() {
    let c = conn();
    add_group_message(&c, 10, Some("306900000001@s.whatsapp.net"), None);
    let db = load(&c).expect("load");
    assert_eq!(
        db.messages[0].sender_handle.as_deref(),
        Some("306900000001@s.whatsapp.net")
    );
}
