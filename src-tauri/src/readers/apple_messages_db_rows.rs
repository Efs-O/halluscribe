// HalluScribe - per-row helpers for the Messages DB reader: text resolution,
// attachment loading and iOS date conversion. Split out of
// `apple_messages_db.rs` to keep that file within the size limit.

use super::{RawAttachment, SkipCounts};
use crate::apple_backup::decode_attributed_body;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Statement;

/// iOS core-data epoch: 2001-01-01 00:00:00 UTC, in unix seconds.
const IOS_EPOCH_SECS: i64 = 978_307_200;
/// Above this, `message.date` is nanoseconds since 2001-01-01; below it, seconds.
const NS_THRESHOLD: i64 = 1_000_000_000_000;

/// The attachments on one message, via `message_attachment_join`. Prepared
/// once per load and bound per message.
pub(super) const ATTACHMENTS_SQL: &str = "SELECT a.transfer_name, a.mime_type, a.total_bytes
     FROM message_attachment_join j
     JOIN attachment a ON a.ROWID = j.attachment_id
     WHERE j.message_id = ?1";

/// Resolve the message text: the `text` column if non-blank, else the decoded
/// `attributedBody` if non-blank, else `None`.
pub(super) fn resolve_text(
    text: &Option<String>,
    attributed_body: &Option<Vec<u8>>,
) -> Option<String> {
    if let Some(t) = text {
        let t = t.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    if let Some(blob) = attributed_body {
        if let Some(decoded) = decode_attributed_body(blob) {
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }
    None
}

/// The attachments on a message, read with the prepared `ATTACHMENTS_SQL`
/// statement. An unreadable attachment row (or a failed query) is counted in
/// `skipped.attachment_errors`; it never aborts the whole load.
pub(super) fn load_attachments(
    stmt: &mut Statement<'_>,
    message_rowid: i64,
    skipped: &mut SkipCounts,
) -> Vec<RawAttachment> {
    let rows = stmt.query_map([message_rowid], |r| {
        Ok(RawAttachment {
            transfer_name: r.get::<_, Option<String>>(0)?,
            mime_type: r.get::<_, Option<String>>(1)?,
            total_bytes: r.get::<_, Option<i64>>(2)?,
        })
    });
    let rows = match rows {
        Ok(rows) => rows,
        Err(_) => {
            skipped.attachment_errors += 1;
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for row in rows {
        match row {
            Ok(attachment) => out.push(attachment),
            Err(_) => skipped.attachment_errors += 1,
        }
    }
    out
}

/// Convert an iOS `message.date` (nanoseconds or seconds since 2001-01-01 UTC)
/// to a `DateTime<Utc>`. Returns `None` for an unconvertible value.
pub(super) fn ios_date_to_utc(value: i64) -> Option<DateTime<Utc>> {
    if value < 0 {
        return None;
    }
    let (secs, nanos) = if value > NS_THRESHOLD {
        (value / 1_000_000_000, (value % 1_000_000_000) as u32)
    } else {
        (value, 0)
    };
    Utc.timestamp_opt(IOS_EPOCH_SECS + secs, nanos).single()
}
