// HalluScribe - pure renderer for a Viber window's raw slice (I.6).
//
// Phase 7b. Turns a `Conversation`, its window's `RawMessage`s, the iPhone
// address book and the default country code into the deterministic,
// human-readable raw transcript. No I/O, no allocation beyond the returned
// `String`. Name resolution (the Phase 7b contact rule): the address book via
// `phone.rs` for the sender's phone number, else the phone verbatim (D7: never
// a guess). Viber has no push-name table of its own, so the address book is
// the only name source. Timestamps are rendered in `chrono::Local` with the
// offset. No names, numbers or text are logged.

use crate::apple_backup::contacts::ContactBook;
use crate::apple_backup::phone::national_form;
use crate::readers::apple_messages_db::{Conversation, RawMessage};
use chrono::{DateTime, Local, Utc};

/// Resolve a phone number to a display name: the address book (via `phone.rs`),
/// else the phone verbatim (D7).
fn resolve_name(phone: &str, book: &ContactBook, default_cc: Option<&str>) -> String {
    let phone = phone.trim();
    if let Some(contact) = book.resolve(phone, default_cc) {
        return contact.name.clone();
    }
    phone.to_string()
}

/// The conversation's title: for a 1:1 chat, the resolved contact name (the
/// address book, else the phone); for a group, the conversation name if set,
/// else the participant phones joined with ", " (at most 4, then "+N").
pub fn conversation_title(
    conv: &Conversation,
    book: &ContactBook,
    default_cc: Option<&str>,
) -> String {
    if conv.participants.len() <= 1 {
        conv.participants
            .first()
            .map(|p| resolve_name(p, book, default_cc))
            .unwrap_or_else(|| "Unknown".to_string())
    } else {
        match conv.display_name.as_deref() {
            Some(name) if !name.trim().is_empty() => name.trim().to_string(),
            _ => {
                let labels: Vec<String> = conv
                    .participants
                    .iter()
                    .map(|p| resolve_name(p, book, default_cc))
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
    let phone = conv.participants.first()?;
    book.resolve(phone, default_cc)
        .and_then(|c| c.organization.clone())
}

/// Render one window's raw slice in the I.6 format.
pub fn render_raw(
    conv: &Conversation,
    msgs: &[RawMessage],
    book: &ContactBook,
    default_cc: Option<&str>,
    backup_label: &str,
) -> String {
    let title = conversation_title(conv, book, default_cc);
    let org = conversation_org(conv, book, default_cc);

    let mut out = String::new();
    out.push_str("# Viber — ");
    out.push_str(&title);
    if let Some(org) = &org {
        out.push_str(" — ");
        out.push_str(org);
    }
    out.push('\n');

    out.push_str(&format!(
        "# chat: {} · participants: {}\n",
        conv.display_name.as_deref().unwrap_or("unknown"),
        participants_line(conv, book, default_cc)
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
            speaker_for(msg, conv, book, default_cc)
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
/// `<name> (<E164>)` for a resolved phone, or the raw phone when unresolved.
fn participants_line(conv: &Conversation, book: &ContactBook, default_cc: Option<&str>) -> String {
    let mut parts: Vec<String> = vec!["Me".to_string()];
    for phone in &conv.participants {
        parts.push(participant_label(phone, book, default_cc));
    }
    parts.join("; ")
}

/// One participant's header label: `<name> (<E164>)` for a resolved phone, or
/// the raw phone when it does not resolve.
fn participant_label(phone: &str, book: &ContactBook, default_cc: Option<&str>) -> String {
    if let Some((_, e164)) = book.resolve_phone(phone, default_cc) {
        format!("{} ({})", resolve_name(phone, book, default_cc), e164)
    } else {
        resolve_name(phone, book, default_cc)
    }
}

/// The message's speaker label. A `Me` message is just `Me`; a resolved phone
/// contact is `<Name> | <E164>` plus the national form when it has one; an
/// unresolved phone is the raw phone; a missing sender (a received row with no
/// index) falls back to the conversation name (1:1) or `Unknown` (group). Used
/// both for the raw slice and for the `ParsedMessage.speaker` field, so the
/// two never diverge.
pub fn speaker_for(
    msg: &RawMessage,
    conv: &Conversation,
    book: &ContactBook,
    default_cc: Option<&str>,
) -> String {
    if msg.is_from_me {
        return "Me".to_string();
    }
    let phone = msg.sender_handle.as_deref().or_else(|| {
        if conv.participants.len() == 1 {
            conv.participants.first().map(|s| s.as_str())
        } else {
            None
        }
    });
    let Some(phone) = phone else {
        // A received row with no sender index: the conversation name for a 1:1
        // chat (at most one participant), else `Unknown` for a group.
        if conv.participants.len() > 1 {
            return "Unknown".to_string();
        }
        return conv
            .display_name
            .as_deref()
            .filter(|n| !n.trim().is_empty())
            .map(|n| n.trim().to_string())
            .unwrap_or_else(|| "Unknown".to_string());
    };
    let phone = phone.trim();
    if let Some((_, e164)) = book.resolve_phone(phone, default_cc) {
        let mut line = format!("{} | {}", resolve_name(phone, book, default_cc), e164);
        if let Some(national) = national_form(&e164, default_cc) {
            line.push_str(&format!(" | {national}"));
        }
        line
    } else {
        resolve_name(phone, book, default_cc)
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
#[path = "viber_raw_tests.rs"]
mod tests;
