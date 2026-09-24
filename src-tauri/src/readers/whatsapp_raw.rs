// HalluScribe - pure renderer for a WhatsApp window's raw slice (I.6).
//
// Phase 7a. Turns a `Conversation`, its window's `RawMessage`s, the push-name
// map, the iPhone address book and the default country code into the
// deterministic, human-readable raw transcript. No I/O, no allocation beyond
// the returned `String`. Name resolution order (the Phase 7a contact rule):
// WhatsApp's own push/partner name first, then the address book via `phone.rs`
// for the number in the JID, else the JID verbatim (D7: never a guess).
// Timestamps are rendered in `chrono::Local` with the offset. No names,
// numbers or text are logged.

use crate::apple_backup::contacts::ContactBook;
use crate::apple_backup::phone::national_form;
use crate::readers::apple_messages_db::{Conversation, RawMessage};
use chrono::{DateTime, Local, Utc};
use std::collections::HashMap;

/// The phone handle in a WhatsApp JID. A 1:1 JID is
/// `<cc><number>@s.whatsapp.net`: always international but with no `+`, so the
/// `+` is added here - otherwise normalization would read it as a national
/// number and prepend the default country code a second time. Any other JID
/// (a group `<id>@g.us`, a `@lid`) is not a phone and yields `None`. With no
/// `@`, the whole string is the handle.
fn jid_number(jid: &str) -> Option<String> {
    match jid.split_once('@') {
        None => Some(jid.to_string()),
        Some((user, "s.whatsapp.net")) => {
            let digits = user.strip_prefix('+').unwrap_or(user);
            let is_phone = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
            is_phone.then(|| format!("+{digits}"))
        }
        Some(_) => None,
    }
}

/// Resolve a JID to a display name: the push name first, then the address book
/// (via the number in the JID), else the JID verbatim (D7).
fn resolve_name(
    jid: &str,
    push_names: &HashMap<String, String>,
    book: &ContactBook,
    default_cc: Option<&str>,
) -> String {
    let jid = jid.trim();
    if let Some(name) = push_names.get(jid) {
        return name.clone();
    }
    if let Some(contact) = jid_number(jid).and_then(|n| book.resolve(&n, default_cc)) {
        return contact.name.clone();
    }
    jid.to_string()
}

/// The conversation's title: for a 1:1 chat, the resolved contact name (push
/// name, then the address book, else the JID); for a group, the partner name
/// if set, else the member labels joined with ", " (at most 4, then "+N").
pub fn conversation_title(
    conv: &Conversation,
    push_names: &HashMap<String, String>,
    book: &ContactBook,
    default_cc: Option<&str>,
) -> String {
    if conv.participants.len() <= 1 {
        conv.participants
            .first()
            .map(|j| resolve_name(j, push_names, book, default_cc))
            .unwrap_or_else(|| "Unknown".to_string())
    } else {
        match conv.display_name.as_deref() {
            Some(name) if !name.trim().is_empty() => name.trim().to_string(),
            _ => {
                let labels: Vec<String> = conv
                    .participants
                    .iter()
                    .map(|j| resolve_name(j, push_names, book, default_cc))
                    .collect();
                join_labels(&labels)
            }
        }
    }
}

/// The project to archive a 1:1 session under: the resolved contact's
/// organization (the client's business name), or `None` for a group (which has
/// no single owner) or when the contact has none. Mirrors the SMS reader, so
/// the title and the project label never duplicate the same name.
pub fn conversation_org(
    conv: &Conversation,
    book: &ContactBook,
    default_cc: Option<&str>,
) -> Option<String> {
    if conv.participants.len() > 1 {
        return None;
    }
    let jid = conv.participants.first()?;
    jid_number(jid)
        .and_then(|n| book.resolve(&n, default_cc))
        .and_then(|c| c.organization.clone())
}

