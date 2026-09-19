// HalluScribe - `halluscribe-backup-probe`: a read-only dev binary that
// inspects an iPhone backup and prints METADATA ONLY.
//
// Usage: halluscribe-backup-probe <backup-dir>
//
// It reports whether the backup is encrypted, the iOS version and last-backup
// date from Info.plist, which of the known logical files resolve, sms.db table
// sizes and date range, and the Viber/WhatsApp domains and their database
// files. It NEVER prints message text, handles, names, phone numbers, emails
// or any column value other than counts and dates. The path comes only from
// argv - there is no default and no hardcoded path.

use app_lib::apple_backup::{
    decode_attributed_body, open_backup, open_sqlite_read_only, plist_value, BackupError,
};
use chrono::TimeZone;
use chrono::Utc;
use rusqlite::Connection;
use std::fs;
use std::path::Path;
use std::process::exit;

/// iOS core-data epoch: 2001-01-01 00:00:00 UTC, in seconds.
const IOS_EPOCH_SECS: i64 = 978_307_200;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args.len() > 3 {
        eprintln!("usage: halluscribe-backup-probe <backup-dir> [domain-filter]");
        exit(2);
    }
    let dir = Path::new(&args[1]);
    // Optional: only print domains whose name contains this substring
    // (e.g. "WhatsAppSMB") so a single app's schema can be read in isolation.
    let domain_filter: Option<&str> = args.get(2).map(|s| s.as_str());

    let handle = match open_backup(dir) {
        Ok(h) => h,
        Err(BackupError::Encrypted) => {
            println!("encrypted: yes");
            return;
        }
        Err(e) => {
            eprintln!("error: {e}");
            exit(1);
        }
    };

    println!("encrypted: no");
    let info = dir.join("Info.plist");
    if info.is_file() {
        match fs::read_to_string(&info) {
            Ok(text) => {
                println!("ios_version: {}", plist_value(&text, "Product Version"));
                println!(
                    "last_backup_date: {}",
                    plist_value(&text, "Last Backup Date")
                );
            }
            Err(e) => println!("info_plist: unreadable: {e}"),
        }
    } else {
        println!("info_plist: missing");
    }

    let logical = [
        ("HomeDomain", "Library/SMS/sms.db"),
        ("HomeDomain", "Library/AddressBook/AddressBook.sqlitedb"),
        (
            "AppDomainGroup-group.net.whatsapp.WhatsAppSMB.shared",
            "ChatStorage.sqlite",
        ),
        (
            "AppDomainGroup-group.net.whatsapp.WhatsApp.shared",
            "ChatStorage.sqlite",
        ),
    ];
    for (domain, rel) in logical {
        let resolved = handle.resolve(domain, rel).is_some();
        println!(
            "logical {domain}/{rel}: resolved {}",
            if resolved { "yes" } else { "no" }
        );
    }

    // SMS attachments live under a directory (MediaDomain/Library/SMS/
    // Attachments/), so they are counted, not resolved as a single file.
    let attachments = handle
        .files_in_domain("MediaDomain")
        .iter()
        .filter(|rel| rel.starts_with("Library/SMS/Attachments/"))
        .filter(|rel| handle.resolve("MediaDomain", rel).is_some())
        .count();
    println!("sms attachments: {attachments} files");

    // sms.db stats - only if it resolves.
    if handle.resolve("HomeDomain", "Library/SMS/sms.db").is_some() {
        match handle.copy_to_temp("HomeDomain", "Library/SMS/sms.db") {
            Ok(sms) => match open_sqlite_read_only(sms.path()) {
                Ok(conn) => print_sms_stats(&conn),
                Err(e) => println!("sms.db: unreadable: {e}"),
            },
            Err(e) => println!("sms.db: unreadable: {e}"),
        }
    }

    // Viber / WhatsApp domains and their database files.
    for needle in ["viber", "whatsapp"] {
        for domain in handle.domains_matching(needle) {
            if let Some(f) = domain_filter {
                if !domain.contains(f) {
                    continue;
                }
            }
            println!("domain: {domain}");
            for rel in handle.files_in_domain(&domain) {
                if !is_db_extension(&rel) {
                    continue;
                }
                match handle.copy_to_temp(&domain, &rel) {
                    Ok(copy) => match open_sqlite_read_only(copy.path()) {
                        Ok(conn) => {
                            print_table_list(&rel, &conn);
                            if rel == "ChatStorage.sqlite" {
                                whatsapp_message_stats(&rel, &conn);
                            }
                        }
                        Err(e) => println!("  {rel}: unreadable: {e}"),
                    },
                    Err(e) => println!("  {rel}: unreadable: {e}"),
                }
            }
        }
    }
}

