// HalluScribe - deterministic, hand-rolled secret-shape scanner (no regex dep).
// Pure text-in, flag-ids-out. No I/O. Used at sweep time to flag (never
// auto-redact) high-confidence secret shapes in archived session markdown.

const CREDENTIAL_KEYWORDS: [&str; 6] = [
    "password", "passwd", "secret", "api_key", "api-key", "apikey",
];
const PLACEHOLDER_VALUES: [&str; 9] = [
    "null", "none", "true", "false", "example", "changeme", "password", "secret", "xxxx",
];
const PLACEHOLDER_LEAD_CHARS: [char; 5] = ['[', '<', '$', '{', '*'];

type Matcher = fn(&str) -> bool;

const RULES: &[(&str, Matcher)] = &[
    ("aws_access_key", has_aws_access_key),
    ("openai_key", has_openai_key),
    ("github_token", has_github_token),
    ("slack_token", has_slack_token),
    ("private_key", has_private_key),
    ("credential_assignment", has_credential_assignment),
    ("url_credentials", has_url_credentials),
];

/// Scan `text` for high-confidence secret shapes and return a sorted,
/// deduplicated list of flag ids. Never mutates or redacts anything.
pub fn scan_for_secrets(text: &str) -> Vec<String> {
    let mut flags: Vec<String> = RULES
        .iter()
        .filter(|(_, matcher)| matcher(text))
        .map(|(id, _)| id.to_string())
        .collect();
    flags.sort();
    flags.dedup();
    flags
}

fn prev_char_alnum(text: &str, idx: usize) -> bool {
    text[..idx]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric())
}

fn next_char_alnum(text: &str, idx: usize) -> bool {
    text[idx..]
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric())
}

fn ceil_char_boundary(text: &str, idx: usize) -> usize {
    let mut idx = idx.min(text.len());
    while idx < text.len() && !text.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

fn has_aws_access_key(text: &str) -> bool {
    for (start, _) in text.match_indices("AKIA") {
        if prev_char_alnum(text, start) {
            continue;
        }
        let tail_start = start + 4;
        let tail_end = tail_start + 16;
        if tail_end > text.len() || !text.is_char_boundary(tail_end) {
            continue;
        }
        let tail = &text[tail_start..tail_end];
        if !tail
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            continue;
        }
        if !next_char_alnum(text, tail_end) {
            return true;
        }
    }
    false
}

fn has_openai_key(text: &str) -> bool {
    for (start, _) in text.match_indices("sk-") {
        if prev_char_alnum(text, start) {
            continue;
        }
        let tail_start = start + 3;
        if tail_start > text.len() {
            continue;
        }
        let rest = &text[tail_start..];
        let valid_len = rest
            .chars()
            .take_while(|&c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            .count();
        if valid_len >= 20 {
            return true;
        }
    }
    false
}

fn has_prefixed_token(text: &str, prefixes: &[&str], min_len: usize, extra_char: char) -> bool {
    for prefix in prefixes {
        for (start, _) in text.match_indices(prefix) {
            let tail_start = start + prefix.len();
            if tail_start > text.len() {
                continue;
            }
            let rest = &text[tail_start..];
            let valid_len = rest
                .chars()
                .take_while(|&c| c.is_ascii_alphanumeric() || c == extra_char)
                .count();
            if valid_len >= min_len {
                return true;
            }
        }
    }
    false
}

fn has_github_token(text: &str) -> bool {
    const PREFIXES: [&str; 5] = ["ghp_", "gho_", "ghs_", "ghr_", "github_pat_"];
    has_prefixed_token(text, &PREFIXES, 20, '_')
}

fn has_slack_token(text: &str) -> bool {
    const PREFIXES: [&str; 4] = ["xoxb-", "xoxp-", "xoxa-", "xoxs-"];
    has_prefixed_token(text, &PREFIXES, 10, '-')
}

fn has_private_key(text: &str) -> bool {
    if text.contains("-----BEGIN PRIVATE KEY-----") {
        return true;
    }
    const MARKER: &str = "-----BEGIN ";
    const LOOKAHEAD: usize = 40;
    for (start, _) in text.match_indices(MARKER) {
        let tail_start = start + MARKER.len();
        if tail_start > text.len() {
            continue;
        }
        let window_end = ceil_char_boundary(text, tail_start + LOOKAHEAD);
        let window = &text[tail_start..window_end];
        if window.contains("PRIVATE KEY-----") {
            return true;
        }
    }
    false
}

fn is_placeholder_value(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }
    if let Some(first) = trimmed.chars().next() {
        if PLACEHOLDER_LEAD_CHARS.contains(&first) {
            return true;
        }
    }
    let lower = trimmed.to_ascii_lowercase();
    PLACEHOLDER_VALUES.contains(&lower.as_str())
}

fn extract_assignment_value(rest: &str) -> Option<&str> {
    let bytes = rest.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() && (bytes[idx] == b' ' || bytes[idx] == b'\t') {
        idx += 1;
    }
    if idx >= bytes.len() || (bytes[idx] != b':' && bytes[idx] != b'=') {
        return None;
    }
    idx += 1;
    while idx < bytes.len() && (bytes[idx] == b' ' || bytes[idx] == b'\t') {
        idx += 1;
    }
    let quote = if idx < bytes.len() && (bytes[idx] == b'\'' || bytes[idx] == b'"') {
        let q = bytes[idx];
        idx += 1;
        Some(q)
    } else {
        None
    };
    let value_start = idx;
    let value_end = match quote {
        Some(q) => match bytes[value_start..].iter().position(|&b| b == q) {
            Some(p) => value_start + p,
            None => bytes.len(),
        },
        None => {
            let mut end = value_start;
            while end < bytes.len() && !(bytes[end] as char).is_whitespace() {
                end += 1;
            }
            end
        }
    };
    if value_end <= value_start {
        return None;
    }
    Some(&rest[value_start..value_end])
}

fn has_credential_assignment(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    for keyword in CREDENTIAL_KEYWORDS {
        for (start, _) in lower.match_indices(keyword) {
            if prev_char_alnum(&lower, start) {
                continue;
            }
            let after_kw = start + keyword.len();
            if after_kw > lower.len() || next_char_alnum(&lower, after_kw) {
                continue;
            }
            let Some(value) = extract_assignment_value(&text[after_kw..]) else {
                continue;
            };
            if value.chars().count() < 4 {
                continue;
            }
            if !is_placeholder_value(value) {
                return true;
            }
        }
    }
    false
}

fn has_url_credentials(text: &str) -> bool {
    for (start, _) in text.match_indices("://") {
        if start < 2 {
            continue;
        }
        let scheme_start = start - 2;
        if !text.is_char_boundary(scheme_start) {
            continue;
        }
        if !text[scheme_start..start]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric())
        {
            continue;
        }
        let after = start + 3;
        if after > text.len() {
            continue;
        }
        let rest = &text[after..];
        let authority_len = rest
            .find(|c: char| c == '/' || c.is_whitespace())
            .unwrap_or(rest.len());
        let authority = &rest[..authority_len];
        let Some(colon_pos) = authority.find(':') else {
            continue;
        };
        let after_colon = &authority[colon_pos + 1..];
        let Some(at_pos) = after_colon.find('@') else {
            continue;
        };
        let pass = &after_colon[..at_pos];
        if pass.is_empty() {
            continue;
        }
        let first = pass.chars().next().expect("checked non-empty above");
        if first != '[' && first != '$' {
            return true;
        }
    }
    false
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod tests;
