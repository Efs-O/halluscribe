// HalluScribe - tests for the typedstream attributedBody decoder.
//
// Every fixture is a synthetic blob built byte by byte here - no real backup
// data, names, numbers or messages.

use super::decode_attributed_body;

/// The typedstream header.
fn header() -> Vec<u8> {
    vec![
        0x04, 0x0B, b's', b't', b'r', b'e', b'a', b'm', b't', b'y', b'p', b'e', b'd',
    ]
}

/// Encode a length the way a typedstream does: 1 byte (< 0x80), `0x81` + u16
/// LE, or `0x82` + u32 LE.
fn length_bytes(len: usize) -> Vec<u8> {
    if len < 0x80 {
        vec![len as u8]
    } else if len <= 0xFFFF {
        vec![0x81, (len & 0xFF) as u8, ((len >> 8) & 0xFF) as u8]
    } else {
        vec![
            0x82,
            (len & 0xFF) as u8,
            ((len >> 8) & 0xFF) as u8,
            ((len >> 16) & 0xFF) as u8,
            ((len >> 24) & 0xFF) as u8,
        ]
    }
}

/// A well-formed blob: header + `marker` + 0x2B + length + `text`.
fn build(marker: &[u8], text: &[u8]) -> Vec<u8> {
    let mut b = header();
    b.extend_from_slice(marker);
    b.push(0x2B);
    b.extend_from_slice(&length_bytes(text.len()));
    b.extend_from_slice(text);
    b
}

const NS: &[u8] = b"NSString";
const NS_MUT: &[u8] = b"NSMutableString";

#[test]
fn decodes_ascii_with_one_byte_length() {
    let blob = build(NS, b"Hello, world");
    assert_eq!(
        decode_attributed_body(&blob).as_deref(),
        Some("Hello, world")
    );
}

#[test]
fn decodes_greek_multibyte_utf8() {
    let text = "Καλημέρα".as_bytes();
    let blob = build(NS, text);
    assert_eq!(decode_attributed_body(&blob).as_deref(), Some("Καλημέρα"));
}

#[test]
fn decodes_emoji_multibyte_utf8() {
    let text = "Hello 😀 world 🚀".as_bytes();
    let blob = build(NS, text);
    assert_eq!(
        decode_attributed_body(&blob).as_deref(),
        Some("Hello 😀 world 🚀")
    );
}

#[test]
fn decodes_u16_length_over_127_bytes() {
    // 200 bytes of ASCII, so the length uses the 0x81 + u16 form.
    let text: Vec<u8> = (0..200u32).map(|i| b'a' + (i % 26) as u8).collect();
    let expected = String::from_utf8(text).unwrap();
    let blob = build(NS, expected.as_bytes());
    assert_eq!(
        decode_attributed_body(&blob).as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn decodes_u32_length_over_65535_bytes() {
    // 70_000 bytes, so the length uses the 0x82 + u32 form.
    let text: Vec<u8> = (0..70_000u32).map(|i| b'a' + (i % 26) as u8).collect();
    let blob = build(NS, &text);
    let expected = String::from_utf8(text.clone()).unwrap();
    assert_eq!(
        decode_attributed_body(&blob).as_deref(),
        Some(expected.as_str())
    );
}

#[test]
fn decodes_ns_mutable_string_marker() {
    let blob = build(NS_MUT, b"mutable text");
    assert_eq!(
        decode_attributed_body(&blob).as_deref(),
        Some("mutable text")
    );
}

#[test]
fn truncated_blob_length_past_end_is_none() {
    // Declares 10 bytes but the payload is only 4.
    let mut b = header();
    b.extend_from_slice(NS);
    b.push(0x2B);
    b.push(10);
    b.extend_from_slice(b"ab");
    assert_eq!(decode_attributed_body(&b), None);
}

#[test]
fn wrong_header_is_none() {
    let mut b = vec![
        0x04, 0x0C, b's', b't', b'r', b'e', b'a', b'm', b't', b'y', b'p', b'e', b'd',
    ];
    b.extend_from_slice(NS);
    b.push(0x2B);
    b.push(2);
    b.extend_from_slice(b"ab");
    assert_eq!(decode_attributed_body(&b), None);
}

#[test]
fn missing_nsstring_marker_is_none() {
    let mut b = header();
    b.extend_from_slice(b"NSObject");
    b.push(0x2B);
    b.push(2);
    b.extend_from_slice(b"ab");
    assert_eq!(decode_attributed_body(&b), None);
}

#[test]
fn no_string_tag_after_marker_is_none() {
    let mut b = header();
    b.extend_from_slice(NS);
    b.extend_from_slice(b"no tag here");
    assert_eq!(decode_attributed_body(&b), None);
}

#[test]
fn invalid_utf8_payload_is_none() {
    let mut b = header();
    b.extend_from_slice(NS);
    b.push(0x2B);
    b.push(2);
    b.extend_from_slice(&[0xFF, 0xFE]);
    assert_eq!(decode_attributed_body(&b), None);
}

#[test]
fn bad_length_lead_byte_is_none() {
    let mut b = header();
    b.extend_from_slice(NS);
    b.push(0x2B);
    b.push(0x83); // not 0x81 or 0x82, and >= 0x80
    b.extend_from_slice(b"ab");
    assert_eq!(decode_attributed_body(&b), None);
}

#[test]
fn empty_blob_is_none() {
    assert_eq!(decode_attributed_body(&[]), None);
}

#[test]
fn empty_string_payload_is_some_empty() {
    let blob = build(NS, b"");
    assert_eq!(decode_attributed_body(&blob).as_deref(), Some(""));
}
