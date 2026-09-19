// HalluScribe - tests for the Viber reader. All fixtures are synthetic: a
// fake backup dir (a real `Manifest.db` plus a hashed `Contacts.data` and the
// iPhone `AddressBook.sqlitedb`), all with invented phone numbers. No real
// data.

use super::{is_available, read};
use crate::readers::{ChatProvider, MessageRole};
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

/// iOS core-data epoch: 2001-01-01 00:00:00 UTC, in unix seconds.
const IOS_EPOCH_SECS: i64 = 978_307_200;

const VIBER_DOMAIN: &str = "AppDomainGroup-group.viber.share.container";
const CONTACTS_REL: &str = "com.viber/database/Contacts.data";
const CHAT_ID: &str = "aa0000000000000000000000000000000000000000";
const ADDR_ID: &str = "bb0000000000000000000000000000000000000000";

/// A valid Viber `ZDATE`: `unix_seconds - IOS_EPOCH_SECS` as a float.
fn viber_date(unix_secs: i64) -> f64 {
    (unix_secs - IOS_EPOCH_SECS) as f64
}

/// One synthetic `ZVIBERMESSAGE` row: (rowid, conversation, unix_secs, state,
/// phone_num_index, system_type, attachment, gallery_type, text). Named to
/// keep the spec's field type under clippy's `type_complexity` limit.
#[allow(clippy::type_complexity)]
type MessageRow = (
    i64,
    i64,
    i64,
    &'static str,
    Option<i64>,
    Option<&'static str>,
    i64,
    i64,
    Option<&'static str>,
);

