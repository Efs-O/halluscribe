// HalluScribe - tests for the WhatsApp raw renderer (I.6). All fixtures are
// synthetic: an in-memory `ContactBook`, a push-name map and hand-built
// conversations. No real data.

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

/// A push-name map: the JID -> WhatsApp's own name for that contact.
fn push_names() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert(
        "+306912345678@s.whatsapp.net".to_string(),
        "Nikos P".to_string(),
    );
    m
}

fn one2one() -> Conversation {
    Conversation {
        key: ConversationKey::Chat(1),
        chat_guid: Some("+306912345678@s.whatsapp.net".to_string()),
        display_name: None,
        service: None,
        participants: vec!["+306912345678@s.whatsapp.net".to_string()],
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
        guid: "wa-1".to_string(),
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
fn the_push_name_wins_over_the_address_book() {
    // Both the push name and the address book know this contact: the push name
    // (WhatsApp's own) must win.
    let conv = one2one();
    let title = conversation_title(&conv, &push_names(), &book(), Some("30"));
    assert_eq!(title, "Nikos P");
}

#[test]
fn the_address_book_is_used_when_there_is_no_push_name() {
    // No push name for this JID: fall back to the address book via the number
    // in the JID.
    let conv = one2one();
    let empty: HashMap<String, String> = HashMap::new();
    let title = conversation_title(&conv, &empty, &book(), Some("30"));
    assert_eq!(title, "Nikos Papadopoulos");
}

#[test]
fn an_unresolved_jid_is_shown_verbatim() {
    // Neither the push name nor the address book knows this JID: show it
    // verbatim (D7: never a guess).
    let conv = Conversation {
        key: ConversationKey::Chat(1),
        chat_guid: Some("999@s.whatsapp.net".to_string()),
        display_name: None,
        service: None,
        participants: vec!["999@s.whatsapp.net".to_string()],
    };
    let title = conversation_title(&conv, &push_names(), &book(), Some("30"));
    assert_eq!(title, "999@s.whatsapp.net");
}

#[test]
fn a_group_title_uses_the_partner_name() {
    let conv = Conversation {
        key: ConversationKey::Chat(1),
        chat_guid: Some("222@g.us".to_string()),
        display_name: Some("Team Marble".to_string()),
        service: None,
        participants: vec![
            "333@s.whatsapp.net".to_string(),
            "444@s.whatsapp.net".to_string(),
        ],
    };
    let title = conversation_title(&conv, &push_names(), &book(), Some("30"));
    assert_eq!(title, "Team Marble");
}

#[test]
fn the_speaker_for_me_is_me() {
    let conv = one2one();
    let m = msg(true, None, Some("hi"), at());
    assert_eq!(
        speaker_for(&m, &conv, &push_names(), &book(), Some("30")),
        "Me"
    );
}

#[test]
fn the_speaker_for_a_resolved_contact_carries_name_and_e164() {
    let conv = one2one();
    let m = msg(
        false,
        Some("+306912345678@s.whatsapp.net"),
        Some("hi"),
        at(),
    );
    let speaker = speaker_for(&m, &conv, &push_names(), &book(), Some("30"));
    // Name (push) | E164 | national form.
    assert!(
        speaker.starts_with("Nikos P | +306912345678"),
        "got: {speaker}"
    );
    assert!(speaker.ends_with("6912345678"), "got: {speaker}");
}

#[test]
fn a_one_to_one_sender_without_a_from_jid_uses_the_contact_jid() {
    // The message carries no ZFROMJID; the 1:1 sender is the session's contact.
    let conv = one2one();
    let m = msg(false, None, Some("hi"), at());
    let speaker = speaker_for(&m, &conv, &push_names(), &book(), Some("30"));
    assert_eq!(speaker, "Nikos P | +306912345678 | 6912345678");
}

#[test]
fn the_org_for_a_one_to_one_is_the_contact_organization() {
    let conv = one2one();
    // The project label is the contact's organization (the client's business
    // name), not the name (which is already the title).
    let org = conversation_org(&conv, &book(), Some("30"));
    assert_eq!(org, Some("ABC Marble Ltd".to_string()));
}

#[test]
fn the_org_for_a_group_is_none() {
    let conv = Conversation {
        key: ConversationKey::Chat(1),
        chat_guid: Some("222@g.us".to_string()),
        display_name: Some("Team".to_string()),
        service: None,
        participants: vec![
            "333@s.whatsapp.net".to_string(),
            "444@s.whatsapp.net".to_string(),
        ],
    };
    assert_eq!(conversation_org(&conv, &book(), Some("30")), None);
}

#[test]
fn the_raw_slice_renders_header_and_messages() {
    let conv = one2one();
    let msgs = vec![
        msg(
            false,
            Some("+306912345678@s.whatsapp.net"),
            Some("Hello"),
            at(),
        ),
        msg(true, None, Some("Hi, how can I help?"), at()),
    ];
    let raw = render_raw(
        &conv,
        &msgs,
        &push_names(),
        &book(),
        Some("30"),
        "backup-xyz",
    );

    // The header line: title (push name) + the contact's organization.
    assert!(
        raw.starts_with("# WhatsApp — Nikos P — ABC Marble Ltd\n"),
        "got:\n{raw}"
    );
    // The chat line carries the JID.
    assert!(
        raw.contains("# chat: +306912345678@s.whatsapp.net"),
        "got:\n{raw}"
    );
    // The window line carries the backup label.
    assert!(raw.contains("backup: backup-xyz"), "got:\n{raw}");
    // Both messages are present, with speakers.
    assert!(raw.contains("Nikos P | +306912345678"), "got:\n{raw}");
    assert!(raw.contains("Me"), "got:\n{raw}");
    assert!(raw.contains("Hello"), "got:\n{raw}");
    assert!(raw.contains("Hi, how can I help?"), "got:\n{raw}");
}

#[test]
fn a_real_jid_without_a_plus_resolves_through_the_address_book() {
    // Real iOS JIDs carry the country code but no `+`. Before the fix the
    // number was read as national and got `+30` prepended a second time.
    let jid = "306912345678@s.whatsapp.net";
    let conv = Conversation {
        key: ConversationKey::Chat(1),
        chat_guid: Some(jid.to_string()),
        display_name: None,
        service: None,
        participants: vec![jid.to_string()],
    };
    let empty: HashMap<String, String> = HashMap::new();
    assert_eq!(
        conversation_title(&conv, &empty, &book(), Some("30")),
        "Nikos Papadopoulos"
    );
    assert_eq!(
        conversation_org(&conv, &book(), Some("30")).as_deref(),
        Some("ABC Marble Ltd")
    );
    let speaker = speaker_for(
        &msg(false, Some(jid), Some("hi"), at()),
        &conv,
        &empty,
        &book(),
        Some("30"),
    );
    assert_eq!(speaker, "Nikos Papadopoulos | +306912345678 | 6912345678");
}

#[test]
fn a_group_jid_is_never_read_as_a_phone_number() {
    // A group id is all digits too, but only `@s.whatsapp.net` JIDs are phones.
    let conv = Conversation {
        key: ConversationKey::Chat(1),
        chat_guid: Some("306912345678@g.us".to_string()),
        display_name: None,
        service: None,
        participants: vec!["306912345678@g.us".to_string()],
    };
    let empty: HashMap<String, String> = HashMap::new();
    assert_eq!(
        conversation_title(&conv, &empty, &book(), Some("30")),
        "306912345678@g.us"
    );
}
