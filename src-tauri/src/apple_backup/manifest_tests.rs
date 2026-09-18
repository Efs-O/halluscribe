// HalluScribe - synthetic tests for the backup resolver (no real backup data).

use super::{open_backup, BackupError};
use rusqlite::Connection;
use std::fs;
use std::path::Path;

const SMS_ID: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678"; // 40 hex

/// Build a fake backup dir: a real SQLite `Manifest.db` with a `Files` table
/// plus hashed payload files. Returns the directory.
fn make_backup(root: &Path) {
    let manifest = root.join("Manifest.db");
    let conn = Connection::open(&manifest).expect("open manifest");
    conn.execute_batch(
        "CREATE TABLE Files (fileID TEXT, domain TEXT, relativePath TEXT);
         INSERT INTO Files VALUES
           ('a1b2c3d4e5f60718293a4b5c6d7e8f9012345678', 'HomeDomain', 'Library/SMS/sms.db'),
           ('bbbb1111222233334444555566667777', 'HomeDomain', 'Library/AddressBook/AddressBook.sqlitedb'),
           ('cccc1111222233334444555566667777', 'AppDomain-group.net.whatsapp.WhatsApp.shared', 'ChatStorage.sqlite'),
           ('dddd1111222233334444555566667777', 'AppDomain-group.net.viber.Viber.shared', 'AppDomain-group.net.viber.Viber.shared/Contacts.data');",
    )
    .expect("create Files table");
    drop(conn);

    let sms = root.join("a1").join(SMS_ID);
    fs::create_dir_all(sms.parent().unwrap()).expect("create hash dir");
    fs::write(&sms, b"fake sms db bytes").expect("write sms.db");
    let addr = root.join("bb").join("bbbb1111222233334444555566667777");
    fs::create_dir_all(addr.parent().unwrap()).expect("create hash dir");
    fs::write(&addr, b"fake address book").expect("write address book");
}

#[test]
fn resolve_hit_returns_hashed_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    make_backup(dir.path());
    let handle = open_backup(dir.path()).expect("open backup");
    let got = handle
        .resolve("HomeDomain", "Library/SMS/sms.db")
        .expect("should resolve");
    assert_eq!(got, dir.path().join("a1").join(SMS_ID));
    assert!(got.is_file());
}

#[test]
fn resolve_miss_when_file_missing_on_disk() {
    let dir = tempfile::tempdir().expect("tempdir");
    make_backup(dir.path());
    // Manifest row exists (whatsapp ChatStorage) but the hashed file was
    // never written to disk, so resolve must miss.
    let handle = open_backup(dir.path()).expect("open backup");
    assert!(handle
        .resolve(
            "AppDomainGroup-group.net.whatsapp.WhatsApp.shared",
            "ChatStorage.sqlite"
        )
        .is_none());
}

#[test]
fn resolve_miss_when_no_manifest_row() {
    let dir = tempfile::tempdir().expect("tempdir");
    make_backup(dir.path());
    let handle = open_backup(dir.path()).expect("open backup");
    assert!(handle
        .resolve("HomeDomain", "Library/Nope/missing.db")
        .is_none());
}

#[test]
fn non_sqlite_header_is_encrypted() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("Manifest.db"), b"not a sqlite file at all").expect("write manifest");
    let err = open_backup(dir.path()).expect_err("should be encrypted");
    assert!(matches!(err, BackupError::Encrypted));
}

#[test]
fn missing_manifest_is_not_a_backup() {
    let dir = tempfile::tempdir().expect("tempdir");
    let err = open_backup(dir.path()).expect_err("should not be a backup");
    assert!(matches!(err, BackupError::NotABackup));
}

#[test]
fn domains_matching_is_case_insensitive_sorted_deduped() {
    let dir = tempfile::tempdir().expect("tempdir");
    make_backup(dir.path());
    let handle = open_backup(dir.path()).expect("open backup");
    let got = handle.domains_matching("whats");
    assert_eq!(
        got,
        vec!["AppDomain-group.net.whatsapp.WhatsApp.shared".to_string()]
    );
    // Case-insensitive: uppercase needle matches the mixed-case domain.
    assert_eq!(handle.domains_matching("WHATS"), got);
    // No match.
    assert!(handle.domains_matching("zzz").is_empty());
}

#[test]
fn tempcopy_file_is_gone_after_drop() {
    let dir = tempfile::tempdir().expect("tempdir");
    make_backup(dir.path());
    let handle = open_backup(dir.path()).expect("open backup");
    let path = {
        let copy = handle
            .copy_to_temp("HomeDomain", "Library/SMS/sms.db")
            .expect("copy");
        let p = copy.path().to_path_buf();
        assert!(p.is_file(), "temp copy should exist while alive");
        p
    };
    assert!(!path.is_file(), "temp copy should be deleted on drop");
}

#[test]
fn copy_to_temp_leaves_source_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    make_backup(dir.path());
    let handle = open_backup(dir.path()).expect("open backup");
    let source = dir.path().join("a1").join(SMS_ID);
    let before = fs::read(&source).expect("read source");
    let copy = handle
        .copy_to_temp("HomeDomain", "Library/SMS/sms.db")
        .expect("copy");
    assert_eq!(fs::read(copy.path()).expect("read copy"), before);
    assert_eq!(fs::read(&source).expect("re-read source"), before);
}
