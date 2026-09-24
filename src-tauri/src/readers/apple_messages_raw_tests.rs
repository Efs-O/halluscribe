// HalluScribe - tests for the Apple Messages raw renderer (I.6). All fixtures
// are synthetic: an in-memory `ContactBook` and hand-built conversations. The
// golden pins the FORMAT by computing the local-time strings the same way the
// renderer does, so it is correct on any machine's time zone.

use super::{attachment_line, conversation_org, conversation_title, human_size, render_raw};
use crate::apple_backup::contacts::{Contact, ContactBook};
use crate::readers::apple_messages_db::{Conversation, ConversationKey, RawAttachment, RawMessage};
use chrono::{DateTime, Local, TimeZone, Utc};
use std::collections::HashMap;

/// A two-person address book: Nikos (phone + org) and Maria (email).
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
    people.insert(
        2,
        Contact {
            id: 2,
            name: "Maria".to_string(),
            organization: None,
        },
    );
    let mut by_phone: HashMap<String, i64> = HashMap::new();
    by_phone.insert("+306912345678".to_string(), 1);
    let mut by_email: HashMap<String, i64> = HashMap::new();
    by_email.insert("maria@example.com".to_string(), 2);
    ContactBook {
        by_phone,
        by_phone_alt: HashMap::new(),
        by_email,
        people,
    }
}

fn one2one() -> Conversation {
    Conversation {
        key: ConversationKey::Chat(1),
        chat_guid: Some("iMessage;-;+306912345678".to_string()),
        display_name: None,
        service: Some("iMessage".to_string()),
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
        guid: "g".to_string(),
        conversation: ConversationKey::Chat(1),
        date_utc: date,
        is_from_me,
        sender_handle: sender.map(|s| s.to_string()),
        service: Some("iMessage".to_string()),
        text: text.map(|t| t.to_string()),
        attachments: vec![],
    }
}

/// The local-time strings, computed exactly as the renderer does, so the
/// golden is independent of the machine's time zone.
fn ts(d: DateTime<Utc>) -> String {
    d.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M:%S %z")
        .to_string()
}
fn date(d: DateTime<Utc>) -> String {
    d.with_timezone(&Local).format("%Y-%m-%d").to_string()
}

#[test]
fn render_raw_one_to_one_is_the_i6_golden() {
    let b = book();
    let cc = Some("30");
    let conv = one2one();
    let d1 = Utc.with_ymd_and_hms(2026, 4, 14, 7, 32, 5).unwrap();
    let d2 = Utc.with_ymd_and_hms(2026, 4, 14, 7, 35, 40).unwrap();
    let msgs = vec![
        msg(true, None, Some("Yes, I have three slabs available."), d1),
        msg(
            false,
            Some("+306912345678"),
            Some("Do you still have Calacatta Light in 2 cm?"),
            d2,
        ),
    ];
    let got = render_raw(&conv, &msgs, &b, cc, "00008020-TEST");
    let expected = format!(
        "# Messages — Nikos Papadopoulos — ABC Marble Ltd\n\
         # chat: iMessage;-;+306912345678 · service: iMessage · participants: Me; Nikos Papadopoulos (+306912345678)\n\
         # window: {} → {} · 2 messages · backup: 00008020-TEST\n\
         \n\
         [{}] Me\n\
         Yes, I have three slabs available.\n\
         \n\
         [{}] Nikos Papadopoulos | +306912345678 | 6912345678\n\
         Do you still have Calacatta Light in 2 cm?",
        date(d1),
        date(d2),
        ts(d1),
        ts(d2)
    );
    assert_eq!(got, expected);
}

#[test]
fn attachment_line_uses_transfer_name_mime_and_size() {
    let a = RawAttachment {
        transfer_name: Some("IMG_2231.HEIC".to_string()),
        mime_type: Some("image/heic".to_string()),
        total_bytes: Some(2_100_000),
    };
    assert_eq!(
        attachment_line(&a),
        "[attachment: IMG_2231.HEIC · image/heic · 2.1 MB]"
    );
}

#[test]
fn attachment_line_uses_placeholders_when_fields_missing() {
    let a = RawAttachment {
        transfer_name: None,
        mime_type: None,
        total_bytes: None,
    };
    assert_eq!(
        attachment_line(&a),
        "[attachment: unnamed · unknown · unknown]"
    );
}

