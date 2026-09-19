// HalluScribe - reader for WhatsApp + WhatsApp Business from an Apple backup.
//
// Phase 7a of the Business Messaging ingestion plan. Both apps share the same
// `ChatStorage.sqlite` schema, so ONE reader serves both, parameterised by the
// app's `ChatProvider` (which selects the backup domain). It opens the backup
// (Phase 1), temp-copies and opens `ChatStorage.sqlite` read-only (Phase 5a's
// `open_sqlite_read_only`), loads the rows (this crate's `whatsapp_db`), splits
// them into windows (the shared `apple_messages_window`), and builds one
// `ParsedSession` per window. Names resolve WhatsApp's own push/partner name
// first, then the iPhone address book via `phone.rs` for the number in the JID
// (D7: an unresolved JID shows verbatim, never guessed). Nothing here logs
// names, JIDs or text; skip counts are surfaced in one line.

use crate::apple_backup::contacts::ContactBook;
use crate::apple_backup::manifest::{open_backup, open_sqlite_read_only, BackupHandle};
use crate::readers::apple_messages_window::{self, Window};
use crate::readers::whatsapp_db::{self, WhatsAppDb};
use crate::readers::whatsapp_raw;
use crate::readers::{
    build_session, estimate_fill_pct, ChatProvider, MessageRole, ParsedMessage, ParsedSession,
    ReaderError,
};
use std::collections::HashMap;
use std::path::Path;

/// The `ChatStorage.sqlite` logical path inside a WhatsApp domain.
const CHAT_STORAGE: &str = "ChatStorage.sqlite";
/// The iPhone address book (the user's own contacts), shared with the SMS
/// reader: WhatsApp's push names come first, then this book via `phone.rs`.
const ADDRESS_BOOK: &str = "Library/AddressBook/AddressBook.sqlitedb";
const HOME_DOMAIN: &str = "HomeDomain";
/// The two WhatsApp app domains, one per `ChatProvider` variant.
const WHATSAPP_DOMAIN: &str = "AppDomainGroup-group.net.whatsapp.WhatsApp.shared";
const WHATSAPP_BUSINESS_DOMAIN: &str = "AppDomainGroup-group.net.whatsapp.WhatsAppSMB.shared";

/// The backup domain an app's `ChatStorage.sqlite` lives in, or `None` for a
/// non-WhatsApp provider.
fn domain_for(app: &ChatProvider) -> Option<&'static str> {
    match app {
        ChatProvider::WhatsApp => Some(WHATSAPP_DOMAIN),
        ChatProvider::WhatsAppBusiness => Some(WHATSAPP_BUSINESS_DOMAIN),
        _ => None,
    }
}

/// The WhatsApp apps present in a backup: those whose domain has a
/// `ChatStorage.sqlite`. The scanner uses this to emit one target per app that
/// actually exists, so a phone without WhatsApp Business never yields a
/// WhatsAppBusiness target (and vice versa).
pub fn available_apps(backup_dir: &Path) -> Result<Vec<ChatProvider>, ReaderError> {
    let handle = open_backup(backup_dir).map_err(|e| ReaderError::Database(e.to_string()))?;
    let mut apps = Vec::new();
    for app in [ChatProvider::WhatsApp, ChatProvider::WhatsAppBusiness] {
        if let Some(domain) = domain_for(&app) {
            if handle.resolve(domain, CHAT_STORAGE).is_some() {
                apps.push(app);
            }
        }
    }
    Ok(apps)
}

/// Read every WhatsApp session out of one app's domain in a backup directory.
pub fn read(
    backup_dir: &Path,
    app: ChatProvider,
    default_cc: Option<&str>,
) -> Result<Vec<ParsedSession>, ReaderError> {
    let Some(domain) = domain_for(&app) else {
        return Err(ReaderError::Database("not a WhatsApp provider".to_string()));
    };

    let handle = open_backup(backup_dir).map_err(|e| ReaderError::Database(e.to_string()))?;

    // `ChatStorage.sqlite` is required for this domain; a missing one is a real
    // error (the scanner only emits a target when it exists).
    let chat_copy = handle
        .copy_to_temp(domain, CHAT_STORAGE)
        .map_err(|e| ReaderError::Database(e.to_string()))?;
    let chat_conn = open_sqlite_read_only(chat_copy.path())
        .map_err(|e| ReaderError::Database(e.to_string()))?;
    let db = whatsapp_db::load(&chat_conn)?;

    // The address book is optional: absent ⇒ an empty book, and unresolved
    // JIDs show verbatim rather than erroring.
    let book = load_contact_book(&handle, default_cc)?;

    let backup_label = backup_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let windows = apple_messages_window::windows(&db.messages, app.provider_key());
    let ctx = WindowCtx {
        push_names: &db.push_names,
        book: &book,
        default_cc,
        backup_dir,
        backup_label: &backup_label,
        app: &app,
    };
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
        if let Some(session) = build_window_session(&conv, msgs, &window, &ctx) {
            sessions.push(session);
        }
    }

    surface_skip_counts(&db);
    Ok(sessions)
}

/// The per-read context shared by every window: the push-name map, the address
/// book, the default country code, the backup dir + label, and the app (which
/// selects the provider + domain). Bundled so `build_window_session` stays
/// under the 7-argument limit.
struct WindowCtx<'a> {
    push_names: &'a HashMap<String, String>,
    book: &'a ContactBook,
    default_cc: Option<&'a str>,
    backup_dir: &'a Path,
    backup_label: &'a str,
    app: &'a ChatProvider,
}

/// Build the `ParsedSession` for one window, or `None` if it has no usable
/// text (a window that is only media still renders, so this is rare).
fn build_window_session(
    conv: &crate::readers::apple_messages_db::Conversation,
    msgs: &[crate::readers::apple_messages_db::RawMessage],
    window: &Window,
    ctx: &WindowCtx,
) -> Option<ParsedSession> {
    let title = whatsapp_raw::conversation_title(conv, ctx.push_names, ctx.book, ctx.default_cc);
    let project_override = whatsapp_raw::conversation_org(conv, ctx.book, ctx.default_cc);

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
                whatsapp_raw::speaker_for(m, conv, ctx.push_names, ctx.book, ctx.default_cc),
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
    let raw_slice = whatsapp_raw::render_raw(
        conv,
        msgs,
        ctx.push_names,
        ctx.book,
        ctx.default_cc,
        ctx.backup_label,
    );

    let mut session = build_session(
        window.session_id.clone(),
        title,
        created_at,
        updated_at,
        ctx.backup_dir.to_path_buf(),
        ctx.app.clone(),
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
/// with the SMS reader: an absent address book is not an error - unresolved
/// JIDs show verbatim (D7).
fn load_contact_book(
    handle: &BackupHandle,
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

/// One line of skip counts only — never names, JIDs or text.
fn surface_skip_counts(db: &WhatsAppDb) {
    let s = &db.skipped;
    if s.media + s.group_events + s.unusable_text + s.orphan_no_session + s.bad_date + s.row_errors
        > 0
    {
        eprintln!(
            "whatsapp: skipped media={} group_events={} unusable_text={} orphan_no_session={} bad_date={} row_errors={}",
            s.media, s.group_events, s.unusable_text, s.orphan_no_session, s.bad_date, s.row_errors
        );
    }
}

#[cfg(test)]
#[path = "whatsapp_tests.rs"]
mod tests;
