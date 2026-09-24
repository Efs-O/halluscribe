// HalluScribe - pure renderer for an Apple Messages window's raw slice (I.6).
//
// Phase 5b. Turns a `Conversation`, its window's `RawMessage`s, the address
// book and the default country code into the deterministic, human-readable raw
// transcript. No I/O, no allocation beyond the returned `String`. Timestamps
// are rendered in `chrono::Local` with the offset (the user thinks in local
// time; the offset keeps it exact). Unresolved handles are shown verbatim,
// never guessed (D7). No names, numbers or text are logged.

use crate::apple_backup::contacts::ContactBook;
use crate::apple_backup::phone::national_form;
use crate::readers::apple_messages_db::{Conversation, RawAttachment, RawMessage};
use chrono::{DateTime, Local, Utc};

/// The conversation's title: for a 1:1 chat or an orphan, the resolved
/// contact's name (else the raw handle); for a group, the `display_name` if
/// set, else the participant labels joined with ", " (at most 4, then "+N").
/// Shared by the session title and the raw header so the two never diverge.
pub fn conversation_title(
    conv: &Conversation,
    book: &ContactBook,
    default_cc: Option<&str>,
) -> String {
    if conv.participants.len() <= 1 {
        // A 1:1 chat or an orphan: the resolved contact's name, else the raw
        // handle verbatim (D7).
        conv.participants
            .first()
            .map(|h| label_for(h, book, default_cc))
            .unwrap_or_else(|| "Unknown".to_string())
    } else {
        // A group: the `display_name` if set, else the participant labels
        // joined with ", " (at most 4, then "+N").
        match conv.display_name.as_deref() {
            Some(name) if !name.trim().is_empty() => name.trim().to_string(),
            _ => {
                let labels: Vec<String> = conv
                    .participants
                    .iter()
                    .map(|h| label_for(h, book, default_cc))
                    .collect();
                join_labels(&labels)
            }
        }
    }
}

/// The organization to archive a 1:1/orphan session under: the resolved
/// contact's `organization`, or `None` for a group (which has no single owner)
/// or when the contact has none.
pub fn conversation_org(
    conv: &Conversation,
    book: &ContactBook,
    default_cc: Option<&str>,
) -> Option<String> {
    if conv.participants.len() > 1 {
        return None;
    }
    conv.participants
        .first()
        .and_then(|h| book.resolve(h, default_cc))
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
    out.push_str("# Messages — ");
    out.push_str(&title);
    if let Some(org) = &org {
        out.push_str(" — ");
        out.push_str(org);
    }
    out.push('\n');

    out.push_str(&format!(
        "# chat: {} · service: {} · participants: {}\n",
        conv.chat_guid.as_deref().unwrap_or("handle"),
        conv.service.as_deref().unwrap_or("unknown"),
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
    out.push('\n'); // one blank line between the header and the first message

    for (i, msg) in msgs.iter().enumerate() {
        if i > 0 {
            // One blank line between messages: the previous block ends without
            // a trailing newline, so two newlines separate it from this one.
            out.push_str("\n\n");
        }
        out.push_str(&format!(
            "[{}] {}",
            local_ts(msg.date_utc),
            speaker_line(msg, book, default_cc)
        ));
        if let Some(text) = &msg.text {
            if !text.trim().is_empty() {
                out.push('\n');
                out.push_str(text);
            }
        }
        for attachment in &msg.attachments {
            out.push('\n');
            out.push_str(&attachment_line(attachment));
        }
    }
    out
}

/// The `participants:` value for the header: `Me;` then each participant as
/// `<label> (<E164>)` for a resolved phone, `<label> (<email>)` for an email
/// contact, or the raw handle when unresolved.
fn participants_line(conv: &Conversation, book: &ContactBook, default_cc: Option<&str>) -> String {
    let mut parts: Vec<String> = vec!["Me".to_string()];
    for handle in &conv.participants {
        parts.push(participant_label(handle, book, default_cc));
    }
    parts.join("; ")
}

/// One participant's header label: `<name> (<E164>)`, `<name> (<email>)`, or
/// the raw handle when it does not resolve.
fn participant_label(handle: &str, book: &ContactBook, default_cc: Option<&str>) -> String {
    let handle = handle.trim();
    if handle.contains('@') {
        return match book.resolve(handle, default_cc) {
            Some(contact) => format!("{} ({})", contact.name, handle),
            None => handle.to_string(),
        };
    }
    match book.resolve_phone(handle, default_cc) {
        Some((contact, e164)) => format!("{} ({})", contact.name, e164),
        None => handle.to_string(),
    }
}

/// A resolved contact's name, else the raw handle verbatim (D7: never a guess).
fn label_for(handle: &str, book: &ContactBook, default_cc: Option<&str>) -> String {
    book.resolve(handle, default_cc)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| handle.trim().to_string())
}

/// The message's speaker line. A `Me` message is just `Me`; a resolved phone
/// contact is `<Name> | <E164>` plus the national form when it has one; an
/// email contact is `<Name> | <email>`; an unresolved handle is the raw
/// handle; a missing sender handle is `Unknown`.
fn speaker_line(msg: &RawMessage, book: &ContactBook, default_cc: Option<&str>) -> String {
    if msg.is_from_me {
        return "Me".to_string();
    }
    let Some(handle) = &msg.sender_handle else {
        return "Unknown".to_string();
    };
    let handle = handle.trim();
    if handle.contains('@') {
        return match book.resolve(handle, default_cc) {
            Some(contact) => format!("{} | {}", contact.name, handle),
            None => handle.to_string(),
        };
    }
    match book.resolve_phone(handle, default_cc) {
        Some((contact, e164)) => {
            let mut line = format!("{} | {}", contact.name, e164);
            if let Some(national) = national_form(&e164, default_cc) {
                line.push_str(&format!(" | {national}"));
            }
            line
        }
        None => handle.to_string(),
    }
}

/// One attachment line: `[attachment: <name> · <mime> · <size>]`, with
/// `unnamed`/`unknown` placeholders and a human size.
pub fn attachment_line(a: &RawAttachment) -> String {
    format!(
        "[attachment: {} · {} · {}]",
        a.transfer_name.as_deref().unwrap_or("unnamed"),
        a.mime_type.as_deref().unwrap_or("unknown"),
        human_size(a.total_bytes)
    )
}

/// A byte count as a human size: `340 KB`, `2.1 MB`, or `unknown` when absent.
pub fn human_size(bytes: Option<i64>) -> String {
    let Some(b) = bytes else {
        return "unknown".to_string();
    };
    let b = b as f64;
    if b >= 1_000_000_000.0 {
        format!("{:.1} GB", b / 1_000_000_000.0)
    } else if b >= 1_000_000.0 {
        format!("{:.1} MB", b / 1_000_000.0)
    } else if b >= 1_000.0 {
        format!("{} KB", (b / 1_000.0).round() as u64)
    } else {
        format!("{} B", b as u64)
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
#[path = "apple_messages_raw_tests.rs"]
mod tests;
