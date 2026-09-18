// HalluScribe - reads named values from an iPhone backup's Info.plist.
//
// Info.plist is XML (device name, iOS version, last backup date, ...). This is
// a small text scanner that returns the value following a given `<key>`,
// taking the FIRST element after the key regardless of whether it is a
// `<string>` or a `<date>`. It deliberately does not add a plist parsing
// crate.

/// Return the value of `<key>key</key>` in a plist, or "n/a" if the key is
/// absent or its value is not an immediate `<string>`/`<date>` element.
///
/// The value is the first element that follows the closing `</key>` (after
/// whitespace), whatever its tag. Taking the first following element - not
/// "the next `<string>` somewhere later" - is what keeps a `<date>` value from
/// being skipped in favour of a later `<string>` (e.g. the Phone Number that
/// follows "Last Backup Date").
pub fn plist_value(xml: &str, key: &str) -> String {
    let marker = format!("<key>{key}</key>");
    let Some(start) = xml.find(&marker) else {
        return "n/a".to_string();
    };
    let after_key = &xml[start + marker.len()..];

    // Skip whitespace; the value element must begin immediately after it.
    let trimmed = after_key.trim_start();
    for tag in ["<string>", "<date>"] {
        if let Some(rest) = trimmed.strip_prefix(tag) {
            // The closing tag is `</string>` / `</date>`: the inner name is the
            // open tag with its leading `<` and trailing `>` removed.
            let inner = &tag[1..tag.len() - 1];
            let close = format!("</{inner}>");
            if let Some(end) = rest.find(&close) {
                return rest[..end].trim().to_string();
            }
        }
    }
    "n/a".to_string()
}

#[cfg(test)]
#[path = "info_plist_tests.rs"]
mod tests;