/// WhatsApp `ChatStorage.sqlite`: print the `ZWAMESSAGE` date range (as raw
/// integers, so the epoch unit is visible) and the `ZMESSAGETYPE` /
/// `ZGROUPEVENTTYPE` value distributions. Metadata only - never a message
/// text, JID, name or number. This is the Phase 7 column/data recon the plan
/// anticipated (I.3 is a guess, not a contract).
fn whatsapp_message_stats(rel: &str, conn: &Connection) {
    if !table_has_column(conn, "ZWAMESSAGE", "ZMESSAGEDATE") {
        return;
    }
    match conn.query_row(
        "SELECT MIN(ZMESSAGEDATE), MAX(ZMESSAGEDATE), COUNT(*) FROM ZWAMESSAGE",
        [],
        |r| {
            Ok((
                r.get::<_, Option<f64>>(0)?,
                r.get::<_, Option<f64>>(1)?,
                r.get::<_, i64>(2)?,
            ))
        },
    ) {
        Ok((min, max, count)) => {
            println!("  {rel}: ZWAMESSAGE {count} rows");
            println!(
                "    ZMESSAGEDATE raw range (REAL): {} .. {} (epoch unit inferred from magnitude)",
                min.map(|v| v.to_string()).unwrap_or_default(),
                max.map(|v| v.to_string()).unwrap_or_default()
            );
            for column in ["ZMESSAGETYPE", "ZGROUPEVENTTYPE"] {
                if !table_has_column(conn, "ZWAMESSAGE", column) {
                    continue;
                }
                let sql = format!(
                    "SELECT {column}, COUNT(*) FROM ZWAMESSAGE GROUP BY {column} ORDER BY {column}"
                );
                let Ok(mut stmt) = conn.prepare(&sql) else {
                    continue;
                };
                let Ok(rows) = stmt
                    .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
                    .map(|it| it.collect::<Result<Vec<_>, _>>().unwrap_or_default())
                else {
                    continue;
                };
                let dist: Vec<String> = rows.iter().map(|(v, c)| format!("{v}={c}")).collect();
                println!("    {column}: {}", dist.join(", "));
            }
        }
        Err(e) => println!("  {rel}: ZWAMESSAGE unreadable: {e}"),
    }
}

/// Print the table list for one database file. For every table it prints the
/// row count and the column names/types from `PRAGMA table_info`. Metadata
/// only - never a row value, name, number or JID.
fn print_table_list(rel: &str, conn: &Connection) {
    match table_names(conn) {
        Ok(names) if !names.is_empty() => {
            println!("  {rel}: {} tables", names.len());
            for t in &names {
                let count = row_count(conn, t).unwrap_or(-1);
                let cols = columns(conn, t).unwrap_or_default();
                let col_str = cols
                    .iter()
                    .map(|(c, ty)| format!("{c}:{ty}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("    {t}: {count} rows | cols [{}]", col_str);
            }
        }
        Ok(_) => println!("  {rel}: (no tables)"),
        Err(e) => println!("  {rel}: unreadable: {e}"),
    }
}

/// `true` if `table` exists and has a column named `column`.
fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let safe = table.replace('\'', "''");
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info('{safe}')")) else {
        return false;
    };
    let Ok(rows) = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .map(|it| it.flatten().collect::<Vec<_>>())
    else {
        return false;
    };
    rows.iter().any(|name| name == column)
}

/// Column names and declared types for one table, in schema order.
fn columns(conn: &Connection, table: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let safe = table.replace('\'', "''");
    let mut stmt = conn.prepare(&format!("PRAGMA table_info('{safe}')"))?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    rows.collect()
}