/// A synthetic Contacts.data spec.
struct ViberData {
    conversations: Vec<(i64, Option<&'static str>, Option<&'static str>)>,
    phones: Vec<(i64, &'static str)>,
    messages: Vec<MessageRow>,
}

impl ViberData {
    fn sql(&self) -> String {
        let mut s = String::from(
            r#"
CREATE TABLE ZVIBERMESSAGE (Z_PK INTEGER PRIMARY KEY, ZCONVERSATION INTEGER, ZDATE REAL, ZTEXT TEXT, ZSTATE TEXT, ZPHONENUMINDEX INTEGER, ZSYSTEMTYPE TEXT, ZATTACHMENT INTEGER, ZGALLERYTYPE INTEGER);
CREATE TABLE ZCONVERSATION (Z_PK INTEGER PRIMARY KEY, ZGROUPID TEXT, ZNAME TEXT);
CREATE TABLE ZPHONENUMBER (Z_PK INTEGER PRIMARY KEY, ZPHONE TEXT);
"#,
        );
        for (pk, group_id, name) in &self.conversations {
            let g = group_id
                .map(|x| format!("'{x}'"))
                .unwrap_or_else(|| "NULL".into());
            let n = name
                .map(|x| format!("'{x}'"))
                .unwrap_or_else(|| "NULL".into());
            s.push_str(&format!(
                "INSERT INTO ZCONVERSATION (Z_PK, ZGROUPID, ZNAME) VALUES ({pk}, {g}, {n});"
            ));
        }
        for (pk, phone) in &self.phones {
            s.push_str(&format!(
                "INSERT INTO ZPHONENUMBER (Z_PK, ZPHONE) VALUES ({pk}, '{phone}');"
            ));
        }
        for (pk, conv, unix, state, phone_idx, system, attach, gallery, text) in &self.messages {
            let t = text
                .map(|x| format!("'{x}'"))
                .unwrap_or_else(|| "NULL".into());
            let p = phone_idx
                .map(|x| x.to_string())
                .unwrap_or_else(|| "NULL".into());
            let sy = system
                .map(|x| format!("'{x}'"))
                .unwrap_or_else(|| "NULL".into());
            s.push_str(&format!(
                "INSERT INTO ZVIBERMESSAGE (Z_PK, ZCONVERSATION, ZDATE, ZTEXT, ZSTATE, ZPHONENUMINDEX, ZSYSTEMTYPE, ZATTACHMENT, ZGALLERYTYPE)
                 VALUES ({pk}, {conv}, {}, {t}, '{state}', {p}, {sy}, {attach}, {gallery});",
                viber_date(*unix)
            ));
        }
        s
    }
}

/// Write a real SQLite file at `path` from `sql`.
fn write_sqlite(path: &Path, sql: &str) {
    let conn = Connection::open(path).expect("open db");
    conn.execute_batch(sql).expect("build db");
    drop(conn);
}

/// Build a fake backup dir: a real `Manifest.db` plus a hashed
/// `Contacts.data` in the Viber domain, and (if given) the iPhone
/// `AddressBook.sqlitedb` in the HomeDomain.
fn make_backup(root: &Path, data: &ViberData, address_book: Option<&str>) {
    let _ = fs::remove_file(root.join("Manifest.db"));
    let _ = fs::remove_dir_all(root.join(&CHAT_ID[..2]));
    let _ = fs::remove_dir_all(root.join(&ADDR_ID[..2]));

    // Contacts.data.
    let chat_tmp = root.join("__contacts.data");
    write_sqlite(&chat_tmp, &data.sql());
    let chat_bytes = fs::read(&chat_tmp).expect("read contacts");
    fs::remove_file(&chat_tmp).expect("remove tmp");
    let chat_path = root.join(&CHAT_ID[..2]).join(CHAT_ID);
    fs::create_dir_all(chat_path.parent().unwrap()).expect("dir");
    fs::write(&chat_path, &chat_bytes).expect("write contacts");

    let mut rows = format!("('{CHAT_ID}', '{VIBER_DOMAIN}', '{CONTACTS_REL}')");

    // The iPhone address book (optional), in the HomeDomain.
    if let Some(addr_sql) = address_book {
        let addr_tmp = root.join("__addr.sqlitedb");
        write_sqlite(&addr_tmp, addr_sql);
        let addr_bytes = fs::read(&addr_tmp).expect("read addr");
        fs::remove_file(&addr_tmp).expect("remove tmp");
        let addr_path = root.join(&ADDR_ID[..2]).join(ADDR_ID);
        fs::create_dir_all(addr_path.parent().unwrap()).expect("dir");
        fs::write(&addr_path, &addr_bytes).expect("write addr");
        rows.push_str(&format!(
            ", ('{ADDR_ID}', 'HomeDomain', 'Library/AddressBook/AddressBook.sqlitedb')"
        ));
    }

    let manifest = root.join("Manifest.db");
    let mc = Connection::open(&manifest).expect("open manifest");
    mc.execute_batch(&format!(
        "CREATE TABLE Files (fileID TEXT, domain TEXT, relativePath TEXT); INSERT INTO Files VALUES {rows};"
    ))
    .expect("build manifest");
    drop(mc);
}

/// The iPhone address book with one phone contact (Nikos, org ABC Marble Ltd).
const ADDR_SQL: &str = r#"
CREATE TABLE ABPerson (ROWID INTEGER PRIMARY KEY, First TEXT, Last TEXT, Organization TEXT);
CREATE TABLE ABMultiValue (record_id INTEGER, property INTEGER, value TEXT);
INSERT INTO ABPerson (ROWID, First, Last, Organization) VALUES (1, 'Nikos', 'Papadopoulos', 'ABC Marble Ltd');
INSERT INTO ABMultiValue (record_id, property, value) VALUES (1, 3, '+306912345678');
"#;

fn two_messages() -> ViberData {
    ViberData {
        conversations: vec![(1, None, Some("Nikos P"))],
        phones: vec![(100, "+306912345678")],
        messages: vec![
            (
                1,
                1,
                1_700_000_000,
                "received",
                Some(100),
                None,
                0,
                0,
                Some("Hello"),
            ),
            (
                2,
                1,
                1_700_000_010,
                "delivered",
                None,
                None,
                0,
                0,
                Some("Hi, how can I help?"),
            ),
        ],
    }
}

#[test]
fn is_available_is_true_when_contacts_data_is_present() {
    let dir = tempdir().unwrap();
    make_backup(dir.path(), &two_messages(), Some(ADDR_SQL));
    assert!(is_available(dir.path()).expect("is_available"));
}

#[test]
fn is_available_is_false_for_a_backup_with_no_viber() {
    let dir = tempdir().unwrap();
    // A manifest with no Viber domain at all.
    let manifest = dir.path().join("Manifest.db");
    let mc = Connection::open(&manifest).expect("open manifest");
    mc.execute_batch(
        "CREATE TABLE Files (fileID TEXT, domain TEXT, relativePath TEXT);
         INSERT INTO Files VALUES ('cc0000000000000000000000000000000000000000', 'HomeDomain', 'Library/SMS/sms.db');",
    )
    .expect("build manifest");
    drop(mc);
    assert!(!is_available(dir.path()).expect("is_available"));
}

#[test]
fn read_yields_one_session_per_conversation() {
    let dir = tempdir().unwrap();
    make_backup(dir.path(), &two_messages(), Some(ADDR_SQL));
    let sessions = read(dir.path(), Some("30")).expect("read");
    assert_eq!(sessions.len(), 1);
    let s = &sessions[0];
    assert_eq!(s.provider, ChatProvider::Viber);
    // The title is the resolved contact name (the address book).
    assert_eq!(s.title, "Nikos Papadopoulos");
    assert!(s.raw_slice.is_some(), "every session carries its raw slice");
    assert!(s.fill_estimated);
    // The project override is the resolved client's organization.
    assert_eq!(s.project_override.as_deref(), Some("ABC Marble Ltd"));
    // Two messages: the incoming (User) and the outgoing (Assistant / Me).
    assert_eq!(s.messages.len(), 2);
    assert_eq!(s.messages[0].role, MessageRole::User);
    assert_eq!(
        s.messages[0].speaker.as_deref(),
        Some("Nikos Papadopoulos | +306912345678 | 6912345678")
    );
    assert_eq!(s.messages[1].role, MessageRole::Assistant);
    assert_eq!(s.messages[1].speaker.as_deref(), Some("Me"));
}

#[test]
fn a_missing_contacts_data_is_a_real_error() {
    let dir = tempdir().unwrap();
    // A backup with no Contacts.data in the Viber domain.
    let manifest = dir.path().join("Manifest.db");
    let mc = Connection::open(&manifest).expect("open manifest");
    mc.execute_batch(
        "CREATE TABLE Files (fileID TEXT, domain TEXT, relativePath TEXT);
         INSERT INTO Files VALUES ('cc0000000000000000000000000000000000000000', 'HomeDomain', 'Library/SMS/sms.db');",
    )
    .expect("build manifest");
    drop(mc);
    let err = read(dir.path(), Some("30")).expect_err("should fail");
    assert!(matches!(err, crate::readers::ReaderError::Database(_)));
}

#[test]
fn a_gap_over_30_days_splits_into_two_sessions() {
    let data = ViberData {
        conversations: vec![(1, None, Some("Nikos P"))],
        phones: vec![(100, "+306912345678")],
        messages: vec![
            (
                1,
                1,
                1_700_000_000,
                "received",
                Some(100),
                None,
                0,
                0,
                Some("first"),
            ),
            // 31 days later: a new window.
            (
                2,
                1,
                1_700_000_000 + 31 * 86_400,
                "received",
                Some(100),
                None,
                0,
                0,
                Some("second"),
            ),
        ],
    };
    let dir = tempdir().unwrap();
    make_backup(dir.path(), &data, Some(ADDR_SQL));
    let sessions = read(dir.path(), Some("30")).expect("read");
    assert_eq!(sessions.len(), 2);
}

/// The session id is `<provider_key>-<sanitized first guid>`; the Viber guid
/// is `viber-<rowid>`, so the id is `viber-viber-<rowid>`.
#[test]
fn the_session_id_is_prefixed_with_the_provider_key() {
    let dir = tempdir().unwrap();
    make_backup(dir.path(), &two_messages(), Some(ADDR_SQL));
    let sessions = read(dir.path(), Some("30")).expect("read");
    assert_eq!(sessions[0].id, "viber-viber-1");
}

/// The reader leaves the backup directory untouched (read-only on every
/// source): no temp copy or journal is left behind in the backup.
#[test]
fn the_backup_dir_is_not_modified() {
    let dir = tempdir().unwrap();
    make_backup(dir.path(), &two_messages(), Some(ADDR_SQL));
    let before: Vec<PathBuf> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let _ = read(dir.path(), Some("30")).expect("read");
    let after: Vec<PathBuf> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(before, after);
}