#[test]
fn human_size_formats_each_unit() {
    assert_eq!(human_size(Some(512)), "512 B");
    assert_eq!(human_size(Some(340_000)), "340 KB");
    assert_eq!(human_size(Some(2_100_000)), "2.1 MB");
    assert_eq!(human_size(Some(1_500_000_000)), "1.5 GB");
    assert_eq!(human_size(None), "unknown");
}

#[test]
fn unresolved_handle_is_rendered_verbatim() {
    let b = book(); // no entry for +15551234567
    let cc = Some("30");
    let conv = Conversation {
        key: ConversationKey::OrphanHandle(9),
        chat_guid: None,
        display_name: None,
        service: None,
        participants: vec!["+15551234567".to_string()],
    };
    let d = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let msgs = vec![msg(false, Some("+15551234567"), Some("hello"), d)];
    let got = render_raw(&conv, &msgs, &b, cc, "B");
    assert!(got.starts_with("# Messages — +15551234567\n"));
    assert!(got.contains("participants: Me; +15551234567"));
    assert!(got.contains("] +15551234567\nhello"));
}

#[test]
fn email_contact_shows_name_and_email() {
    let b = book();
    let cc = Some("30");
    let conv = Conversation {
        key: ConversationKey::Chat(2),
        chat_guid: Some("email;maria@example.com".to_string()),
        display_name: None,
        service: Some("email".to_string()),
        participants: vec!["maria@example.com".to_string()],
    };
    let d = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let msgs = vec![msg(false, Some("maria@example.com"), Some("hi"), d)];
    let got = render_raw(&conv, &msgs, &b, cc, "B");
    assert!(got.starts_with("# Messages — Maria\n")); // no org for Maria
    assert!(got.contains("participants: Me; Maria (maria@example.com)"));
    assert!(got.contains("] Maria | maria@example.com\n"));
}

#[test]
fn conversation_org_is_the_resolved_org_for_one_to_one() {
    let b = book();
    assert_eq!(
        conversation_org(&one2one(), &b, Some("30")),
        Some("ABC Marble Ltd".to_string())
    );
}

#[test]
fn conversation_org_is_none_for_a_group() {
    let b = book();
    let conv = Conversation {
        key: ConversationKey::Chat(3),
        chat_guid: Some("g".to_string()),
        display_name: Some("Team".to_string()),
        service: None,
        participants: vec!["+306912345678".to_string(), "+15551234567".to_string()],
    };
    assert_eq!(conversation_org(&conv, &b, Some("30")), None);
}

#[test]
fn conversation_org_is_none_when_contact_has_no_org() {
    let b = book();
    let conv = Conversation {
        key: ConversationKey::OrphanHandle(2),
        chat_guid: None,
        display_name: None,
        service: None,
        participants: vec!["maria@example.com".to_string()],
    };
    assert_eq!(conversation_org(&conv, &b, Some("30")), None);
}

#[test]
fn group_title_prefers_display_name() {
    let b = book();
    let conv = Conversation {
        key: ConversationKey::Chat(3),
        chat_guid: Some("g".to_string()),
        display_name: Some("The Team".to_string()),
        service: None,
        participants: vec!["+306912345678".to_string(), "+15551234567".to_string()],
    };
    assert_eq!(conversation_title(&conv, &b, Some("30")), "The Team");
}

#[test]
fn group_title_without_display_name_joins_labels() {
    let b = book();
    let conv = Conversation {
        key: ConversationKey::Chat(3),
        chat_guid: Some("g".to_string()),
        display_name: None,
        service: None,
        participants: vec!["+306912345678".to_string(), "+15551234567".to_string()],
    };
    // +306912345678 resolves to Nikos; +15551234567 is unresolved (verbatim).
    assert_eq!(
        conversation_title(&conv, &b, Some("30")),
        "Nikos Papadopoulos, +15551234567"
    );
}

#[test]
fn group_title_caps_participants_at_four_then_plus_n() {
    let b = book();
    let conv = Conversation {
        key: ConversationKey::Chat(3),
        chat_guid: Some("g".to_string()),
        display_name: None,
        service: None,
        participants: vec![
            "a@x.com".to_string(),
            "b@x.com".to_string(),
            "c@x.com".to_string(),
            "d@x.com".to_string(),
            "e@x.com".to_string(),
        ],
    };
    assert_eq!(
        conversation_title(&conv, &b, Some("30")),
        "a@x.com, b@x.com, c@x.com, d@x.com, +1"
    );
}

#[test]
fn one_to_one_title_is_the_resolved_name() {
    let b = book();
    assert_eq!(
        conversation_title(&one2one(), &b, Some("30")),
        "Nikos Papadopoulos"
    );
}
