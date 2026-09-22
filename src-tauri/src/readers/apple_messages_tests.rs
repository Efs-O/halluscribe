// HalluScribe - end-to-end tests for the Apple Messages reader. All fixtures
// are synthetic: a temp backup dir with a real `Manifest.db`, a real `sms.db`
// and (optionally) a real `AddressBook.sqlitedb`, built in the test. No real
// data, names, numbers or messages.

use super::read;
use crate::readers::ChatProvider;
use rusqlite::Connection;
use std::fs;
use std::path::Path;

const SMS_ID: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";
const ADDR_ID: &str = "bbbb1111222233334444555566667777";

const SMS_SCHEMA: &str = r#"
CREATE TABLE message (ROWID INTEGER PRIMARY KEY, guid TEXT, text TEXT, attributedBody BLOB,
    handle_id INTEGER, date INTEGER, is_from_me INTEGER, service TEXT,
    associated_message_type INTEGER, item_type INTEGER);
CREATE TABLE handle (ROWID INTEGER PRIMARY KEY, id TEXT, service TEXT);
CREATE TABLE chat (ROWID INTEGER PRIMARY KEY, guid TEXT, chat_identifier TEXT,
    display_name TEXT, service_name TEXT);
CREATE TABLE chat_message_join (chat_id INTEGER, message_id INTEGER);
CREATE TABLE chat_handle_join (chat_id INTEGER, handle_id INTEGER);
CREATE TABLE attachment (ROWID INTEGER PRIMARY KEY, filename TEXT, mime_type TEXT,
    transfer_name TEXT, total_bytes INTEGER);
CREATE TABLE message_attachment_join (message_id INTEGER, attachment_id INTEGER);
"#;

const ADDR_SCHEMA: &str = r#"
CREATE TABLE ABPerson (ROWID INTEGER PRIMARY KEY, First TEXT, Last TEXT, Organization TEXT);
CREATE TABLE ABMultiValue (record_id INTEGER, property INTEGER, value TEXT);
"#;

/// One attachment spec: (ROWID, transfer_name, mime_type, total_bytes).
type Attachment = (i64, Option<&'static str>, Option<&'static str>, Option<i64>);

