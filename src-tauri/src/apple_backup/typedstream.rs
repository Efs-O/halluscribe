// HalluScribe - decodes an NSArchiver "typedstream" attributedBody blob.
//
// iOS stores some SMS message text in the `message.attributedBody` column as an
// NSKeyedArchiver typedstream blob rather than a plain `text` string. This
// module extracts the first NSString from such a blob. It is pure: it never
// panics on any input (every byte access is bounds-checked via `get(..)`) and
// returns `None` for anything that is not a well-formed attributedBody.
//
// Phase 3 of the Business Messaging ingestion plan.

/// The typedstream header: `0x04 0x0B` + ASCII `streamtyped`.
const HEADER: &[u8] = &[
    0x04, 0x0B, b's', b't', b'r', b'e', b'a', b'm', b't', b'y', b'p', b'e', b'd',
];

/// The string type tag in a typedstream: `'+'` (0x2B).
const STRING_TAG: u8 = 0x2B;

/// The ASCII class-name markers that precede the string payload. Both
/// `NSString` and `NSMutableString` appear in real data; the marker is the
/// earliest occurrence of either.
const MARKERS: [&[u8]; 2] = [b"NSString", b"NSMutableString"];

/// Decode the first NSString out of an NSArchiver typedstream `attributedBody`
/// blob. Returns `None` for any input that is not a well-formed blob; an empty
/// decoded string is `Some("")`.
pub fn decode_attributed_body(blob: &[u8]) -> Option<String> {
    // 1. typedstream header.
    if !blob.starts_with(HEADER) {
        return None;
    }

    // 2. first class marker (NSString or NSMutableString).
    let (marker_start, marker_len) = find_marker(blob)?;
    let after_marker = blob.get(marker_start + marker_len..)?;

    // 3. first string tag after the marker.
    let tag = after_marker.iter().position(|&b| b == STRING_TAG)?;
    let tag_pos = marker_start + marker_len + tag;

    // 4. read the length (1, 2 or 4 bytes little-endian).
    let (length, consumed) = read_length(blob, tag_pos + 1)?;

    // 5. take exactly `length` bytes as UTF-8.
    let start = tag_pos + 1 + consumed;
    let payload = blob.get(start..start.checked_add(length)?)?;
    String::from_utf8(payload.to_vec()).ok()
}

/// The `(start, length)` of the earliest occurrence of any class marker in
/// `blob`, or `None` if none is present.
fn find_marker(blob: &[u8]) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for marker in MARKERS {
        if let Some(pos) = find_subseq(blob, marker) {
            match best {
                None => best = Some((pos, marker.len())),
                Some((bpos, _)) if pos < bpos => best = Some((pos, marker.len())),
                _ => {}
            }
        }
    }
    best
}

/// Index of the first occurrence of `needle` in `haystack`, or `None`.
fn find_subseq(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Read a typedstream length field at `pos`. Returns `(length, bytes_consumed)`
/// where `bytes_consumed` covers the lead byte plus any length bytes, or `None`
/// for an unknown lead byte or a length field that runs past the end of the
/// blob.
fn read_length(blob: &[u8], pos: usize) -> Option<(usize, usize)> {
    let lead = *blob.get(pos)?;
    if lead < 0x80 {
        Some((lead as usize, 1))
    } else if lead == 0x81 {
        let lo = *blob.get(pos + 1)?;
        let hi = *blob.get(pos + 2)?;
        Some((lo as usize | ((hi as usize) << 8), 3))
    } else if lead == 0x82 {
        let b0 = *blob.get(pos + 1)?;
        let b1 = *blob.get(pos + 2)?;
        let b2 = *blob.get(pos + 3)?;
        let b3 = *blob.get(pos + 4)?;
        let len =
            (b0 as usize) | ((b1 as usize) << 8) | ((b2 as usize) << 16) | ((b3 as usize) << 24);
        Some((len, 5))
    } else {
        None
    }
}

#[cfg(test)]
#[path = "typedstream_tests.rs"]
mod tests;
