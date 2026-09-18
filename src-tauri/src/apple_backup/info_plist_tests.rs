// HalluScribe - synthetic tests for the Info.plist value scanner.

use super::plist_value;

const PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
	<key>ProductVersion</key>
	<string>18.4.1</string>
	<key>Last Backup Date</key>
	<date>2026-01-01T00:00:00Z</date>
	<key>Phone Number</key>
	<string>+000</string>
	<key>Serial Number</key>
	<string>ABC123</string>
</dict>
</plist>"#;

#[test]
fn reads_a_string_value() {
    assert_eq!(plist_value(PLIST, "ProductVersion"), "18.4.1");
}

#[test]
fn reads_a_date_value_and_does_not_skip_to_the_next_string() {
    // The regression the supervisor flagged: "Last Backup Date" is a <date>,
    // and the next <string> in the file is the Phone Number. The scanner must
    // return the date, not the phone number.
    assert_eq!(
        plist_value(PLIST, "Last Backup Date"),
        "2026-01-01T00:00:00Z"
    );
}

#[test]
fn a_string_value_after_a_date_is_its_own_key() {
    assert_eq!(plist_value(PLIST, "Phone Number"), "+000");
}

#[test]
fn missing_key_is_n_a() {
    assert_eq!(plist_value(PLIST, "Model Number"), "n/a");
}

#[test]
fn a_non_string_non_date_value_is_n_a() {
    let xml = "<dict><key>Count</key><integer>5</integer></dict>";
    assert_eq!(plist_value(xml, "Count"), "n/a");
}

#[test]
fn value_on_the_same_line_as_the_key() {
    let xml = "<dict><key>ProductVersion</key><string>17.0</string></dict>";
    assert_eq!(plist_value(xml, "ProductVersion"), "17.0");
}
