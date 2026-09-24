// HalluScribe - tests for the AddressBook contact index. All fixtures are
// synthetic: no real names, numbers or emails.

use super::{ContactBook, ContactId};
use rusqlite::Connection;

const CC_GR: Option<&str> = Some("30");

/// An in-memory address book. `people` is a list of (First, Last,
/// Organization); `phones` and `emails` are (person_index, value).
fn book(
    people: &[(&str, &str, &str)],
    phones: &[(usize, &str)],
    emails: &[(usize, &str)],
) -> ContactBook {
    let conn = Connection::open_in_memory().expect("in-memory db");
    conn.execute_batch(
        "CREATE TABLE ABPerson (ROWID INTEGER PRIMARY KEY, First TEXT, Last TEXT, Organization TEXT);
         CREATE TABLE ABMultiValue (record_id INTEGER, property INTEGER, value TEXT);",
    )
    .expect("create tables");
    for (i, (first, last, org)) in people.iter().enumerate() {
        let rowid: i64 = (i + 1) as i64;
        conn.execute(
            "INSERT INTO ABPerson (ROWID, First, Last, Organization) VALUES (?1, ?2, ?3, ?4)",
            (rowid, first, last, org),
        )
        .expect("insert person");
    }
    for (idx, value) in phones {
        let rowid: i64 = (*idx + 1) as i64;
        conn.execute(
            "INSERT INTO ABMultiValue (record_id, property, value) VALUES (?1, 3, ?2)",
            (rowid, *value),
        )
        .expect("insert phone");
    }
    for (idx, value) in emails {
        let rowid: i64 = (*idx + 1) as i64;
        conn.execute(
            "INSERT INTO ABMultiValue (record_id, property, value) VALUES (?1, 4, ?2)",
            (rowid, *value),
        )
        .expect("insert email");
    }
    ContactBook::from_connection(&conn, CC_GR).expect("build book")
}

#[test]
fn name_is_first_last_and_org_is_kept() {
    let b = book(
        &[("Nikos", "Papadopoulos", "ABC Marble")],
        &[(0, "6912345678")],
        &[],
    );
    let c = b.resolve("691 234 5678", CC_GR).expect("resolve");
    assert_eq!(c.name, "Nikos Papadopoulos");
    assert_eq!(c.organization.as_deref(), Some("ABC Marble"));
}

#[test]
fn name_falls_back_to_organization() {
    let b = book(&[("", "", "ABC Marble Ltd")], &[(0, "6912345678")], &[]);
    let c = b.resolve("6912345678", CC_GR).expect("resolve");
    assert_eq!(c.name, "ABC Marble Ltd");
    assert_eq!(c.organization.as_deref(), Some("ABC Marble Ltd"));
}

#[test]
fn nameless_person_is_skipped() {
    // No name and no organization: the person is not in the book, so the
    // handle does not resolve.
    let b = book(&[("", "", "")], &[(0, "6912345678")], &[]);
    assert!(b.resolve("6912345678", CC_GR).is_none());
    assert!(b.people.is_empty());
}

#[test]
fn email_lookup_is_case_insensitive() {
    let b = book(
        &[("Nikos", "Papadopoulos", "")],
        &[],
        &[(0, "Nikos@Example.com")],
    );
    let c = b
        .resolve("nikos@example.com", CC_GR)
        .expect("resolve email");
    assert_eq!(c.name, "Nikos Papadopoulos");
}

#[test]
fn a_number_on_two_contacts_is_unresolved() {
    // The same normalized number claimed by two different people is ambiguous
    // and dropped, not guessed.
    let b = book(
        &[("Alice", "A", ""), ("Bob", "B", "")],
        &[(0, "6912345678"), (1, "6912345678")],
        &[],
    );
    assert!(b.resolve("6912345678", CC_GR).is_none());
    assert!(!b.by_phone.contains_key("+306912345678"));
}

#[test]
fn the_same_contact_twice_is_resolved() {
    // The same person listing the number twice is fine - one distinct claimant.
    let b = book(
        &[("Nikos", "Papadopoulos", "")],
        &[(0, "6912345678"), (0, "+30 691-234-5678")],
        &[],
    );
    let c = b.resolve("691 234 5678", CC_GR).expect("resolve");
    assert_eq!(c.name, "Nikos Papadopoulos");
}