fn is_db_extension(rel: &str) -> bool {
    rel.ends_with(".sqlite")
        || rel.ends_with(".db")
        || rel.ends_with(".sqlitedb")
        || rel.ends_with(".data")
}

/// Print sms.db table sizes, the message date range, and the share of rows
/// whose text is NULL but attributedBody is set. Counts and dates only.
fn print_sms_stats(conn: &Connection) {
    let tables = match table_names(conn) {
        Ok(names) => names,
        Err(e) => {
            println!("sms.db: unreadable: {e}");
            return;
        }
    };
    for t in &tables {
        match row_count(conn, t) {
            Ok(count) => println!("sms.db table {t}: {count} rows"),
            Err(e) => println!("sms.db table {t}: unreadable: {e}"),
        }
    }

    if tables.iter().any(|t| t == "message") {
        match message_stats(conn) {
            Ok((min, max, total, null_text_with_body)) => {
                if let Some(m) = min {
                    println!("message.date min: {}", to_utc(m));
                }
                if let Some(x) = max {
                    println!("message.date max: {}", to_utc(x));
                }
                let pct = if total > 0 {
                    null_text_with_body as f64 / total as f64 * 100.0
                } else {
                    0.0
                };
                println!(
                    "message rows text NULL but attributedBody set: {null_text_with_body} of {total} ({pct:.1}%)",
                );
                // Decode count only - never the text. Each NULL-text row with a
                // non-NULL attributedBody is streamed and decoded; a row read
                // error or an undecodable blob counts as failed.
                match attributed_body_decode_stats(conn) {
                    Ok((ok, fail)) => {
                        let n = ok + fail;
                        println!("attributedBody decode: {ok} of {n} decoded, {fail} failed");
                    }
                    Err(e) => println!("attributedBody decode: unreadable: {e}"),
                }
            }
            Err(e) => println!("sms.db message: unreadable: {e}"),
        }
    }
}

/// Count how many NULL-text rows with a non-NULL attributedBody decode to a
/// string, and how many fail. Streams row by row; never returns or prints the
/// decoded text. A row read error counts as a failure.
fn attributed_body_decode_stats(conn: &Connection) -> rusqlite::Result<(i64, i64)> {
    let mut stmt = conn.prepare(
        "SELECT attributedBody FROM message WHERE text IS NULL AND attributedBody IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
    let mut ok: i64 = 0;
    let mut fail: i64 = 0;
    for row in rows {
        match row {
            Ok(blob) => {
                if decode_attributed_body(&blob).is_some() {
                    ok += 1;
                } else {
                    fail += 1;
                }
            }
            Err(_) => fail += 1,
        }
    }
    Ok((ok, fail))
}

/// min/max date, total row count, and the count of rows with NULL text but a
/// non-NULL attributedBody.
fn message_stats(conn: &Connection) -> rusqlite::Result<(Option<i64>, Option<i64>, i64, i64)> {
    let (min, max): (Option<i64>, Option<i64>) =
        conn.query_row("SELECT MIN(date), MAX(date) FROM message", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM message", [], |r| r.get(0))?;
    let null_text_with_body: i64 = conn.query_row(
        "SELECT COUNT(*) FROM message WHERE text IS NULL AND attributedBody IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    Ok((min, max, total, null_text_with_body))
}

fn table_names(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}

fn row_count(conn: &Connection, table: &str) -> rusqlite::Result<i64> {
    let safe = table.replace('\'', "''");
    conn.query_row(&format!("SELECT COUNT(*) FROM '{safe}'"), [], |r| r.get(0))
}

/// Convert an iOS `message.date` to a UTC string. Values above 1e12 are
/// nanoseconds since 2001-01-01; anything smaller is seconds since the same
/// epoch.
fn to_utc(v: i64) -> String {
    let secs = if v > 1_000_000_000_000 {
        v / 1_000_000_000
    } else {
        v
    };
    let unix = IOS_EPOCH_SECS + secs;
    match Utc.timestamp_opt(unix, 0).single() {
        Some(dt) => dt.to_rfc3339(),
        None => format!("(unconvertible: {v})"),
    }
}
