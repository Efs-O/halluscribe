// HalluScribe - reader for Viber from an Apple backup.
//
// Phase 7b of the Business Messaging ingestion plan. It opens the backup
// (Phase 1), temp-copies and opens `Contacts.data` (`TempCopy::open`, which also
// replays a backed-up WAL), loads the rows (this crate's `viber_db`), splits
// them into windows (the shared `apple_messages_window`), and builds one
// `ParsedSession` per window. Names resolve the sender's phone number via the
// iPhone address book and `phone.rs` (D7: an unresolved number shows verbatim,
// never guessed). Nothing here logs names, numbers or text; skip counts are
// surfaced in one line.

use crate::apple_backup::contacts::ContactBook;
use crate::apple_backup::manifest::{open_backup, BackupHandle};
use crate::readers::apple_messages_window::{self, Window};
use crate::readers::viber_db::{self, ViberDb};
use crate::readers::viber_raw;
use crate::readers::{
    build_session, estimate_fill_pct, ChatProvider, MessageRole, ParsedMessage, ParsedSession,
    ReaderError,
};
use std::path::Path;

/// The `Contacts.data` logical path inside the Viber domain.
const CONTACTS_DATA: &str = "com.viber/database/Contacts.data";
/// The iPhone address book (the user's own contacts), shared with the SMS and
/// WhatsApp readers: the sender's phone is resolved through it via `phone.rs`.
const ADDRESS_BOOK: &str = "Library/AddressBook/AddressBook.sqlitedb";
const HOME_DOMAIN: &str = "HomeDomain";
/// The Viber app domain (one app; there is no separate Viber Business domain).
const VIBER_DOMAIN: &str = "AppDomainGroup-group.viber.share.container";

/// Whether a backup has a Viber `Contacts.data`. The scanner uses this to emit
/// a target only when Viber is actually present.
pub fn is_available(backup_dir: &Path) -> Result<bool, ReaderError> {
    let handle = open_backup(backup_dir).map_err(|e| ReaderError::Database(e.to_string()))?;
    Ok(handle.resolve(VIBER_DOMAIN, CONTACTS_DATA).is_some())
}

/// Read every Viber session out of a backup directory.
pub fn read(
    backup_dir: &Path,
    default_cc: Option<&str>,
) -> Result<Vec<ParsedSession>, ReaderError> {
    let handle = open_backup(backup_dir).map_err(|e| ReaderError::Database(e.to_string()))?;

    // `Contacts.data` is required; a missing one is a real error (the scanner
    // only emits a target when it exists).
    let chat_copy = handle
        .copy_to_temp(VIBER_DOMAIN, CONTACTS_DATA)
        .map_err(|e| ReaderError::Database(e.to_string()))?;
    let chat_conn = chat_copy
        .open()
        .map_err(|e| ReaderError::Database(e.to_string()))?;
    let db = viber_db::load(&chat_conn)?;

    // The address book is optional: absent ⇒ an empty book, and unresolved
    // numbers show verbatim rather than erroring.
    let book = load_contact_book(&handle, default_cc)?;

    let backup_label = backup_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let windows = apple_messages_window::windows(&db.messages, "viber");
    let mut sessions = Vec::new();
    for window in windows {
        let Some(conv) = db
            .conversations
            .iter()
            .find(|c| c.key == window.conversation)
            .cloned()
        else {
            continue;
        };
        let msgs = &db.messages[window.range.clone()];
        if msgs.is_empty() {
            continue;
        }
        if let Some(session) = build_window_session(
            &conv,
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
/// text (a window that is only media still renders, so this is rare).
fn build_window_session(
    conv: &crate::readers::apple_messages_db::Conversation,
    msgs: &[crate::readers::apple_messages_db::RawMessage],
    window: &Window,
    book: &ContactBook,
    default_cc: Option<&str>,
    backup_dir: &Path,
    backup_label: &str,
) -> Option<ParsedSession> {
    let title = viber_raw::conversation_title(conv, book, default_cc);
    let project_override = viber_raw::conversation_org(conv, book, default_cc);

    let mut messages = Vec::with_capacity(msgs.len());
    for m in msgs {
        let text = m.text.clone().unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        let (role, speaker) = if m.is_from_me {
            (MessageRole::Assistant, "Me".to_string())
        } else {
            (
                MessageRole::User,
                viber_raw::speaker_for(m, conv, book, default_cc),
            )
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
    let raw_slice = viber_raw::render_raw(conv, msgs, book, default_cc, backup_label);

    let mut session = build_session(
        window.session_id.clone(),
        title,
        created_at,
        updated_at,
        backup_dir.to_path_buf(),
        ChatProvider::Viber,
        fill_pct,
        true,
        messages,
    )?;
    session.raw_slice = Some(raw_slice);
    session.project_override = project_override;
    Some(session)
}

/// Load the iPhone address book from the backup, or an empty book if the file
/// is absent. A file that exists but cannot be opened is a real error. Shared
/// with the SMS and WhatsApp readers: an absent address book is not an error -
/// unresolved numbers show verbatim (D7).
fn load_contact_book(
    handle: &BackupHandle,
    default_cc: Option<&str>,
) -> Result<ContactBook, ReaderError> {
    // An absent address book (no manifest row, or no hashed file) is not an
    // error: unresolved numbers show verbatim (D7). A file that IS present but
    // cannot be copied (permissions, damaged hashed file) is a real error, not
    // a silent empty book.
    if handle.resolve(HOME_DOMAIN, ADDRESS_BOOK).is_none() {
        return Ok(ContactBook::default());
    }
    let copy = handle
        .copy_to_temp(HOME_DOMAIN, ADDRESS_BOOK)
        .map_err(|e| ReaderError::Database(e.to_string()))?;
    let conn = copy
        .open()
        .map_err(|e| ReaderError::Database(e.to_string()))?;
    ContactBook::from_connection(&conn, default_cc)
        .map_err(|e| ReaderError::Database(e.to_string()))
}

/// One line of skip counts only — never names, numbers or text.
fn surface_skip_counts(db: &ViberDb) {
    let s = &db.skipped;
    if s.system
        + s.media
        + s.unknown_state
        + s.unusable_text
        + s.orphan_no_session
        + s.bad_date
        + s.row_errors
        + s.no_sender
        > 0
    {
        eprintln!(
            "viber: skipped system={} media={} unknown_state={} unusable_text={} orphan_no_session={} bad_date={} row_errors={} no_sender={}",
            s.system, s.media, s.unknown_state, s.unusable_text, s.orphan_no_session, s.bad_date, s.row_errors, s.no_sender
        );
    }
}

#[cfg(test)]
#[path = "viber_tests.rs"]
mod tests;
