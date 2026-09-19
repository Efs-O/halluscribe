// HalluScribe - splits sorted Messages rows into session windows.
//
// Phase 5a of the Business Messaging ingestion plan. A conversation is split
// into windows at inactivity gaps greater than 30 days, and each window is
// capped at 400 messages. The input is already sorted by
// (conversation, date, rowid), so a conversation change is simply a boundary.
//
// Pure: no I/O, no allocation beyond the returned `Vec<Window>`. The two
// windowing constants live here, in one place (grep before adding more).

use crate::readers::apple_messages_db::{ConversationKey, RawMessage};
use chrono::Duration;
use std::ops::Range;

/// A window splits a conversation after an inactivity gap longer than this.
pub const WINDOW_GAP_DAYS: i64 = 30;
/// A window holds at most this many messages before a new one starts.
pub const WINDOW_MAX_MESSAGES: usize = 400;

/// One window of a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub conversation: ConversationKey,
    /// The `guid` of the window's first message.
    pub first_guid: String,
    /// `apple_messages-<sanitized first_guid>`, stable across re-splits.
    pub session_id: String,
    /// Inclusive start / exclusive end index into the input slice.
    pub range: Range<usize>,
}

/// Split `messages` (already sorted by conversation, date, rowid) into windows.
/// A new window starts on a new conversation, on a gap greater than
/// `WINDOW_GAP_DAYS` from the previous message, or when the current window
/// already holds `WINDOW_MAX_MESSAGES` messages. `provider_key` prefixes the
/// session id (e.g. `apple_messages` or `whatsapp`), so the same logic serves
/// every business-messaging reader.
pub fn windows(messages: &[RawMessage], provider_key: &str) -> Vec<Window> {
    let mut out: Vec<Window> = Vec::new();
    if messages.is_empty() {
        return out;
    }

    let mut start = 0usize;
    let mut count = 0usize;
    let mut prev_date: Option<chrono::DateTime<chrono::Utc>> = None;

    for i in 0..messages.len() {
        let msg = &messages[i];
        let new_conversation = i > 0 && messages[i - 1].conversation != msg.conversation;
        let cap_reached = count >= WINDOW_MAX_MESSAGES;
        let gap_too_large = prev_date
            .map(|prev| msg.date_utc.signed_duration_since(prev))
            .is_some_and(|d| d > Duration::days(WINDOW_GAP_DAYS));

        if i > 0 && (new_conversation || cap_reached || gap_too_large) {
            out.push(make_window(messages, start, count, provider_key));
            start = i;
            count = 0;
        }
        count += 1;
        prev_date = Some(msg.date_utc);
    }
    out.push(make_window(messages, start, count, provider_key));
    out
}

/// Build the `Window` for `messages[start..start + count]`.
fn make_window(messages: &[RawMessage], start: usize, count: usize, provider_key: &str) -> Window {
    let first = &messages[start];
    let first_guid = first.guid.clone();
    Window {
        conversation: first.conversation.clone(),
        first_guid: first_guid.clone(),
        session_id: format!("{provider_key}-{}", sanitize(&first_guid)),
        range: start..(start + count),
    }
}

/// Map every char outside `[A-Za-z0-9_-]` to `_`, so a `guid` becomes a safe
/// session-id fragment.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "apple_messages_window_tests.rs"]
mod tests;
