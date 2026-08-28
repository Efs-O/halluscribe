// HalluScribe - low-signal session eligibility for unattended sweeps.
//
// A Codex context-fill percentage includes the IDE and agent instruction
// envelope, so it is not evidence that the actual user exchange is worth
// archiving. Keep this classifier intentionally narrow: genuine work always
// reaches the summariser, while known greetings and capability probes do not.

use crate::readers::{ChatProvider, ParsedSession};

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
}
