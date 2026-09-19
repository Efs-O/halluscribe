// HalluScribe - tests for the WhatsApp reader. All fixtures are synthetic: a
// fake backup dir (a real `Manifest.db` plus a hashed `ChatStorage.sqlite` and
// `ContactsV2.sqlite`), all with invented JIDs. No real data.

use super::{available_apps, read};
use crate::readers::{ChatProvider, MessageRole, ParsedSession};
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

/// iOS core-data epoch: 2001-01-01 00:00:00 UTC, in unix seconds.
const IOS_EPOCH_SECS: i64 = 978_307_200;

const WHATSAPP_DOMAIN: &str = "AppDomainGroup-group.net.whatsapp.WhatsApp.shared";
const WHATSAPP_BUSINESS_DOMAIN: &str = "AppDomainGroup-group.net.whatsapp.WhatsAppSMB.shared";
const CHAT_ID: &str = "aa0000000000000000000000000000000000000000";
const ADDR_ID: &str = "bb0000000000000000000000000000000000000000";

/// A valid WhatsApp `ZMESSAGEDATE`: `unix_seconds - IOS_EPOCH_SECS` as a float.
fn wa_date(unix_secs: i64) -> f64 {
    (unix_secs - IOS_EPOCH_SECS) as f64
}

/// One synthetic `ZWAMESSAGE` row: (rowid, session, from_me, unix_secs,
/// msg_type, group_event, text, from_jid). Named to keep the spec's field type
/// under clippy's `type_complexity` limit.
#[allow(clippy::type_complexity)]
type MessageRow = (
    i64,
    i64,
    bool,
    i64,
    i64,
    i64,
    Option<&'static str>,
    Option<&'static str>,
);

