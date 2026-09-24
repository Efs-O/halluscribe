// HalluScribe - synthetic tests for WAL copying and the shared manifest index
// of the backup resolver (no real backup data).

use super::open_backup;
use rusqlite::Connection;
use std::fs;
use std::path::Path;
use std::sync::Arc;

const DB_ID: &str = "e1e2e3e4e5e6e7e8e9e0e1e2e3e4e5e6e7e8e9e0";
const WAL_ID: &str = "f1f2f3f4f5f6f7f8f9f0f1f2f3f4f5f6f7f8f9f0";

/// Write `Manifest.db` listing `rows` as (fileID, domain, relativePath).
fn write_manifest(root: &Path, rows: &[(&str, &str, &str)]) {
    let conn = Connection::open(root.join("Manifest.db")).expect("open manifest");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS Files (fileID TEXT, domain TEXT, relativePath TEXT)",
    )
    .expect("create Files");
    for row in rows {
        conn.execute(
            "INSERT INTO Files VALUES (?1, ?2, ?3)",
            [row.0, row.1, row.2],
        )
        .expect("insert file row");
    }
}

/// Place a payload file at its hashed backup path.
fn place(root: &Path, file_id: &str, source: &Path) {
    let dest = root.join(&file_id[..2]).join(file_id);
    fs::create_dir_all(dest.parent().unwrap()).expect("create hash dir");
    fs::copy(source, dest).expect("copy payload");
}

/// A backup holding `Library/t.db` with one checkpointed row and one row that
/// exists only in its `-wal`, as a phone backup taken mid-write would.
fn backup_with_wal(root: &Path) {
    let work = tempfile::tempdir().expect("work dir");
    let db = work.path().join("t.db");
    let conn = Connection::open(&db).expect("open db");
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
         CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1);
         PRAGMA wal_checkpoint(TRUNCATE);
         INSERT INTO t VALUES (2);",
    )
    .expect("fill db");
    // Copy while the connection is open, so the second row stays in the WAL.
    place(root, DB_ID, &db);
    place(root, WAL_ID, &work.path().join("t.db-wal"));
    drop(conn);
    write_manifest(
        root,
        &[
            (DB_ID, "HomeDomain", "Library/t.db"),
            (WAL_ID, "HomeDomain", "Library/t.db-wal"),
        ],
    );
}

fn row_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
        .expect("count rows")
}

#[test]
fn rows_only_in_the_backed_up_wal_are_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    backup_with_wal(dir.path());
    let handle = open_backup(dir.path()).expect("open backup");
    let copy = handle
        .copy_to_temp("HomeDomain", "Library/t.db")
        .expect("copy");
    assert_eq!(row_count(&copy.open().expect("open copy")), 2);
    // The backup's own files are untouched: no `-shm` appears beside them.
    assert!(!dir.path().join("e1").join(format!("{DB_ID}-shm")).exists());
}

#[test]
fn the_wal_and_shm_copies_are_gone_after_drop() {
    let dir = tempfile::tempdir().expect("tempdir");
    backup_with_wal(dir.path());
    let handle = open_backup(dir.path()).expect("open backup");
    let copy = handle
        .copy_to_temp("HomeDomain", "Library/t.db")
        .expect("copy");
    let conn = copy.open().expect("open copy");
    row_count(&conn);
    let wal = super::temp_copy::sibling(copy.path(), "-wal");
    let shm = super::temp_copy::sibling(copy.path(), "-shm");
    assert!(wal.is_file(), "the WAL copy exists while alive");
    drop(conn);
    drop(copy);
    assert!(!wal.exists() && !shm.exists());
}

#[test]
fn live_handles_share_one_manifest_index() {
    let dir = tempfile::tempdir().expect("tempdir");
    backup_with_wal(dir.path());
    let a = open_backup(dir.path()).expect("open a");
    let b = open_backup(dir.path()).expect("open b");
    assert!(Arc::ptr_eq(&a.manifest, &b.manifest));
}

#[test]
fn a_rewritten_manifest_is_read_again() {
    let dir = tempfile::tempdir().expect("tempdir");
    backup_with_wal(dir.path());
    let a = open_backup(dir.path()).expect("open a");
    assert!(a.resolve("HomeDomain", "Library/new.db").is_none());
    // A new backup adds a file: the manifest grows, so the index is re-read.
    let payload = dir.path().join("payload");
    fs::write(&payload, b"new").expect("write payload");
    place(dir.path(), "a0a0", &payload);
    write_manifest(dir.path(), &[("a0a0", "HomeDomain", "Library/new.db")]);
    let b = open_backup(dir.path()).expect("open b");
    assert!(!Arc::ptr_eq(&a.manifest, &b.manifest));
    assert!(b.resolve("HomeDomain", "Library/new.db").is_some());
}
