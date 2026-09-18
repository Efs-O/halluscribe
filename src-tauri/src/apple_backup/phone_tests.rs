// HalluScribe - tests for phone normalization (D8: verbatim match only, no
// guessing). All inputs are synthetic.

use super::{national_form, normalize};

const CC_GR: Option<&str> = Some("30");
const CC_UK: Option<&str> = Some("44");
const NO_CC: Option<&str> = None;

#[test]
fn greek_mobile_and_landline_every_spelling_with_cc() {
    // Greek mobile in every spelling, cc 30.
    assert_eq!(
        normalize("6912345678", CC_GR).as_deref(),
        Some("+306912345678")
    );
    assert_eq!(
        normalize("691 234 5678", CC_GR).as_deref(),
        Some("+306912345678")
    );
    assert_eq!(
        normalize("+30 691-234-5678", CC_GR).as_deref(),
        Some("+306912345678")
    );
    assert_eq!(
        normalize("0030 6912345678", CC_GR).as_deref(),
        Some("+306912345678")
    );
    // Greek landline (Athens area code 210), cc 30.
    assert_eq!(
        normalize("(210) 1234567", CC_GR).as_deref(),
        Some("+302101234567")
    );
}

#[test]
fn greek_inputs_with_blank_cc() {
    // National numbers with no default cc are NOT normalized (verbatim match
    // only, D8). Already-international ones still are.
    assert_eq!(normalize("6912345678", NO_CC), None);
    assert_eq!(normalize("691 234 5678", NO_CC), None);
    assert_eq!(normalize("(210) 1234567", NO_CC), None);
    assert_eq!(
        normalize("+30 691-234-5678", NO_CC).as_deref(),
        Some("+306912345678")
    );
    assert_eq!(
        normalize("0030 6912345678", NO_CC).as_deref(),
        Some("+306912345678")
    );
}

#[test]
fn uk_mobile_with_cc_drops_trunk_zero() {
    assert_eq!(
        normalize("07911 123456", CC_UK).as_deref(),
        Some("+447911123456")
    );
}

#[test]
fn short_codes_are_never_normalized() {
    // 5-digit and 4-digit short codes: even with a cc, the international digit
    // count is below 8, so they are rejected.
    assert_eq!(normalize("54321", CC_GR), None);
    assert_eq!(normalize("1234", CC_GR), None);
}

#[test]
fn emails_and_alphanumeric_sender_ids_are_rejected() {
    assert_eq!(normalize("someone@example.com", CC_GR), None);
    assert_eq!(normalize("ABC123", CC_GR), None);
    assert_eq!(normalize("HELP-DESK", CC_GR), None);
}

#[test]
fn a_bad_default_cc_is_none() {
    // 4-digit and non-numeric country codes are invalid; a national number with
    // one is not guessed.
    assert_eq!(normalize("6912345678", Some("3000")), None);
    assert_eq!(normalize("6912345678", Some("30x")), None);
    assert_eq!(normalize("6912345678", Some("")), None);
}

#[test]
fn a_leading_plus_on_the_cc_is_stripped() {
    assert_eq!(
        normalize("6912345678", Some("+30")).as_deref(),
        Some("+306912345678")
    );
}

#[test]
fn national_form_round_trip() {
    let e164 = normalize("6912345678", CC_GR).unwrap();
    assert_eq!(e164, "+306912345678");
    assert_eq!(national_form(&e164, CC_GR).as_deref(), Some("6912345678"));
}

#[test]
fn national_form_none_when_cc_does_not_match() {
    // The number is Greek but the cc is UK, so it does not start with +44.
    assert_eq!(national_form("+306912345678", CC_UK), None);
    // No cc at all.
    assert_eq!(national_form("+306912345678", NO_CC), None);
}
