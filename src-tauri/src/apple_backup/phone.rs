// HalluScribe - phone-number normalization for business-messaging handles.
//
// Turns a raw SMS handle into an E.164 international form (`+<cc><number>`)
// so that two spellings of the same number land on the same key. This is the
// D8 rule: normalization needs a user-set default country code; without one,
// only already-international numbers (`+…`, `00…`) are normalized and local
// numbers are matched verbatim (never guessed).
//
// Pure and total: `normalize` and `national_form` never panic on any input and
// return `None` for anything that is not a well-formed phone number. No
// logging of numbers here.

/// Normalize a raw handle into E.164 form, or `None` if it is not a phone
/// number or cannot be made international without guessing (D8).
///
/// `default_cc` is the user's default country code (e.g. `"30"` or `"+30"`).
/// It may be `None` or blank, in which case only already-international inputs
/// are normalized.
pub fn normalize(raw: &str, default_cc: Option<&str>) -> Option<String> {
    // 1. Trim, then drop the formatting characters. What remains must be an
    //    optional leading '+' followed by ASCII digits only - this is what
    //    rejects emails and alphanumeric sender IDs.
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '.' | '(' | ')'))
        .collect();

    let (has_plus, digits) = match cleaned.strip_prefix('+') {
        Some(rest) => (true, rest),
        None => (false, cleaned.as_str()),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    // 2. An explicit `+` is already international - keep it as is. A `00`
    //    after the `+` is not stripped, so `finish` rejects it (malformed).
    if has_plus {
        return finish(digits);
    }
    // 3. A leading `00` is the international prefix - replace it with `+`.
    if let Some(stripped) = digits.strip_prefix("00") {
        return finish(stripped);
    }
    // 4. Otherwise it is a national number: it needs a valid default country
    //    code, or we refuse (verbatim match only, never guess).
    let cc = default_cc.and_then(country_code)?;
    // Drop ONE leading trunk `0` if present, then prepend `+<cc>`.
    let national = digits.strip_prefix('0').unwrap_or(digits);
    finish(&format!("{cc}{national}"))
}

/// Read a bare digit string (no `+`, no `00`) as an already-international
/// number: `"306912345678"` ⇒ `"+306912345678"`. This is the form WhatsApp
/// JIDs and some Viber and address-book values use - they carry the country
/// code but no `+`. `normalize` would treat such a value as national and
/// prepend the default country code a second time, so callers use this as a
/// separate, lower-priority candidate. `None` for anything else.
pub fn bare_international(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '.' | '(' | ')'))
        .collect();
    if cleaned.starts_with("00") || !cleaned.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    finish(&cleaned)
}

/// The national (subscriber) form of an E.164 number for a given default
/// country code: the digits after `+<cc>`, with no trunk `0` re-added. Returns
/// `None` if `e164` does not start with `+<cc>` or no valid `default_cc` is
/// given. Example: `"+306912345678"` with cc `"30"` ⇒ `"6912345678"`.
pub fn national_form(e164: &str, default_cc: Option<&str>) -> Option<String> {
    let cc = default_cc.and_then(country_code)?;
    let rest = e164.strip_prefix(&format!("+{cc}"))?;
    Some(rest.to_string())
}

/// A valid default country code: 1-3 ASCII digits, an optional leading `+`
/// stripped, surrounding whitespace ignored. Blank or anything else ⇒ `None`.
fn country_code(cc: &str) -> Option<&str> {
    let trimmed = cc.trim();
    let digits = trimmed.strip_prefix('+').unwrap_or(trimmed);
    if (1..=3).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit()) {
        Some(digits)
    } else {
        None
    }
}

/// Wrap a final international digit string (no `+`) in E.164 form, enforcing
/// the 8..=15 digit length. This is what keeps short codes out: a 5-digit
/// short code plus a 2-digit cc is only 7 digits, so it is rejected. No
/// country code starts with `0`, so a leading `0` here (e.g. `+0030…`) is
/// malformed and rejected too.
fn finish(digits: &str) -> Option<String> {
    if (8..=15).contains(&digits.len()) && !digits.starts_with('0') {
        Some(format!("+{digits}"))
    } else {
        None
    }
}

#[cfg(test)]
#[path = "phone_tests.rs"]
mod tests;