/// Render one window's raw slice in the I.6 format.
pub fn render_raw(
    conv: &Conversation,
    msgs: &[RawMessage],
    push_names: &HashMap<String, String>,
    book: &ContactBook,
    default_cc: Option<&str>,
    backup_label: &str,
) -> String {
    let title = conversation_title(conv, push_names, book, default_cc);
    let org = conversation_org(conv, book, default_cc);

    let mut out = String::new();
    out.push_str("# WhatsApp — ");
    out.push_str(&title);
    if let Some(org) = &org {
        out.push_str(" — ");
        out.push_str(org);
    }
    out.push('\n');

    out.push_str(&format!(
        "# chat: {} · participants: {}\n",
        conv.chat_guid.as_deref().unwrap_or("unknown"),
        participants_line(conv, push_names, book, default_cc)
    ));

    let (first, last) = (msgs.first(), msgs.last());
    out.push_str(&format!(
        "# window: {} → {} · {} messages · backup: {}\n",
        local_date(first.map(|m| m.date_utc)),
        local_date(last.map(|m| m.date_utc)),
        msgs.len(),
        backup_label
    ));
    out.push('\n');

    for (i, msg) in msgs.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&format!(
            "[{}] {}",
            local_ts(msg.date_utc),
            speaker_for(msg, conv, push_names, book, default_cc)
        ));
        if let Some(text) = &msg.text {
            if !text.trim().is_empty() {
                out.push('\n');
                out.push_str(text);
            }
        }
    }
    out
}

/// The `participants:` value for the header: `Me;` then each participant as
/// `<name> (<E164>)` for a resolved phone, or the raw JID when unresolved.
fn participants_line(
    conv: &Conversation,
    push_names: &HashMap<String, String>,
    book: &ContactBook,
    default_cc: Option<&str>,
) -> String {
    let mut parts: Vec<String> = vec!["Me".to_string()];
    for jid in &conv.participants {
        parts.push(participant_label(jid, push_names, book, default_cc));
    }
    parts.join("; ")
}

/// One participant's header label: `<name> (<E164>)` for a resolved phone, or
/// the raw JID when it does not resolve.
fn participant_label(
    jid: &str,
    push_names: &HashMap<String, String>,
    book: &ContactBook,
    default_cc: Option<&str>,
) -> String {
    if let Some((_, e164)) = jid_number(jid).and_then(|n| book.resolve_phone(&n, default_cc)) {
        format!(
            "{} ({})",
            resolve_name(jid, push_names, book, default_cc),
            e164
        )
    } else {
        resolve_name(jid, push_names, book, default_cc)
    }
}

/// The message's speaker label. A `Me` message is just `Me`; a resolved phone
/// contact is `<Name> | <E164>` plus the national form when it has one; an
/// unresolved JID is the raw JID; a missing sender JID is `Unknown`. Used both
/// for the raw slice and for the `ParsedMessage.speaker` field, so the two
/// never diverge.
pub fn speaker_for(
    msg: &RawMessage,
    conv: &Conversation,
    push_names: &HashMap<String, String>,
    book: &ContactBook,
    default_cc: Option<&str>,
) -> String {
    if msg.is_from_me {
        return "Me".to_string();
    }
    // A 1:1 chat's sender is the session's contact JID even when the message
    // carries no `ZFROMJID`.
    let jid = msg.sender_handle.as_deref().or_else(|| {
        if conv.participants.len() == 1 {
            conv.participants.first().map(|s| s.as_str())
        } else {
            None
        }
    });
    let Some(jid) = jid else {
        return "Unknown".to_string();
    };
    let jid = jid.trim();
    if let Some((_, e164)) = jid_number(jid).and_then(|n| book.resolve_phone(&n, default_cc)) {
        let mut line = format!(
            "{} | {}",
            resolve_name(jid, push_names, book, default_cc),
            e164
        );
        if let Some(national) = national_form(&e164, default_cc) {
            line.push_str(&format!(" | {national}"));
        }
        line
    } else {
        resolve_name(jid, push_names, book, default_cc)
    }
}

/// Join participant labels with ", ", showing at most 4 then "+N".
fn join_labels(labels: &[String]) -> String {
    if labels.len() <= 4 {
        labels.join(", ")
    } else {
        let shown = labels
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        format!("{shown}, +{}", labels.len() - 4)
    }
}

/// A `DateTime<Utc>` as a local `YYYY-MM-DD` (for the window header).
fn local_date(dt: Option<DateTime<Utc>>) -> String {
    dt.map(|d| d.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// A `DateTime<Utc>` as a local `YYYY-MM-DD HH:MM:SS +HH:MM` (the offset keeps
/// the instant exact even when the PC's time zone changes).
fn local_ts(dt: DateTime<Utc>) -> String {
    dt.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S %z")
        .to_string()
}

#[cfg(test)]
#[path = "whatsapp_raw_tests.rs"]
mod tests;
