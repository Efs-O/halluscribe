// HalluScribe - query tokenization: quoted-phrase vs AND-of-terms parsing for keyword search.

/// Result of parsing a raw search query string (see `parse_query`).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ParsedQuery {
    /// A `"quoted phrase"` query: exact-substring mode, verbatim old behaviour.
    /// The string is already lowercased and has its surrounding quotes stripped.
    Phrase(String),
    /// An unquoted query: AND-of-terms mode. Every token must match somewhere
    /// in the entry's searchable fields. Never empty (see `tokenize`).
    Tokens(Vec<String>),
}

/// ASCII English function words dropped from unquoted queries so that fluent
/// agent phrasing ("has the Forge bridge been implemented") reduces to its
/// distinctive nouns. Deliberately ASCII-only: non-ASCII (e.g. Greek) tokens
/// are never touched by this list, so they can't accidentally be stripped.
/// The interrogatives are kept as one complete family on purpose. "what", "how"
/// and "when" were stripped while "where", "why", "which" and "who" were not,
/// which silently broke a whole class of question. Measured 2026-08-04: a
/// session whose body contains the board heading "| task | who wins |" matches
/// `gemma wins` and `who wins`, but NOT `where gemma wins` — the stray `where`
/// is ANDed in and no session contains it. Retrieval failed on the user's
/// phrasing while the evidence sat in the index.
const STOP: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "has", "have", "had", "do", "does",
    "did", "it", "in", "on", "of", "to", "for", "and", "or", "not", "what", "how", "when", "where",
    "why", "which", "who", "this", "that", "with",
];

/// Characters (beyond whitespace) that split a query into tokens.
const SPLIT_PUNCTUATION: &[char] = &['?', ',', '.', '!', ':', ';', '"', '\'', '(', ')'];

/// Parse a raw search query string per the tokenization plan §3.1:
/// 1. Trim. If it starts and ends with `"` with >=1 char inside, treat it as
///    an exact-substring phrase (today's behaviour, verbatim).
/// 2. Otherwise lowercase, split on whitespace/punctuation, drop empties and
///    stop-words - falling back to the un-stopped token list if stripping
///    stop-words would empty it (a query is never reduced to zero tokens).
pub(super) fn parse_query(query: &str) -> ParsedQuery {
    let trimmed = query.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        if !inner.is_empty() {
            return ParsedQuery::Phrase(inner.to_lowercase());
        }
    }
    let tokens = tokenize(trimmed);
    if tokens.is_empty() {
        // Punctuation/whitespace-only input tokenizes to nothing; an empty
        // AND would vacuously match every session. Fall back to exact
        // substring on the raw text (old behaviour: garbage in, 0 hits out).
        return ParsedQuery::Phrase(trimmed.to_lowercase());
    }
    ParsedQuery::Tokens(tokens)
}

/// Lowercase, split on whitespace + punctuation, drop empties and stop-words.
/// Never returns an empty vec for a non-empty input: if removing stop-words
/// would empty the token list, the un-stopped tokens are kept instead.
fn tokenize(query: &str) -> Vec<String> {
    let raw: Vec<String> = query
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || SPLIT_PUNCTUATION.contains(&c))
        .filter(|token| !token.is_empty())
        .map(String::from)
        .collect();

    let stopped: Vec<String> = raw
        .iter()
        .filter(|token| !STOP.contains(&token.as_str()))
        .cloned()
        .collect();

    if stopped.is_empty() {
        raw
    } else {
        stopped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_phrase_strips_quotes_and_lowercases() {
        match parse_query("\"Forge MCP client bridge\"") {
            ParsedQuery::Phrase(phrase) => assert_eq!(phrase, "forge mcp client bridge"),
            ParsedQuery::Tokens(_) => panic!("expected Phrase"),
        }
    }

    #[test]
    fn stop_words_are_stripped() {
        match parse_query("has the Forge bridge been implemented") {
            ParsedQuery::Tokens(tokens) => {
                assert_eq!(tokens, vec!["forge", "bridge", "implemented"]);
            }
            ParsedQuery::Phrase(_) => panic!("expected Tokens"),
        }
    }

    #[test]
    fn interrogatives_are_stripped_as_one_family() {
        // Regression for the 2026-08-04 retrieval failure: "where" was ANDed
        // into the query while its siblings "what"/"how"/"when" were stripped,
        // so "where gemma wins" required a literal "where" in the session and
        // matched nothing. All interrogatives must behave the same way.
        for query in [
            "what gemma wins",
            "how gemma wins",
            "when gemma wins",
            "where gemma wins",
            "why gemma wins",
            "which gemma wins",
            "who gemma wins",
        ] {
            match parse_query(query) {
                ParsedQuery::Tokens(tokens) => {
                    assert_eq!(tokens, vec!["gemma", "wins"], "query: {query}");
                }
                ParsedQuery::Phrase(_) => panic!("expected Tokens for {query}"),
            }
        }
    }

    #[test]
    fn quoted_interrogative_phrase_is_still_exact() {
        // Stripping applies to unquoted token queries only — a quoted phrase
        // must stay verbatim, so "who wins" can still be searched exactly.
        match parse_query("\"who wins\"") {
            ParsedQuery::Phrase(phrase) => assert_eq!(phrase, "who wins"),
            ParsedQuery::Tokens(_) => panic!("expected Phrase"),
        }
    }

    #[test]
    fn stop_word_only_query_falls_back_to_raw_tokens() {
        // Every word here ("has", "it", "been") is a stop-word, so stripping
        // them would empty the token list - fall back to the raw tokens
        // instead of ever returning zero tokens.
        match parse_query("has it been?") {
            ParsedQuery::Tokens(tokens) => {
                assert!(!tokens.is_empty());
                assert_eq!(tokens, vec!["has", "it", "been"]);
            }
            ParsedQuery::Phrase(_) => panic!("expected Tokens"),
        }
    }

    #[test]
    fn greek_tokens_are_preserved() {
        match parse_query("διόρθωση σφάλματος") {
            ParsedQuery::Tokens(tokens) => {
                assert_eq!(tokens, vec!["διόρθωση", "σφάλματος"]);
            }
            ParsedQuery::Phrase(_) => panic!("expected Tokens"),
        }
    }

    #[test]
    fn punctuation_splits_tokens() {
        match parse_query("mcpBridge, implementation!") {
            ParsedQuery::Tokens(tokens) => {
                assert_eq!(tokens, vec!["mcpbridge", "implementation"]);
            }
            ParsedQuery::Phrase(_) => panic!("expected Tokens"),
        }
    }

    #[test]
    fn punctuation_only_query_falls_back_to_phrase_mode() {
        // "???" has no tokens after splitting; it must NOT become an empty
        // AND (which would match everything) - exact-substring instead.
        match parse_query("???") {
            ParsedQuery::Phrase(phrase) => assert_eq!(phrase, "???"),
            ParsedQuery::Tokens(_) => panic!("expected Phrase fallback"),
        }
    }

    #[test]
    fn single_word_passthrough() {
        match parse_query("auth") {
            ParsedQuery::Tokens(tokens) => assert_eq!(tokens, vec!["auth"]),
            ParsedQuery::Phrase(_) => panic!("expected Tokens"),
        }
    }
}
