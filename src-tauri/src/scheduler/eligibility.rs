// HalluScribe - low-signal session eligibility for unattended sweeps.
//
// A Codex context-fill percentage includes the IDE and agent instruction
// envelope, so it is not evidence that the actual user exchange is worth
// archiving. Keep this classifier intentionally narrow: genuine work always
// reaches the summariser, while known greetings and capability probes do not.

use crate::readers::{ChatProvider, MessageRole, ParsedMessage, ParsedSession};

/// A business (phone) chat session needs at least this many messages to be
/// summarised. Shorter ones are one-liners ("ok thanks") the summary adds
/// nothing to.
pub(super) const BUSINESS_MIN_MESSAGES: usize = 3;

/// A phone chat session not worth a summary: the owner never wrote in it
/// (spam, bank codes, courier and shop notices) or it has fewer than
/// `BUSINESS_MIN_MESSAGES` messages. Decided per session, not per contact, so
/// a month of unanswered notices from a contact is skipped even when the owner
/// replied to that contact another time. The owner's messages are the
/// `Assistant` role in every business reader.
pub(super) fn is_low_signal_business_session(session: &ParsedSession) -> bool {
    is_low_signal_business_chat(&session.provider, &session.messages)
}

fn is_low_signal_business_chat(provider: &ChatProvider, messages: &[ParsedMessage]) -> bool {
    provider.is_business()
        && (messages.len() < BUSINESS_MIN_MESSAGES
            || !messages
                .iter()
                .any(|message| message.role == MessageRole::Assistant))
}

pub(super) fn is_low_signal_codex_session(session: &ParsedSession) -> bool {
    if session.provider != ChatProvider::Codex {
        return false;
    }
    let requests = session
        .transcript_units()
        .into_iter()
        .filter_map(|unit| unit.strip_prefix("[User]\n").map(normalize_request))
        .collect::<Vec<_>>();
    if requests.is_empty() {
        return false;
    }

    let conversation = requests.join(" ");
    is_external_capability_probe(&conversation)
        || requests
            .iter()
            .all(|request| is_greeting_or_presence_check(request))
}

fn normalize_request(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_greeting_or_presence_check(request: &str) -> bool {
    matches!(
        request,
        "hello"
            | "hi"
            | "hey"
            | "ping"
            | "pong"
            | "test"
            | "testing"
            | "are you here"
            | "are you there"
            | "are you online"
            | "you there"
    )
}

fn is_external_capability_probe(conversation: &str) -> bool {
    (conversation.contains("board")
        && (conversation.contains("monitor")
            || conversation.contains("post to the board")
            || conversation.contains("post on the board")))
        || conversation.contains("search previous session")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_observed_low_signal_request_shapes() {
        assert!(is_greeting_or_presence_check("hello"));
        assert!(is_greeting_or_presence_check("are you here"));
        assert!(is_external_capability_probe(
            "hey codex are you monitoring the board i posted"
        ));
        assert!(is_external_capability_probe(
            "can you search previous sessions"
        ));
    }

    #[test]
    fn does_not_classify_a_real_work_request_as_low_signal() {
        assert!(!is_greeting_or_presence_check("fix the authentication bug"));
        assert!(!is_external_capability_probe(
            "read the board component and fix its keyboard navigation"
        ));
    }

    fn chat(roles: &[MessageRole]) -> Vec<ParsedMessage> {
        roles
            .iter()
            .map(|role| ParsedMessage {
                role: role.clone(),
                text: "text".to_string(),
                timestamp: None,
                speaker: None,
            })
            .collect()
    }

    use MessageRole::{Assistant as Me, User as Them};

    #[test]
    fn a_business_chat_the_owner_never_wrote_in_is_low_signal() {
        let messages = chat(&[Them, Them, Them, Them]);
        assert!(is_low_signal_business_chat(
            &ChatProvider::WhatsApp,
            &messages
        ));
    }

    #[test]
    fn a_business_chat_under_three_messages_is_low_signal() {
        let messages = chat(&[Them, Me]);
        assert!(is_low_signal_business_chat(&ChatProvider::Viber, &messages));
    }

    #[test]
    fn a_business_chat_with_a_reply_and_three_messages_is_kept() {
        let messages = chat(&[Them, Me, Them]);
        for provider in [
            ChatProvider::AppleMessages,
            ChatProvider::WhatsApp,
            ChatProvider::WhatsAppBusiness,
            ChatProvider::Viber,
        ] {
            assert!(
                !is_low_signal_business_chat(&provider, &messages),
                "{provider:?}"
            );
        }
    }

    #[test]
    fn the_business_rule_never_touches_a_non_business_session() {
        let messages = chat(&[Them]);
        assert!(!is_low_signal_business_chat(
            &ChatProvider::ClaudeCode,
            &messages
        ));
    }
}
