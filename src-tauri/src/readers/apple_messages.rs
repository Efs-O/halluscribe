// HalluScribe - reader for iPhone "Messages" from an Apple backup (Phase 5b).
//
// Opens the backup (Phase 1), temp-copies and opens `sms.db` read-only (Phase
// 5a's `load` + `windows`), and builds one `ParsedSession` per window. The
// address book is resolved for names and organizations; an absent
// `AddressBook.sqlitedb` is not an error — unresolved handles show verbatim
// (D7). The raw slice is rendered by `apple_messages_raw`. Nothing here logs
// names, handles or text; skip counts are surfaced in one line.

use crate::apple_backup::contacts::ContactBook;
use crate::apple_backup::manifest::{open_backup, open_sqlite_read_only};
use crate::readers::apple_messages_db::{self, RawMessage};
use crate::readers::apple_messages_raw::{
    attachment_line, conversation_org, conversation_title, render_raw,
};
use crate::readers::apple_messages_window::{self, Window};
use crate::readers::{
    build_session, estimate_fill_pct, ChatProvider, MessageRole, ParsedMessage, ParsedSession,
    ReaderError,
};
use std::path::Path;

/// The logical paths of the two databases inside an Apple backup.
const SMS_DB: &str = "Library/SMS/sms.db";
const ADDRESS_BOOK: &str = "Library/AddressBook/AddressBook.sqlitedb";
const HOME_DOMAIN: &str = "HomeDomain";

/// Read every Messages session out of an Apple backup directory.
pub fn read(
    backup_dir: &Path,
    default_cc: Option<&str>,
) -> Result<Vec<ParsedSession>, ReaderError> {
    let handle = open_backup(backup_dir).map_err(|e| ReaderError::Database(e.to_string()))?;

    // `sms.db` is required; a missing one is a real error.
    let sms_copy = handle
        .copy_to_temp(HOME_DOMAIN, SMS_DB)
        .map_err(|e| ReaderError::Database(e.to_string()))?;
    let sms_conn =
        open_sqlite_read_only(sms_copy.path()).map_err(|e| ReaderError::Database(e.to_string()))?;
    let db = apple_messages_db::load(&sms_conn)?;

    // The address book is optional: absent ⇒ an empty book, and unresolved
    // handles show verbatim rather than erroring.
    let book = load_contact_book(&handle, default_cc)?;

    let backup_label = backup_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let windows = apple_messages_window::windows(&db.messages);
    let mut sessions = Vec::new();
    for window in windows {
        let conv = match db
            .conversations
            .iter()
            .find(|c| c.key == window.conversation)
        {
            Some(c) => c,
            None => continue,
        };
        let msgs = &db.messages[window.range.clone()];
        if msgs.is_empty() {
            continue;
        }
        if let Some(session) = build_window_session(
            conv,
            msgs,
            &window,
            &book,
            default_cc,
            backup_dir,
            &backup_label,
        ) {
            sessions.push(session);
        }
    }

    surface_skip_counts(&db);
    Ok(sessions)
}

/// Build the `ParsedSession` for one window, or `None` if it has no usable
/// text (a window that is only attachments still renders, so this is rare).
fn build_window_session(
    conv: &apple_messages_db::Conversation,
    msgs: &[RawMessage],
    window: &Window,
    book: &ContactBook,
    default_cc: Option<&str>,
    backup_dir: &Path,
    backup_label: &str,
) -> Option<ParsedSession> {
    let title = conversation_title(conv, book, default_cc);
    let project_override = conversation_org(conv, book, default_cc);

    let mut messages = Vec::with_capacity(msgs.len());
    for m in msgs {
        let mut text = m.text.clone().unwrap_or_default();
        for attachment in &m.attachments {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&attachment_line(attachment));
        }
        if text.trim().is_empty() {
            continue;
        }
        let (role, speaker) = if m.is_from_me {
            (MessageRole::Assistant, "Me".to_string())
        } else {
            (MessageRole::User, speaker_label(m, book, default_cc))
        };
        messages.push(ParsedMessage {
            role,
            text,
            timestamp: Some(m.date_utc),
            speaker: Some(speaker),
        });
    }

    let transcript = messages
        .iter()
        .map(|m| m.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if transcript.trim().is_empty() {
        return None;
    }

    let created_at = msgs.first()?.date_utc;
    let updated_at = msgs.last().map(|m| m.date_utc);
    let fill_pct = estimate_fill_pct(&transcript);
    let raw_slice = render_raw(conv, msgs, book, default_cc, backup_label);

    let mut session = build_session(
        window.session_id.clone(),
        title,
        created_at,
        updated_at,
        backup_dir.to_path_buf(),
        ChatProvider::AppleMessages,
        fill_pct,
        true,
        messages,
    )?;
    session.raw_slice = Some(raw_slice);
    session.project_override = project_override;
    Some(session)
}

/// The speaker label for a non-`Me` message: the resolved contact's name, the
/// raw handle verbatim when unresolved, or `"Unknown"` when there is no handle.
fn speaker_label(m: &RawMessage, book: &ContactBook, default_cc: Option<&str>) -> String {
    let Some(handle) = m.sender_handle.as_deref() else {
        return "Unknown".to_string();
    };
    book.resolve(handle, default_cc)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| handle.trim().to_string())
}

/// Load the address book from the backup, or an empty book if the file is
/// absent. A file that exists but cannot be opened is a real error.
fn load_contact_book(
    handle: &crate::apple_backup::manifest::BackupHandle,
    default_cc: Option<&str>,
) -> Result<ContactBook, ReaderError> {
    let Some(copy) = handle.copy_to_temp(HOME_DOMAIN, ADDRESS_BOOK).ok() else {
        return Ok(ContactBook::default());
    };
    let conn =
        open_sqlite_read_only(copy.path()).map_err(|e| ReaderError::Database(e.to_string()))?;
    ContactBook::from_connection(&conn, default_cc)
        .map_err(|e| ReaderError::Database(e.to_string()))
}

/// One line of skip counts only — never names, handles or text.
fn surface_skip_counts(db: &apple_messages_db::MessagesDb) {
    let s = &db.skipped;
    if s.tapbacks
        + s.group_events
        + s.unusable_text
        + s.orphan_no_handle
        + s.bad_date
        + s.row_errors
        > 0
    {
        eprintln!(
            "apple_messages: skipped tapbacks={} group_events={} unusable_text={} orphan_no_handle={} bad_date={} row_errors={}",
            s.tapbacks, s.group_events, s.unusable_text, s.orphan_no_handle, s.bad_date, s.row_errors
        );
    }
}

#[cfg(test)]
#[path = "apple_messages_tests.rs"]
mod tests;
