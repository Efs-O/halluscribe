// HalluScribe - tests for Messages windowing. All fixtures are synthetic.

use super::{windows, WINDOW_GAP_DAYS, WINDOW_MAX_MESSAGES};
use crate::readers::apple_messages_db::{ConversationKey, RawMessage};
use chrono::{TimeZone, Utc};

fn msg(guid: &str, conv: ConversationKey, date: chrono::DateTime<Utc>) -> RawMessage {
    RawMessage {
        rowid: 0,
        guid: guid.to_string(),
        conversation: conv,
        date_utc: date,
        is_from_me: false,
        sender_handle: None,
        service: None,
        text: Some("x".to_string()),
        attachments: Vec::new(),
    }
}

fn at(days: i64) -> chrono::DateTime<Utc> {
    // A fixed base instant plus `days`.
    let base = Utc
        .with_ymd_and_hms(2020, 1, 1, 0, 0, 0)
        .single()
        .expect("valid");
    base + chrono::Duration::days(days)
}

const C1: ConversationKey = ConversationKey::Chat(1);

#[test]
fn a_31_day_gap_splits_but_exactly_30_does_not() {
    // Two messages 30 days apart: one window (gap is not > 30).
    let within = vec![msg("a", C1, at(0)), msg("b", C1, at(30))];
    assert_eq!(windows(&within).len(), 1);

    // Two messages 31 days apart: two windows (gap is > 30).
    let across = vec![msg("a", C1, at(0)), msg("b", C1, at(31))];
    assert_eq!(windows(&across).len(), 2);
}

#[test]
fn a_401_message_run_splits_into_400_plus_1() {
    let mut messages: Vec<RawMessage> = Vec::new();
    for i in 0..401 {
        messages.push(msg(&format!("m{i}"), C1, at(0)));
    }
    let ws = windows(&messages);
    assert_eq!(ws.len(), 2);
    assert_eq!(ws[0].range.len(), WINDOW_MAX_MESSAGES);
    assert_eq!(ws[1].range.len(), 1);
    assert_eq!(ws[1].first_guid, "m400");
}

#[test]
fn a_conversation_change_splits() {
    let messages = vec![
        msg("a", ConversationKey::Chat(1), at(0)),
        msg("b", ConversationKey::Chat(2), at(1)),
        msg("c", ConversationKey::Chat(1), at(2)),
    ];
    let ws = windows(&messages);
    assert_eq!(ws.len(), 3);
    assert_eq!(ws[0].conversation, ConversationKey::Chat(1));
    assert_eq!(ws[1].conversation, ConversationKey::Chat(2));
    assert_eq!(ws[2].conversation, ConversationKey::Chat(1));
}

#[test]
fn the_session_id_sanitizes_the_first_guid() {
    // A guid with `;`, `+` and `/` becomes all underscores.
    let messages = vec![msg("a;b+c/d", C1, at(0))];
    let ws = windows(&messages);
    assert_eq!(ws[0].session_id, "apple_messages-a_b_c_d");
}

#[test]
fn appending_one_message_leaves_earlier_windows_unchanged() {
    let base = vec![
        msg("a", C1, at(0)),
        msg("b", C1, at(1)),
        msg("c", C1, at(2)),
    ];
    let before = windows(&base);

    // Append a message that does NOT open a new window (same conversation,
    // within the gap, under the cap).
    let extended = vec![
        msg("a", C1, at(0)),
        msg("b", C1, at(1)),
        msg("c", C1, at(2)),
        msg("d", C1, at(3)),
    ];
    let after = windows(&extended);

    assert_eq!(after.len(), before.len());
    for (b, a) in before.iter().zip(after.iter()) {
        assert_eq!(b.session_id, a.session_id);
        assert_eq!(b.range.start, a.range.start);
    }
    // The last window grew by one; its id (from the first message) is unchanged.
    assert_eq!(after[0].range, 0..4);
}

#[test]
fn the_gap_constant_is_thirty_days() {
    assert_eq!(WINDOW_GAP_DAYS, 30);
    assert_eq!(WINDOW_MAX_MESSAGES, 400);
}
