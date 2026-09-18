// HalluScribe - tests for the search fold (D9). Synthetic Greek/Latin/emoji
// strings only; no real data.

use super::fold_for_search;

/// Every accented vowel, dialytika pair, and final sigma, both ways: the
/// accented form folds to the plain one, and the plain form is unchanged.
/// Codepoints are explicit so the final-sigma entry (ς U+03C2 -> σ U+03C3)
/// is unambiguous.
#[test]
fn greek_accent_matrix_folds_both_ways() {
    let pairs = [
        ('\u{03AC}', '\u{03B1}'), // ά -> α
        ('\u{03AD}', '\u{03B5}'), // έ -> ε
        ('\u{03AE}', '\u{03B7}'), // ή -> η
        ('\u{03AF}', '\u{03B9}'), // ί -> ι
        ('\u{03CC}', '\u{03BF}'), // ό -> ο
        ('\u{03CD}', '\u{03C5}'), // ύ -> υ
        ('\u{03CE}', '\u{03C9}'), // ώ -> ω
        ('\u{0390}', '\u{03B9}'), // ΐ -> ι
        ('\u{03B0}', '\u{03C5}'), // ΰ -> υ
        ('\u{03CA}', '\u{03B9}'), // ϊ -> ι
        ('\u{03CB}', '\u{03C5}'), // ϋ -> υ
        ('\u{03C2}', '\u{03C3}'), // ς (final sigma) -> σ (medial sigma)
    ];
    for (accented, plain) in pairs {
        assert_eq!(
            fold_for_search(&accented.to_string()),
            plain.to_string(),
            "accented {accented}"
        );
        // The plain form is a fixed point.
        assert_eq!(
            fold_for_search(&plain.to_string()),
            plain.to_string(),
            "plain {plain}"
        );
    }
}

/// Uppercase accented forms fold to the plain lowercase letter (to_lowercase
/// first, then the table).
#[test]
fn uppercase_accented_forms_fold_to_plain_lowercase() {
    let pairs = [
        ('Ά', 'α'),
        ('Έ', 'ε'),
        ('Ή', 'η'),
        ('Ί', 'ι'),
        ('Ό', 'ο'),
        ('Ύ', 'υ'),
        ('Ώ', 'ω'),
        ('Ϊ', 'ι'),
        ('Ϋ', 'υ'),
        ('Ι', 'ι'),
        ('Υ', 'υ'),
        ('Σ', 'σ'),
    ];
    for (upper, plain) in pairs {
        assert_eq!(
            fold_for_search(&upper.to_string()),
            plain.to_string(),
            "uppercase {upper}"
        );
    }
}

/// The D9 headline cases, both directions. The sigma words are built with
/// explicit codepoints: a final-sigma word (ς U+03C2) and an uppercase word
/// (Σ U+03A3) must both fold to the same medial-sigma form (σ U+03C3).
#[test]
fn d9_headline_cases() {
    // Accented text, plain query.
    assert_eq!(
        fold_for_search("\u{039A}\u{03B1}\u{03BB}\u{03B7}\u{03BC}\u{03AD}\u{03C1}\u{03B1}"),
        "\u{03BA}\u{03B1}\u{03BB}\u{03B7}\u{03BC}\u{03B5}\u{03C1}\u{03B1}"
    ); // Καλημέρα -> καλημερα
       // A final-sigma word folds to the medial-sigma form.
    assert_eq!(
        fold_for_search("\u{03BF}\u{03B4}\u{03BF}\u{03C2}"),
        "\u{03BF}\u{03B4}\u{03BF}\u{03C3}"
    ); // οδός -> οδος
       // An uppercase word folds to the same medial-sigma form.
    assert_eq!(
        fold_for_search("\u{039F}\u{0394}\u{039F}\u{03A3}"),
        "\u{03BF}\u{03B4}\u{03BF}\u{03C3}"
    ); // ΟΔΟΣ -> οδος
       // Plain text stays put, so a plain query hits it.
    assert_eq!(
        fold_for_search("\u{03BA}\u{03B1}\u{03BB}\u{03B7}\u{03BC}\u{03B5}\u{03C1}\u{03B1}"),
        "\u{03BA}\u{03B1}\u{03BB}\u{03B7}\u{03BC}\u{03B5}\u{03C1}\u{03B1}"
    );
    assert_eq!(
        fold_for_search("\u{03BF}\u{03B4}\u{03BF}\u{03C3}"),
        "\u{03BF}\u{03B4}\u{03BF}\u{03C3}"
    );
}

/// Latin/ASCII is unchanged by the fold (lowercased, as before).
#[test]
fn latin_ascii_unchanged() {
    assert_eq!(fold_for_search("Hello World"), "hello world");
    assert_eq!(fold_for_search("mcpBridge"), "mcpbridge");
    assert_eq!(fold_for_search("ABC123"), "abc123");
}

/// Emoji and CJK pass through (lowercased, which is a no-op for them).
#[test]
fn emoji_and_cjk_pass_through() {
    assert_eq!(fold_for_search("😀 hello"), "😀 hello");
    assert_eq!(fold_for_search("日本語 テスト"), "日本語 テスト");
    assert_eq!(fold_for_search("café ☕"), "café ☕");
}

/// The byte-length guarantee: folding never changes char count, so raw-search
/// offsets taken from the folded line stay valid. Checked on a mixed
/// Greek/Latin/emoji string.
#[test]
fn fold_preserves_char_count_vs_lowercase() {
    let samples = [
        "Καλημέρα καλημερα",
        "ΟΔΟΣ οδός οδος",
        "ΚΑΛΑΚΆΤΑ καλακατα",
        "Hello 😀 日本語 café άέήίόύώϊϋς",
        "mixed ΆΐΰςΣσ αβγδε",
    ];
    for s in samples {
        assert_eq!(
            fold_for_search(s).len(),
            s.to_lowercase().len(),
            "byte length changed for {s:?}"
        );
        assert_eq!(
            fold_for_search(s).chars().count(),
            s.to_lowercase().chars().count(),
            "char count changed for {s:?}"
        );
    }
}
