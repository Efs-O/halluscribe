// HalluScribe - unit tests for the hand-rolled secret-pattern scanner.

use super::*;

#[test]
fn empty_string_yields_empty_vec() {
    assert!(scan_for_secrets("").is_empty());
}

#[test]
fn clean_text_yields_empty_vec() {
    assert!(scan_for_secrets("just a normal sentence about Rust and Tauri").is_empty());
}

// -- aws_access_key --

#[test]
fn aws_key_valid_flags() {
    let flags = scan_for_secrets("key is AKIAIOSFODNN7EXAMPLE in the config");
    assert_eq!(flags, vec!["aws_access_key".to_string()]);
}

#[test]
fn aws_key_embedded_in_longer_word_does_not_flag() {
    assert!(scan_for_secrets("XAKIAIOSFODNN7EXAMPLEX").is_empty());
}

#[test]
fn aws_key_short_tail_does_not_flag() {
    assert!(scan_for_secrets("AKIASHORT").is_empty());
}

// -- openai_key --

#[test]
fn openai_key_valid_length_flags() {
    let flags = scan_for_secrets("token: sk-abcdefghijklmnopqrstuvwxyz1234");
    assert_eq!(flags, vec!["openai_key".to_string()]);
}

#[test]
fn openai_key_too_short_does_not_flag() {
    assert!(scan_for_secrets("token sk-1234").is_empty());
}

#[test]
fn openai_key_preceded_by_alnum_does_not_flag() {
    assert!(scan_for_secrets("task-abcdefghijklmnopqrstuvwxyz1234").is_empty());
}

// -- github_token --

#[test]
fn github_ghp_token_flags() {
    let flags = scan_for_secrets("ghp_abcdefghijklmnopqrstuvwxyz1234567890");
    assert_eq!(flags, vec!["github_token".to_string()]);
}

#[test]
fn github_pat_token_flags() {
    let flags = scan_for_secrets("github_pat_abcdefghijklmnopqrstuvwxyz1234567890");
    assert_eq!(flags, vec!["github_token".to_string()]);
}

#[test]
fn github_token_too_short_does_not_flag() {
    assert!(scan_for_secrets("ghp_shorttoken").is_empty());
}

// -- slack_token --

#[test]
fn slack_xoxb_token_flags() {
    let flags = scan_for_secrets("xoxb-1234567890-abcdefghij");
    assert_eq!(flags, vec!["slack_token".to_string()]);
}

#[test]
fn slack_token_too_short_does_not_flag() {
    assert!(scan_for_secrets("xoxb-123").is_empty());
}

// -- private_key --

#[test]
fn rsa_private_key_flags() {
    let flags =
        scan_for_secrets("-----BEGIN RSA PRIVATE KEY-----\nMIIB...\n-----END RSA PRIVATE KEY-----");
    assert_eq!(flags, vec!["private_key".to_string()]);
}

#[test]
fn openssh_private_key_flags() {
    let flags = scan_for_secrets(
        "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----",
    );
    assert_eq!(flags, vec!["private_key".to_string()]);
}

#[test]
fn no_space_private_key_form_flags() {
    let flags = scan_for_secrets("-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----");
    assert_eq!(flags, vec!["private_key".to_string()]);
}

#[test]
fn plain_certificate_does_not_flag() {
    assert!(
        scan_for_secrets("-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----").is_empty()
    );
}

// -- credential_assignment --

#[test]
fn password_assignment_flags() {
    let flags = scan_for_secrets("password: hunter42");
    assert_eq!(flags, vec!["credential_assignment".to_string()]);
}

#[test]
fn password_redacted_placeholder_does_not_flag() {
    assert!(scan_for_secrets("password = [REDACTED]").is_empty());
}

#[test]
fn api_key_angle_bracket_placeholder_does_not_flag() {
    assert!(scan_for_secrets("api_key: <YOUR_KEY>").is_empty());
}

#[test]
fn secret_env_placeholder_does_not_flag() {
    assert!(scan_for_secrets("secret: ${VAR}").is_empty());
}

#[test]
fn password_none_does_not_flag() {
    assert!(scan_for_secrets("password: none").is_empty());
}

// -- url_credentials --

#[test]
fn ftp_url_with_credentials_flags() {
    let flags = scan_for_secrets("ftp://bob:hunter42@host/x");
    assert_eq!(flags, vec!["url_credentials".to_string()]);
}

#[test]
fn https_url_without_credentials_does_not_flag() {
    assert!(scan_for_secrets("https://example.com/path").is_empty());
}

#[test]
fn ftp_url_with_redacted_password_does_not_flag() {
    assert!(scan_for_secrets("ftp://bob:[REDACTED]@host").is_empty());
}

// -- combined --

#[test]
fn multiple_secrets_are_sorted_and_deduped() {
    let text = "AKIAIOSFODNN7EXAMPLE and again AKIAIOSFODNN7EXAMPLE, also sk-abcdefghijklmnopqrstuvwxyz1234";
    let flags = scan_for_secrets(text);
    assert_eq!(
        flags,
        vec!["aws_access_key".to_string(), "openai_key".to_string()]
    );
}