/// A compact spec for a synthetic sms.db.
#[derive(Default)]
struct Sms {
    handles: Vec<(i64, &'static str)>,
    chats: Vec<(i64, &'static str, Option<&'static str>)>,
    chat_msg: Vec<(i64, i64)>,
    chat_handle: Vec<(i64, i64)>,
    msgs: Vec<Msg>,
    attachments: Vec<Attachment>,
    msg_att: Vec<(i64, i64)>,
}

#[derive(Default)]
struct Msg {
    rowid: i64,
    guid: &'static str,
    text: Option<&'static str>,
    handle_id: i64,
    date: i64,
    from_me: bool,
}

impl Sms {
    fn sql(&self) -> String {
        let mut s = String::from(SMS_SCHEMA);
        for (id, h) in &self.handles {
            s.push_str(&format!(
                "INSERT INTO handle (ROWID, id, service) VALUES ({id}, '{h}', 'SMS');"
            ));
        }
        for (id, guid, display) in &self.chats {
            let d = display.unwrap_or("NULL");
            let d = if d == "NULL" {
                "NULL".to_string()
            } else {
                format!("'{d}'")
            };
            s.push_str(&format!(
                "INSERT INTO chat (ROWID, guid, chat_identifier, display_name, service_name) VALUES ({id}, '{guid}', '{guid}', {d}, 'SMS');"
            ));
        }
        for (c, m) in &self.chat_msg {
            s.push_str(&format!("INSERT INTO chat_message_join VALUES ({c}, {m});"));
        }
        for (c, h) in &self.chat_handle {
            s.push_str(&format!("INSERT INTO chat_handle_join VALUES ({c}, {h});"));
        }
        for m in &self.msgs {
            let t = m.text.unwrap_or("NULL");
            let t = if m.text.is_none() {
                "NULL".to_string()
            } else {
                format!("'{t}'")
            };
            s.push_str(&format!(
                "INSERT INTO message (ROWID, guid, text, attributedBody, handle_id, date, is_from_me, service, associated_message_type, item_type)
                 VALUES ({} , '{}', {}, NULL, {}, {}, {}, 'SMS', 0, 0);",
                m.rowid, m.guid, t, m.handle_id, m.date, m.from_me as i64
            ));
        }
        for (id, transfer, mime, bytes) in &self.attachments {
            let tr = transfer
                .map(|t| format!("'{t}'"))
                .unwrap_or_else(|| "NULL".into());
            let mi = mime
                .map(|t| format!("'{t}'"))
                .unwrap_or_else(|| "NULL".into());
            let by = bytes
                .map(|b| b.to_string())
                .unwrap_or_else(|| "NULL".into());
            s.push_str(&format!(
                "INSERT INTO attachment (ROWID, filename, mime_type, transfer_name, total_bytes) VALUES ({id}, '/private/var/xx', {mi}, {tr}, {by});"
            ));
        }
        for (m, a) in &self.msg_att {
            s.push_str(&format!(
                "INSERT INTO message_attachment_join VALUES ({m}, {a});"
            ));
        }
        s
    }
}

/// A compact spec for a synthetic AddressBook.
struct Addr {
    people: Vec<(i64, &'static str, &'static str, Option<&'static str>)>,
    values: Vec<(i64, i64, &'static str)>,
}

impl Addr {
    fn sql(&self) -> String {
        let mut s = String::from(ADDR_SCHEMA);
        for (id, first, last, org) in &self.people {
            let o = org
                .map(|x| format!("'{x}'"))
                .unwrap_or_else(|| "NULL".into());
            s.push_str(&format!(
                "INSERT INTO ABPerson (ROWID, First, Last, Organization) VALUES ({id}, '{first}', '{last}', {o});"
            ));
        }
        for (rec, prop, value) in &self.values {
            s.push_str(&format!(
                "INSERT INTO ABMultiValue (record_id, property, value) VALUES ({rec}, {prop}, '{value}');"
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

/// Build a fake backup dir: a real `Manifest.db` plus hashed payload files
/// holding real `sms.db` (and, if given, `AddressBook.sqlitedb`) bytes.
fn make_backup(root: &Path, sms: &Sms, addr: Option<&Addr>) {
    // Idempotent: a re-read of the same dir must not find a stale manifest or
    // payload from the previous build.
    let _ = fs::remove_file(root.join("Manifest.db"));
    let _ = fs::remove_dir_all(root.join(&SMS_ID[..2]));
    let _ = fs::remove_dir_all(root.join(&ADDR_ID[..2]));

    let sms_tmp = root.join("__sms.db");
    write_sqlite(&sms_tmp, &sms.sql());
    let sms_bytes = fs::read(&sms_tmp).expect("read sms");
    fs::remove_file(&sms_tmp).expect("remove tmp");

    let addr_bytes = match addr {
        Some(a) => {
            let t = root.join("__addr.db");
            write_sqlite(&t, &a.sql());
            let b = fs::read(&t).expect("read addr");
            fs::remove_file(&t).expect("remove tmp");
            Some(b)
        }
        None => None,
    };

    let sms_path = root.join(&SMS_ID[..2]).join(SMS_ID);
    fs::create_dir_all(sms_path.parent().unwrap()).expect("dir");
    fs::write(&sms_path, &sms_bytes).expect("write sms");

    let mut rows = format!("('{SMS_ID}', 'HomeDomain', 'Library/SMS/sms.db')");
    if let Some(b) = &addr_bytes {
        let addr_path = root.join(&ADDR_ID[..2]).join(ADDR_ID);
        fs::create_dir_all(addr_path.parent().unwrap()).expect("dir");
        fs::write(&addr_path, b).expect("write addr");
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

/// The address book with one phone contact (Nikos, org ABC Marble Ltd) and one
/// email contact (Maria).
fn addr_book() -> Addr {
    Addr {
        people: vec![
            (1, "Nikos", "Papadopoulos", Some("ABC Marble Ltd")),
            (2, "Maria", "", None),
        ],
        values: vec![(1, 3, "+306912345678"), (2, 4, "maria@example.com")],
    }
}

/// A backup with a 1:1 (Nikos), a group (The Team), and an orphan.
fn three_conversations() -> Sms {
    Sms {
        handles: vec![
            (1, "+306912345678"),
            (2, "+306912345678"),
            (3, "+15551234567"),
            (4, "+15559999999"),
        ],
        chats: vec![
            (1, "iMessage;-;+306912345678", None),
            (2, "group", Some("The Team")),
        ],
        chat_msg: vec![(1, 1), (1, 2), (2, 3)],
        chat_handle: vec![(1, 1), (2, 2), (2, 3)],
        msgs: vec![
            Msg {
                rowid: 1,
                guid: "one-me",
                text: Some("hello there"),
                handle_id: 0,
                date: 10_000_000,
                from_me: true,
            },
            Msg {
                rowid: 2,
                guid: "one-them",
                text: Some("hi, how can I help"),
                handle_id: 0,
                date: 10_000_100,
                from_me: false,
            },
            Msg {
                rowid: 3,
                guid: "group-msg",
                text: Some("hey team"),
                handle_id: 3,
                date: 10_000_200,
                from_me: false,
            },
            Msg {
                rowid: 4,
                guid: "orphan-msg",
                text: Some("orphan hello"),
                handle_id: 4,
                date: 10_000_300,
                from_me: false,
            },
        ],
        ..Default::default()
    }
}

#[test]
fn end_to_end_yields_one_session_per_conversation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sms = three_conversations();
    make_backup(dir.path(), &sms, Some(&addr_book()));

    let sessions = read(dir.path(), Some("30")).expect("read");
    assert_eq!(sessions.len(), 3, "one session per conversation");
    for s in &sessions {
        assert_eq!(s.provider, ChatProvider::AppleMessages);
        assert!(s.raw_slice.is_some(), "every session carries its raw slice");
        assert!(s.fill_estimated);
    }

    let by_title: std::collections::HashMap<&str, &crate::readers::ParsedSession> =
        sessions.iter().map(|s| (s.title.as_str(), s)).collect();

    // 1:1: resolved name, org override, "Me" + resolved speaker.
    let one = by_title["Nikos Papadopoulos"];
    assert_eq!(one.project_override.as_deref(), Some("ABC Marble Ltd"));
    assert_eq!(one.messages.len(), 2);
    assert_eq!(one.messages[0].speaker.as_deref(), Some("Me"));
    assert_eq!(
        one.messages[1].speaker.as_deref(),
        Some("Nikos Papadopoulos")
    );
    // The raw header carries the E.164 and the org.
    let raw = one.raw_slice.as_deref().unwrap();
    assert!(raw.starts_with("# Messages — Nikos Papadopoulos — ABC Marble Ltd\n"));
    assert!(raw.contains("participants: Me; Nikos Papadopoulos (+306912345678)"));

    // Group: display name, no org override, unresolved participant verbatim.
    let group = by_title["The Team"];
    assert_eq!(group.project_override, None);
    assert_eq!(group.messages[0].speaker.as_deref(), Some("+15551234567"));
    assert!(group
        .raw_slice
        .as_deref()
        .unwrap()
        .contains("participants: Me; Nikos Papadopoulos (+306912345678); +15551234567"));

    // Orphan: raw handle verbatim, no org.
    let orphan = by_title["+15559999999"];
    assert_eq!(orphan.project_override, None);
    assert_eq!(orphan.messages[0].speaker.as_deref(), Some("+15559999999"));
}

#[test]
fn attachment_line_appears_in_the_raw_and_transcript() {
    let mut sms = three_conversations();
    // Attach a photo to the 1:1 "hello there" message (rowid 1).
    sms.attachments
        .push((1, Some("IMG_0001.jpg"), Some("image/jpeg"), Some(2_100_000)));
    sms.msg_att.push((1, 1));
    let dir = tempfile::tempdir().expect("tempdir");
    make_backup(dir.path(), &sms, Some(&addr_book()));

    let sessions = read(dir.path(), Some("30")).expect("read");
    let one = sessions
        .iter()
        .find(|s| s.title == "Nikos Papadopoulos")
        .unwrap();
    let raw = one.raw_slice.as_deref().unwrap();
    assert!(raw.contains("[attachment: IMG_0001.jpg · image/jpeg · 2.1 MB]"));
    // The same line is folded into the transcript text.
    assert!(one
        .transcript()
        .contains("[attachment: IMG_0001.jpg · image/jpeg · 2.1 MB]"));
}

#[test]
fn missing_address_book_still_reads_with_verbatim_speakers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sms = three_conversations();
    make_backup(dir.path(), &sms, None); // no AddressBook.sqlitedb

    let sessions = read(dir.path(), Some("30")).expect("read");
    assert_eq!(sessions.len(), 3);
    // The 1:1 is now titled by the raw handle (unresolved), with no org.
    let one = sessions
        .iter()
        .find(|s| s.title == "+306912345678")
        .expect("unresolved 1:1");
    assert_eq!(one.project_override, None);
    assert_eq!(one.messages[1].speaker.as_deref(), Some("+306912345678"));
}

#[test]
fn re_read_keeps_closed_windows_stable_and_grows_only_the_last() {
    // One conversation spanning a >30-day gap ⇒ two windows.
    let gap = 40 * 86_400;
    let base = 10_000_000;
    let mut sms = Sms {
        handles: vec![(1, "+306912345678")],
        chats: vec![(1, "iMessage;-;+306912345678", None)],
        chat_msg: vec![(1, 1), (1, 2)],
        chat_handle: vec![(1, 1)],
        msgs: vec![
            Msg {
                rowid: 1,
                guid: "w1",
                text: Some("first window"),
                handle_id: 0,
                date: base,
                from_me: false,
            },
            Msg {
                rowid: 2,
                guid: "w2",
                text: Some("second window"),
                handle_id: 0,
                date: base + gap,
                from_me: false,
            },
        ],
        ..Default::default()
    };

    let dir = tempfile::tempdir().expect("tempdir");
    make_backup(dir.path(), &sms, Some(&addr_book()));
    let first = read(dir.path(), Some("30")).expect("read");
    assert_eq!(first.len(), 2, "a >30-day gap splits into two windows");
    let (id_a, hash_a) = (first[0].id.clone(), first[0].transcript_hash.clone());
    let (id_b, _hash_b) = (first[1].id.clone(), first[1].transcript_hash.clone());

    // Append one message to the SECOND window and re-read.
    sms.chat_msg.push((1, 3));
    sms.msgs.push(Msg {
        rowid: 3,
        guid: "w2b",
        text: Some("appended"),
        handle_id: 0,
        date: base + gap + 100,
        from_me: false,
    });
    make_backup(dir.path(), &sms, Some(&addr_book()));
    let second = read(dir.path(), Some("30")).expect("read");
    assert_eq!(second.len(), 2);

    // The earlier window is byte-for-byte stable: same id and transcript hash.
    assert_eq!(second[0].id, id_a);
    assert_eq!(second[0].transcript_hash, hash_a);
    // The last window keeps its id (anchored on its first message) but its
    // transcript hash changes because a message was added.
    assert_eq!(second[1].id, id_b);
    assert_ne!(second[1].transcript_hash, _hash_b);
    assert_eq!(second[1].messages.len(), 2);
}

#[test]
fn raw_slices_for_source_yields_one_slice_per_session_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sms = three_conversations();
    make_backup(dir.path(), &sms, Some(&addr_book()));

    let slices = crate::readers::raw_slices_for_source(dir.path(), "apple_messages", Some("30"))
        .expect("apple_messages is multi-session");
    assert_eq!(slices.len(), 3, "one slice per session");
    for s in slices.values() {
        assert!(s.starts_with("# Messages — "));
    }
}

#[test]
fn missing_sms_db_is_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A backup with a manifest but no sms.db payload.
    let manifest = dir.path().join("Manifest.db");
    let mc = Connection::open(&manifest).expect("open");
    mc.execute_batch("CREATE TABLE Files (fileID TEXT, domain TEXT, relativePath TEXT);")
        .expect("build");
    drop(mc);
    assert!(read(dir.path(), Some("30")).is_err());
}