/// A synthetic ChatStorage spec.
struct WaData {
    sessions: Vec<(i64, &'static str, Option<&'static str>, bool)>,
    members: Vec<(i64, i64, &'static str)>,
    messages: Vec<MessageRow>,
    push: Vec<(i64, &'static str, &'static str)>,
}

impl WaData {
    fn sql(&self) -> String {
        let mut s = String::from(
            r#"
CREATE TABLE ZWAMESSAGE (Z_PK INTEGER PRIMARY KEY, ZCHATSESSION INTEGER, ZISFROMME INTEGER, ZMESSAGEDATE REAL, ZMESSAGETYPE INTEGER, ZGROUPEVENTTYPE INTEGER, ZTEXT TEXT, ZPUSHNAME TEXT, ZFROMJID TEXT);
CREATE TABLE ZWACHATSESSION (Z_PK INTEGER PRIMARY KEY, ZCONTACTJID TEXT, ZPARTNERNAME TEXT, ZSESSIONTYPE INTEGER);
CREATE TABLE ZWAGROUPMEMBER (Z_PK INTEGER PRIMARY KEY, ZCHATSESSION INTEGER, ZMEMBERJID TEXT, ZCONTACTNAME TEXT);
CREATE TABLE ZWAPROFILEPUSHNAME (Z_PK INTEGER PRIMARY KEY, ZJID TEXT, ZPUSHNAME TEXT);
"#,
        );
        for (pk, jid, partner, is_group) in &self.sessions {
            let p = partner
                .map(|x| format!("'{x}'"))
                .unwrap_or_else(|| "NULL".into());
            s.push_str(&format!(
                "INSERT INTO ZWACHATSESSION (Z_PK, ZCONTACTJID, ZPARTNERNAME, ZSESSIONTYPE) VALUES ({pk}, '{jid}', {p}, {});",
                if *is_group { 1 } else { 0 }
            ));
        }
        for (pk, session, jid) in &self.members {
            s.push_str(&format!(
                "INSERT INTO ZWAGROUPMEMBER (Z_PK, ZCHATSESSION, ZMEMBERJID, ZCONTACTNAME) VALUES ({pk}, {session}, '{jid}', NULL);"
            ));
        }
        for (pk, session, from_me, unix, mtype, gevent, text, from_jid) in &self.messages {
            let t = text
                .map(|x| format!("'{x}'"))
                .unwrap_or_else(|| "NULL".into());
            let f = from_jid
                .map(|x| format!("'{x}'"))
                .unwrap_or_else(|| "NULL".into());
            s.push_str(&format!(
                "INSERT INTO ZWAMESSAGE (Z_PK, ZCHATSESSION, ZISFROMME, ZMESSAGEDATE, ZMESSAGETYPE, ZGROUPEVENTTYPE, ZTEXT, ZPUSHNAME, ZFROMJID)
                 VALUES ({pk}, {session}, {}, {}, {mtype}, {gevent}, {t}, NULL, {f});",
                if *from_me { 1 } else { 0 },
                wa_date(*unix)
            ));
        }
        for (pk, jid, name) in &self.push {
            s.push_str(&format!(
                "INSERT INTO ZWAPROFILEPUSHNAME (Z_PK, ZJID, ZPUSHNAME) VALUES ({pk}, '{jid}', '{name}');"
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
/// `ChatStorage.sqlite` in the given domain, and (if given) the iPhone
/// `AddressBook.sqlitedb` in the HomeDomain.
fn make_backup(root: &Path, domain: &str, data: &WaData, address_book: Option<&str>) {
    let _ = fs::remove_file(root.join("Manifest.db"));
    let _ = fs::remove_dir_all(root.join(&CHAT_ID[..2]));
    let _ = fs::remove_dir_all(root.join(&ADDR_ID[..2]));

    // ChatStorage.
    let chat_tmp = root.join("__chat.sqlite");
    write_sqlite(&chat_tmp, &data.sql());
    let chat_bytes = fs::read(&chat_tmp).expect("read chat");
    fs::remove_file(&chat_tmp).expect("remove tmp");
    let chat_path = root.join(&CHAT_ID[..2]).join(CHAT_ID);
    fs::create_dir_all(chat_path.parent().unwrap()).expect("dir");
    fs::write(&chat_path, &chat_bytes).expect("write chat");

    let mut rows = format!("('{CHAT_ID}', '{domain}', 'ChatStorage.sqlite')");

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

fn two_messages() -> WaData {
    WaData {
        sessions: vec![(1, "+306912345678@s.whatsapp.net", Some("Nikos P"), false)],
        members: vec![],
        messages: vec![
            (
                1,
                1,
                false,
                1_700_000_000,
                0,
                0,
                Some("Hello"),
                Some("+306912345678@s.whatsapp.net"),
            ),
            (
                2,
                1,
                true,
                1_700_000_010,
                0,
                0,
                Some("Hi, how can I help?"),
                None,
            ),
        ],
        push: vec![(1, "+306912345678@s.whatsapp.net", "Nikos P")],
    }
}

#[test]
fn available_apps_reports_only_present_domains() {
    let dir = tempdir().unwrap();
    make_backup(dir.path(), WHATSAPP_DOMAIN, &two_messages(), Some(ADDR_SQL));
    let apps = available_apps(dir.path()).expect("available_apps");
    assert_eq!(apps, vec![ChatProvider::WhatsApp]);
}

#[test]
fn available_apps_reports_business_when_present() {
    let dir = tempdir().unwrap();
    make_backup(
        dir.path(),
        WHATSAPP_BUSINESS_DOMAIN,
        &two_messages(),
        Some(ADDR_SQL),
    );
    let apps = available_apps(dir.path()).expect("available_apps");
    assert_eq!(apps, vec![ChatProvider::WhatsAppBusiness]);
}

#[test]
fn available_apps_is_empty_for_a_backup_with_no_whatsapp() {
    let dir = tempdir().unwrap();
    // A manifest with no WhatsApp domain at all.
    let manifest = dir.path().join("Manifest.db");
    let mc = Connection::open(&manifest).expect("open manifest");
    mc.execute_batch(
        "CREATE TABLE Files (fileID TEXT, domain TEXT, relativePath TEXT);
         INSERT INTO Files VALUES ('cc0000000000000000000000000000000000000000', 'HomeDomain', 'Library/SMS/sms.db');",
    )
    .expect("build manifest");
    drop(mc);
    let apps = available_apps(dir.path()).expect("available_apps");
    assert!(apps.is_empty());
}

#[test]
fn read_yields_one_session_per_conversation() {
    let dir = tempdir().unwrap();
    make_backup(dir.path(), WHATSAPP_DOMAIN, &two_messages(), Some(ADDR_SQL));
    let sessions = read(dir.path(), ChatProvider::WhatsApp, Some("30")).expect("read");
    assert_eq!(sessions.len(), 1);
    let s = &sessions[0];
    assert_eq!(s.provider, ChatProvider::WhatsApp);
    assert_eq!(s.title, "Nikos P");
    assert!(s.raw_slice.is_some(), "every session carries its raw slice");
    assert!(s.fill_estimated);
    // The project override is the resolved client's organization.
    assert_eq!(s.project_override.as_deref(), Some("ABC Marble Ltd"));
    // Two messages: the incoming (User) and the outgoing (Assistant / Me).
    assert_eq!(s.messages.len(), 2);
    assert_eq!(s.messages[0].role, MessageRole::User);
    assert_eq!(
        s.messages[0].speaker.as_deref(),
        Some("Nikos P | +306912345678 | 6912345678")
    );
    assert_eq!(s.messages[1].role, MessageRole::Assistant);
    assert_eq!(s.messages[1].speaker.as_deref(), Some("Me"));
}

#[test]
fn read_is_a_real_error_for_a_non_whatsapp_provider() {
    let dir = tempdir().unwrap();
    make_backup(dir.path(), WHATSAPP_DOMAIN, &two_messages(), Some(ADDR_SQL));
    let err = read(dir.path(), ChatProvider::AppleMessages, Some("30")).expect_err("should fail");
    assert!(matches!(err, crate::readers::ReaderError::Database(_)));
}

#[test]
fn a_missing_chat_storage_is_a_real_error() {
    let dir = tempdir().unwrap();
    // A backup with no ChatStorage in the personal domain.
    let manifest = dir.path().join("Manifest.db");
    let mc = Connection::open(&manifest).expect("open manifest");
    mc.execute_batch(
        "CREATE TABLE Files (fileID TEXT, domain TEXT, relativePath TEXT);
         INSERT INTO Files VALUES ('cc0000000000000000000000000000000000000000', 'HomeDomain', 'Library/SMS/sms.db');",
    )
    .expect("build manifest");
    drop(mc);
    let err = read(dir.path(), ChatProvider::WhatsApp, Some("30")).expect_err("should fail");
    assert!(matches!(err, crate::readers::ReaderError::Database(_)));
}

#[test]
fn a_gap_over_30_days_splits_into_two_sessions() {
    let data = WaData {
        sessions: vec![(1, "+306912345678@s.whatsapp.net", Some("Nikos P"), false)],
        members: vec![],
        messages: vec![
            (
                1,
                1,
                false,
                1_700_000_000,
                0,
                0,
                Some("first"),
                Some("+306912345678@s.whatsapp.net"),
            ),
            // 31 days later: a new window.
            (
                2,
                1,
                false,
                1_700_000_000 + 31 * 86_400,
                0,
                0,
                Some("second"),
                Some("+306912345678@s.whatsapp.net"),
            ),
        ],
        push: vec![(1, "+306912345678@s.whatsapp.net", "Nikos P")],
    };
    let dir = tempdir().unwrap();
    make_backup(dir.path(), WHATSAPP_DOMAIN, &data, Some(ADDR_SQL));
    let sessions = read(dir.path(), ChatProvider::WhatsApp, Some("30")).expect("read");
    assert_eq!(sessions.len(), 2);
}

/// The session id is `<provider_key>-<sanitized first guid>`; the WhatsApp
/// guid is `wa-<rowid>`, so the id is `whatsapp-wa-<rowid>`.
#[test]
fn the_session_id_is_prefixed_with_the_provider_key() {
    let dir = tempdir().unwrap();
    make_backup(dir.path(), WHATSAPP_DOMAIN, &two_messages(), Some(ADDR_SQL));
    let sessions = read(dir.path(), ChatProvider::WhatsApp, Some("30")).expect("read");
    assert_eq!(sessions[0].id, "whatsapp-wa-1");
}

/// The reader leaves the backup directory untouched (read-only on every
/// source): no temp copy or journal is left behind in the backup.
#[test]
fn the_backup_dir_is_not_modified() {
    let dir = tempdir().unwrap();
    make_backup(dir.path(), WHATSAPP_DOMAIN, &two_messages(), Some(ADDR_SQL));
    let before: Vec<PathBuf> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let _ = read(dir.path(), ChatProvider::WhatsApp, Some("30")).expect("read");
    let after: Vec<PathBuf> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(before, after);
}

/// A `ParsedSession` helper the tests above rely on: the reader sets
/// `raw_slice` and `project_override` (the business-messaging contract).
#[test]
fn the_session_carries_business_messaging_fields() {
    let dir = tempdir().unwrap();
    make_backup(dir.path(), WHATSAPP_DOMAIN, &two_messages(), Some(ADDR_SQL));
    let sessions = read(dir.path(), ChatProvider::WhatsApp, Some("30")).expect("read");
    let s: &ParsedSession = &sessions[0];
    assert!(s.raw_slice.is_some());
    assert!(s.project_override.is_some());
}
