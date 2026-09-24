// HalluScribe - tests for the Viber raw renderer (I.6). All fixtures are
// synthetic: an in-memory `ContactBook` and hand-built conversations. No real
// data.

use super::{conversation_org, conversation_title, render_raw, speaker_for};
use crate::apple_backup::contacts::{Contact, ContactBook};
use crate::readers::apple_messages_db::{Conversation, ConversationKey, RawMessage};
use chrono::{DateTime, TimeZone, Utc};
use std::collections::HashMap;

/// A one-person address book: Nikos, keyed by his E.164 phone.
fn book() -> ContactBook {
    let mut people: HashMap<i64, Contact> = HashMap::new();
    people.insert(
        1,
        Contact {
            id: 1,
            name: "Nikos Papadopoulos".to_string(),
            organization: Some("ABC Marble Ltd".to_string()),
        },
    );
    let mut by_phone: HashMap<String, i64> = HashMap::new();
    by_phone.insert("+306912345678".to_string(), 1);
    ContactBook {
        by_phone,
        by_phone_alt: HashMap::new(),
        by_email: HashMap::new(),
        people,
    }
}

fn one2one() -> Conversation {
    Conversation {
        key: ConversationKey::Chat(1),
        chat_guid: None,
        display_name: None,
        service: None,
        participants: vec!["+306912345678".to_string()],
    }
}

fn msg(
    is_from_me: bool,
    sender: Option<&str>,
    text: Option<&str>,
    date: DateTime<Utc>,
) -> RawMessage {
    RawMessage {
        rowid: 1,
        guid: "viber-1".to_string(),
        conversation: ConversationKey::Chat(1),
        date_utc: date,
        is_from_me,
        sender_handle: sender.map(|s| s.to_string()),
        service: None,
        text: text.map(|t| t.to_string()),
        attachments: vec![],
    }
}

fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 10, 30, 0)
        .single()
        .expect("valid")
}

#[test]
fn the_address_book_is_used_for_the_title() {
    // Viber has no push-name table: the address book is the only name source.
    let conv = one2one();
    let title = conversation_title(&conv, &book(), Some("30"));
    assert_eq!(title, "Nikos Papadopoulos");
}

#[test]
fn an_unresolved_phone_is_shown_verbatim() {
    // The address book does not know this number: show it verbatim (D7).
    let conv = Conversation {
        key: ConversationKey::Chat(1),
        chat_guid: None,
        display_name: None,
        service: None,
        participants: vec!["+30699999999".to_string()],
    };
    let title = conversation_title(&conv, &book(), Some("30"));
    assert_eq!(title, "+30699999999");
}

#[test]
fn a_group_title_uses_the_conversation_name() {
    let conv = Conversation {
        key: ConversationKey::Chat(1),
        chat_guid: None,
        display_name: Some("Team Marble".to_string()),
        service: None,
        participants: vec!["+306912345678".to_string(), "+306987654321".to_string()],
    };
    let title = conversation_title(&conv, &book(), Some("30"));
    assert_eq!(title, "Team Marble");
}

#[test]
fn the_speaker_for_me_is_me() {
    let conv = one2one();
    let m = msg(true, None, Some("hi"), at());
    assert_eq!(speaker_for(&m, &conv, &book(), Some("30")), "Me");
}

#[test]
fn the_speaker_for_a_resolved_contact_carries_name_and_e164() {
    let conv = one2one();
    let m = msg(false, Some("+306912345678"), Some("hi"), at());
    let speaker = speaker_for(&m, &conv, &book(), Some("30"));
    // Name | E164 | national form.
    assert!(
        speaker.starts_with("Nikos Papadopoulos | +306912345678"),
        "got: {speaker}"
    );
    assert!(speaker.ends_with("6912345678"), "got: {speaker}");
}

#[test]
fn a_one_to_one_sender_without_a_phone_uses_the_contact_phone() {
    // The message carries no sender phone; the 1:1 sender is the contact.
    let conv = one2one();
    let m = msg(false, None, Some("hi"), at());
    let speaker = speaker_for(&m, &conv, &book(), Some("30"));
    assert_eq!(speaker, "Nikos Papadopoulos | +306912345678 | 6912345678");
}

#[test]
fn a_received_row_with_no_sender_in_a_group_is_unknown() {
    // A received row with no sender phone, in a group: `Unknown`.
    let conv = Conversation {
        key: ConversationKey::Chat(1),
        chat_guid: None,
        display_name: Some("Team Marble".to_string()),
        service: None,
        participants: vec!["+306912345678".to_string(), "+306987654321".to_string()],
    };
    let m = msg(false, None, Some("hi"), at());
    let speaker = speaker_for(&m, &conv, &book(), Some("30"));
    assert_eq!(speaker, "Unknown");
}

#[test]
fn a_received_row_with_no_sender_in_a_one_to_one_uses_the_conv_name() {
    // A received row with no sender phone, in a 1:1 chat: the conversation
    // name, when it has one.
    let conv = Conversation {
        key: ConversationKey::Chat(1),
        chat_guid: None,
        display_name: Some("Nikos P".to_string()),
        service: None,
        participants: vec![],
    };
    let m = msg(false, None, Some("hi"), at());
    let speaker = speaker_for(&m, &conv, &book(), Some("30"));
    assert_eq!(speaker, "Nikos P");
}

#[test]
fn the_org_for_a_one_to_one_is_the_contact_organization() {
    let conv = one2one();
    let org = conversation_org(&conv, &book(), Some("30"));
    assert_eq!(org, Some("ABC Marble Ltd".to_string()));
}

#[test]
fn the_org_for_a_group_is_none() {
    let conv = Conversation {
        key: ConversationKey::Chat(1),
        chat_guid: None,
        display_name: Some("Team".to_string()),
        service: None,
        participants: vec!["+306912345678".to_string(), "+306987654321".to_string()],
    };
    assert_eq!(conversation_org(&conv, &book(), Some("30")), None);
}

#[test]
fn the_raw_slice_renders_header_and_messages() {
    let conv = one2one();
    let msgs = vec![
        msg(false, Some("+306912345678"), Some("Hello"), at()),
        msg(true, None, Some("Hi, how can I help?"), at()),
    ];
    let raw = render_raw(&conv, &msgs, &book(), Some("30"), "backup-xyz");

    // The header line: title (address book) + the contact's organization.
    assert!(
        raw.starts_with("# Viber — Nikos Papadopoulos — ABC Marble Ltd\n"),
        "got:\n{raw}"
    );
    // The window line carries the backup label.
    assert!(raw.contains("backup: backup-xyz"), "got:\n{raw}");
    // Both messages are present, with speakers.
    assert!(
        raw.contains("Nikos Papadopoulos | +306912345678"),
        "got:\n{raw}"
    );
    assert!(raw.contains("Me"), "got:\n{raw}");
    assert!(raw.contains("Hello"), "got:\n{raw}");
    assert!(raw.contains("Hi, how can I help?"), "got:\n{raw}");
}

#[test]
fn a_bare_international_sender_resolves_through_the_address_book() {
    // Viber stores numbers without the leading '+'; they must still match.
    let conv = one2one();
    let m = msg(false, Some("306912345678"), Some("hi"), at());
    let speaker = speaker_for(&m, &conv, &book(), Some("30"));
    assert_eq!(speaker, "Nikos Papadopoulos | +306912345678 | 6912345678");
}
