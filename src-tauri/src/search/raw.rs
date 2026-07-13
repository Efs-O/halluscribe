// HalluScribe - raw transcript pure matcher: dual-needle scan + excerpts.

const EXCERPT_CHARS: usize = 200;

#[derive(Debug, PartialEq, Eq)]
pub struct RawExcerpt {
    pub line_no: usize,
    pub excerpt: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RawScanOutcome {
    pub total_hits: usize,
    pub excerpts: Vec<RawExcerpt>,
    pub excerpts_truncated: bool,
}

/// Return the representation stored inside a JSON string value, without the
/// serializer's surrounding quotes.
pub(crate) fn json_escaped(needle: &str) -> String {
    let serialized = serde_json::to_string(needle)
        .expect("serializing a Rust string as a JSON string is infallible");
    serialized[1..serialized.len() - 1].to_owned()
}

/// Scan one decompressed transcript while retaining at most one excerpt per
/// matching line. Matching offsets belong to the lowercased line; excerpts
/// deliberately use that same copy because Unicode lowercasing can change
/// byte lengths relative to the source.
pub(crate) fn scan_text(text: &str, needle: &str, max_excerpts: usize) -> RawScanOutcome {
    if needle.trim().is_empty() {
        return RawScanOutcome {
            total_hits: 0,
            excerpts: Vec::new(),
            excerpts_truncated: false,
        };
    }

    let literal = needle.to_lowercase();
    let escaped = json_escaped(needle).to_lowercase();
    let second_needle = (escaped != literal).then_some(escaped.as_str());
    let literal_chars = literal.chars().count();
    let escaped_chars = escaped.chars().count();
    let mut outcome = RawScanOutcome {
        total_hits: 0,
        excerpts: Vec::new(),
        excerpts_truncated: false,
    };

    for (line_index, line) in text.lines().enumerate() {
        let lowered = line.to_lowercase();
        let mut line_hits = 0;
        let mut first_hit = None;

        // Starting only at char boundaries makes overlapping matching safe for
        // UTF-8 while the OR merges dual-needle hits at an identical position.
        for (byte_index, _) in lowered.char_indices() {
            let tail = &lowered[byte_index..];
            let literal_match = tail.starts_with(&literal);
            let escaped_match = second_needle.is_some_and(|candidate| tail.starts_with(candidate));
            if literal_match || escaped_match {
                line_hits += 1;
                let matched_chars = if literal_match {
                    literal_chars
                } else {
                    escaped_chars
                };
                first_hit.get_or_insert((byte_index, matched_chars));
            }
        }

        outcome.total_hits += line_hits;
        let Some((hit_byte, hit_chars)) = first_hit else {
            continue;
        };

        if outcome.excerpts.len() < max_excerpts {
            outcome.excerpts.push(RawExcerpt {
                line_no: line_index + 1,
                excerpt: excerpt_around(&lowered, hit_byte, hit_chars),
            });
        } else {
            outcome.excerpts_truncated = true;
        }
    }

    outcome
}

fn excerpt_around(line: &str, hit_byte: usize, hit_chars: usize) -> String {
    let total_chars = line.chars().count();
    if total_chars <= EXCERPT_CHARS {
        return line.to_owned();
    }

    let hit_start = line[..hit_byte].chars().count();
    let hit_center = hit_start + hit_chars / 2;
    let mut start = hit_center.saturating_sub(EXCERPT_CHARS / 2);
    start = start.min(total_chars - EXCERPT_CHARS);
    line.chars().skip(start).take(EXCERPT_CHARS).collect()
}