#[test]
fn a_verbatim_short_code_key_resolves() {
    // A short code does not normalize, so it is keyed verbatim (whitespace
    // stripped) and resolves by the same verbatim path.
    let b = book(&[("Short", "Code", "")], &[(0, "54321")], &[]);
    let c = b.resolve("54321", CC_GR).expect("resolve short code");
    assert_eq!(c.name, "Short Code");
    // The verbatim key is stored, not a normalized one.
    assert!(b.by_phone.contains_key("54321"));
}

#[test]
fn a_value_with_no_person_is_ignored() {
    // An ABMultiValue row whose record_id is not a person is dropped, not an
    // error.
    let conn = Connection::open_in_memory().expect("in-memory db");
    conn.execute_batch(
        "CREATE TABLE ABPerson (ROWID INTEGER PRIMARY KEY, First TEXT, Last TEXT, Organization TEXT);
         CREATE TABLE ABMultiValue (record_id INTEGER, property INTEGER, value TEXT);
         INSERT INTO ABPerson (ROWID, First, Last, Organization) VALUES (1, 'Nikos', 'Papadopoulos', '');
         INSERT INTO ABMultiValue (record_id, property, value) VALUES (999, 3, '6912345678');",
    )
    .expect("seed");
    let b = ContactBook::from_connection(&conn, CC_GR).expect("build");
    assert!(b.resolve("6912345678", CC_GR).is_none());
}

#[test]
fn resolve_never_logs_or_panics_on_garbage_handles() {
    let b = book(&[("Nikos", "Papadopoulos", "")], &[(0, "6912345678")], &[]);
    // Garbage handles resolve to None rather than panicking.
    assert!(b.resolve("", CC_GR).is_none());
    assert!(b.resolve("   ", CC_GR).is_none());
    assert!(b.resolve("not a number", CC_GR).is_none());
    assert!(b.resolve("someone@example.com", CC_GR).is_none());
    // The real one still resolves.
    assert!(b.resolve("6912345678", CC_GR).is_some());
}

/// A contact id is an i64 ROWID, as the plan's type alias requires.
#[test]
fn contact_id_is_an_i64_rowid() {
    let b = book(&[("Nikos", "Papadopoulos", "")], &[(0, "6912345678")], &[]);
    let id: ContactId = 1;
    assert!(b.people.contains_key(&id));
}

#[test]
fn a_bare_international_handle_resolves_to_a_normalized_entry() {
    // WhatsApp/Viber give the number with its country code but no `+`.
    // Normalizing it as national would prepend the cc twice (+30306...).
    let b = book(
        &[("Nikos", "Papadopoulos", "")],
        &[(0, "+30 6912345678")],
        &[],
    );
    let (c, key) = b.resolve_phone("306912345678", CC_GR).expect("resolve");
    assert_eq!(c.name, "Nikos Papadopoulos");
    assert_eq!(key, "+306912345678");
    // With no default cc too.
    assert!(b.resolve("306912345678", None).is_some());
}

#[test]
fn an_entry_saved_as_bare_international_resolves_from_e164() {
    // The address book holds `306912345678` (cc, no `+`); an iMessage handle
    // arrives as `+306912345678`. The secondary index bridges the two.
    let b = book(
        &[("Nikos", "Papadopoulos", "")],
        &[(0, "306912345678")],
        &[],
    );
    let (c, key) = b.resolve_phone("+306912345678", CC_GR).expect("resolve");
    assert_eq!(c.name, "Nikos Papadopoulos");
    assert_eq!(key, "+306912345678");
}

#[test]
fn the_secondary_index_never_shadows_a_primary_match() {
    // Alice is saved as the national `6912345678` (primary +306912345678);
    // Bob as the bare `306912345678` (secondary +306912345678). The primary
    // key wins for Alice's number.
    let b = book(
        &[("Alice", "A", ""), ("Bob", "B", "")],
        &[(0, "6912345678"), (1, "306912345678")],
        &[],
    );
    let c = b.resolve("+306912345678", CC_GR).expect("resolve");
    assert_eq!(c.name, "Alice A");
}
