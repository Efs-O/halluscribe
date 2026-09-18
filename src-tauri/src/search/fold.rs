// HalluScribe - search folding: case + Greek accents (D9).
//
// One explicit fold table used by BOTH raw search and session search. It
// lowercases, then strips Greek tonos/dialytika and normalises the final sigma
// so that `καλημερα` finds `Καλημέρα` and `ΟΔΟΣ` finds `οδός`. No new crate,
// no Unicode normalisation (NFD) library: a single char-by-char table.
//
// Offset safety: every table entry maps one char to one char of the SAME UTF-8
// byte length (all of these are 2 bytes → 2 bytes), so `fold_for_search(x).len()`
// equals `x.to_lowercase().len()`. Raw search takes match offsets from the
// folded line and cuts excerpts with them; that stays valid only because the
// fold never changes char count. Greek↔Latin transliteration is deliberately
// NOT folded (D9): the agent searches both spellings.

/// Fold a string for search matching: lowercase, then strip Greek accents and
/// normalise the final sigma. See the module docs for the byte-length
/// guarantee that keeps raw-search offsets valid.
pub fn fold_for_search(s: &str) -> String {
    s.to_lowercase().chars().map(fold_char).collect()
}

/// The one-char fold. `to_lowercase` already turns the uppercase accented
/// forms (Ά, Έ, …) into their lowercase accented forms, so this table only
/// needs the lowercase accented/dialytika/final-sigma set. Codepoints are
/// written as explicit escapes so the table is unambiguous — in particular the
/// final-sigma entry maps ς (U+03C2) to the MEDIAL sigma σ (U+03C3), which is
/// what lets an uppercase query (`ΟΔΟΣ` → `οδος`) match a final-sigma word
/// (`οδός`).
fn fold_char(c: char) -> char {
    match c {
        '\u{03AC}' => '\u{03B1}', // ά -> α
        '\u{03AD}' => '\u{03B5}', // έ -> ε
        '\u{03AE}' => '\u{03B7}', // ή -> η
        '\u{03AF}' => '\u{03B9}', // ί -> ι
        '\u{03CC}' => '\u{03BF}', // ό -> ο
        '\u{03CD}' => '\u{03C5}', // ύ -> υ
        '\u{03CE}' => '\u{03C9}', // ώ -> ω
        '\u{0390}' => '\u{03B9}', // ΐ -> ι
        '\u{03B0}' => '\u{03C5}', // ΰ -> υ
        '\u{03CA}' => '\u{03B9}', // ϊ -> ι
        '\u{03CB}' => '\u{03C5}', // ϋ -> υ
        '\u{03C2}' => '\u{03C3}', // ς (final sigma) -> σ (medial sigma)
        other => other,
    }
}

#[cfg(test)]
#[path = "fold_tests.rs"]
mod tests;
