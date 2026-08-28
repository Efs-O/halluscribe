// HalluScribe - output-token accounting for archived sessions.
// One home for "how many tokens did the model generate in this session",
// deliberately separate from `preprocessor`, which owns transcript rendering.

/// Chars per token for the fallback estimate. A rough average: close for
/// English prose, optimistic for dense code, and well off for Greek and other
/// scripts that tokenize far less efficiently. Only ever applied to text the
/// model produced, never to a whole transcript.
const CHARS_PER_TOKEN: usize = 4;

/// Total tokens the model generated in one session, and whether that number was
/// reported by the server or inferred from character counts.
///
/// The flag is not decoration. A measured count from Claude Code includes
/// hidden thinking tokens; an estimate derived from a chat export cannot see
/// them at all, because the provider strips reasoning traces before export. The
/// two are not comparable, so anything that sorts or totals them has to be able
/// to tell them apart.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenCount {
    pub output: u64,
    pub estimated: bool,
}

impl TokenCount {
    /// Server-reported. Includes thinking tokens wherever the server counts them.
    pub fn measured(output: u64) -> Self {
        Self {
            output,
            estimated: false,
        }
    }

    /// Inferred from characters of generated text.
    pub fn estimated(output: u64) -> Self {
        Self {
            output,
            estimated: true,
        }
    }

    /// Nothing to report — no usage data and no generated text to measure.
    pub fn unknown() -> Self {
        Self::estimated(0)
    }

    /// `measured` when the server reported anything at all, else the estimate.
    ///
    /// A server that reports zero output tokens is reporting a real zero, so
    /// `Some(0)` stays measured; only the absence of usage data falls back.
    pub fn from_usage_or_chars(reported: Option<u64>, generated_chars: usize) -> Self {
        match reported {
            Some(output) => Self::measured(output),
            None => Self::estimated(estimate_from_chars(generated_chars)),
        }
    }
}

/// Tokens implied by a character count of model-generated text.
pub fn estimate_from_chars(chars: usize) -> u64 {
    (chars / CHARS_PER_TOKEN) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_divides_by_chars_per_token() {
        assert_eq!(estimate_from_chars(400), 100);
        assert_eq!(estimate_from_chars(0), 0);
        // Rounds down rather than reporting a token for a stray character.
        assert_eq!(estimate_from_chars(3), 0);
    }

    #[test]
    fn reported_usage_wins_over_characters() {
        let count = TokenCount::from_usage_or_chars(Some(1234), 999_999);
        assert_eq!(count, TokenCount::measured(1234));
    }

    #[test]
    fn a_reported_zero_is_still_measured() {
        // The server saying "zero" is data; only missing usage is a guess.
        let count = TokenCount::from_usage_or_chars(Some(0), 4000);
        assert!(!count.estimated);
        assert_eq!(count.output, 0);
    }

    #[test]
    fn missing_usage_falls_back_to_the_estimate() {
        let count = TokenCount::from_usage_or_chars(None, 4000);
        assert_eq!(count, TokenCount::estimated(1000));
    }

    #[test]
    fn unknown_is_an_estimated_zero() {
        assert!(TokenCount::unknown().estimated);
        assert_eq!(TokenCount::unknown().output, 0);
    }
}
