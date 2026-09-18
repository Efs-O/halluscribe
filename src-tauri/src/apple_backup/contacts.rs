// HalluScribe - reads an iPhone AddressBook into a phone/email -> person index.
//
// The caller opens the `AddressBook.sqlitedb` read-only (Phase 1) and passes
// the connection in; this module never opens files. It builds a `ContactBook`
// keyed by normalized E.164 phone and lowercased email so a raw SMS handle can
// be resolved to a person's name. Ambiguous keys (claimed by two different
// people) are dropped, never guessed (D7/D8). No names, numbers or emails are
// logged.

use crate::apple_backup::phone::normalize;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

/// A person's stable identifier: their `ABPerson.ROWID`.
pub type ContactId = i64;

/// One person from the address book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub id: ContactId,
    /// "First Last" (trimmed); falls back to the organization if both name
    /// parts are empty.
    pub name: String,
    /// The organization, when present.
    pub organization: Option<String>,
}

/// An address book indexed for handle resolution.
#[derive(Debug, Default)]
pub struct ContactBook {
    /// Normalized E.164 phone (or verbatim, when unnormalizable) -> person.
    pub by_phone: HashMap<String, ContactId>,
    /// Lowercased email -> person.
    pub by_email: HashMap<String, ContactId>,
    /// Person id -> person.
    pub people: HashMap<ContactId, Contact>,
}

impl ContactBook {
    /// Read the address book from an open connection. `default_cc` is the
    /// user's default country code for phone normalization (D8).
    pub fn from_connection(conn: &Connection, default_cc: Option<&str>) -> rusqlite::Result<Self> {
        let people = read_people(conn)?;

        // Each key -> the set of distinct people that claim it. A key claimed
        // by two different people is ambiguous and is dropped, not guessed.
        let mut phone_claims: HashMap<String, HashSet<ContactId>> = HashMap::new();
        let mut email_claims: HashMap<String, HashSet<ContactId>> = HashMap::new();
        collect_values(
            conn,
            &people,
            default_cc,
            &mut phone_claims,
            &mut email_claims,
        )?;

        Ok(ContactBook {
            by_phone: resolve_claims(&phone_claims),
            by_email: resolve_claims(&email_claims),
            people,
        })
    }

    /// Resolve a raw SMS handle to a person. A handle containing `@` is looked
    /// up as an email (trim + lowercase); otherwise it is normalized to E.164
    /// and, if that fails, matched verbatim (whitespace stripped) - exactly as
    /// when the book was built. Returns `None` for an unresolved handle.
    pub fn resolve(&self, handle: &str, default_cc: Option<&str>) -> Option<&Contact> {
        let handle = handle.trim();
        if handle.contains('@') {
            let key = handle.to_lowercase();
            return self.by_email.get(&key).and_then(|id| self.people.get(id));
        }
        let verbatim: String = handle.chars().filter(|c| !c.is_whitespace()).collect();
        let id = normalize(handle, default_cc)
            .as_ref()
            .and_then(|n| self.by_phone.get(n))
            .or_else(|| self.by_phone.get(&verbatim))?;
        self.people.get(id)
    }
}

/// Read every `ABPerson` row into a person map, dropping nameless people.
fn read_people(conn: &Connection) -> rusqlite::Result<HashMap<ContactId, Contact>> {
    let mut people: HashMap<ContactId, Contact> = HashMap::new();
    let mut stmt = conn.prepare("SELECT ROWID, First, Last, Organization FROM ABPerson")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    for row in rows {
        let (id, first, last, org) = row?;
        let first = first.unwrap_or_default();
        let last = last.unwrap_or_default();
        let org = org.unwrap_or_default();
        let Some(name) = compose_name(&first, &last, &org) else {
            continue; // no name and no organization: skip the person
        };
        people.insert(
            id,
            Contact {
                id,
                name,
                organization: non_blank(&org),
            },
        );
    }
    Ok(people)
}

/// Read `ABMultiValue` phone (property 3) and email (property 4) rows and add
/// them to the claim maps. Values whose `record_id` is not a known person are
/// ignored.
fn collect_values(
    conn: &Connection,
    people: &HashMap<ContactId, Contact>,
    default_cc: Option<&str>,
    phone_claims: &mut HashMap<String, HashSet<ContactId>>,
    email_claims: &mut HashMap<String, HashSet<ContactId>>,
) -> rusqlite::Result<()> {
    let mut stmt = conn
        .prepare("SELECT record_id, property, value FROM ABMultiValue WHERE property IN (3, 4)")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (record_id, property, value) = row?;
        let Some(value) = value else { continue };
        if !people.contains_key(&record_id) {
            continue; // a value with no person is dropped
        }
        match property {
            3 => {
                let key = phone_key(&value, default_cc);
                phone_claims.entry(key).or_default().insert(record_id);
            }
            4 => {
                let key = value.trim().to_lowercase();
                if key.is_empty() {
                    continue;
                }
                email_claims.entry(key).or_default().insert(record_id);
            }
            _ => {}
        }
    }
    Ok(())
}

/// The phone key for a raw value: the normalized E.164 form when it
/// normalizes, otherwise the value verbatim with whitespace stripped (D8:
/// verbatim match, never guess).
fn phone_key(value: &str, default_cc: Option<&str>) -> String {
    if let Some(e164) = normalize(value, default_cc) {
        return e164;
    }
    value.chars().filter(|c| !c.is_whitespace()).collect()
}

/// "First Last" trimmed; if both parts are empty, the organization trimmed; if
/// that is empty too, `None` (the person is skipped).
fn compose_name(first: &str, last: &str, org: &str) -> Option<String> {
    let name = format!("{first} {last}").trim().to_string();
    if !name.is_empty() {
        return Some(name);
    }
    non_blank(org)
}

/// `Some(trimmed)` when the string is non-blank, else `None`.
fn non_blank(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Keep only the keys claimed by exactly one distinct person. A key claimed by
/// two or more different people is ambiguous and is removed (unresolved).
fn resolve_claims(claims: &HashMap<String, HashSet<ContactId>>) -> HashMap<String, ContactId> {
    let mut out: HashMap<String, ContactId> = HashMap::new();
    for (key, ids) in claims {
        if ids.len() == 1 {
            if let Some(id) = ids.iter().copied().next() {
                out.insert(key.clone(), id);
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "contacts_tests.rs"]
mod tests;
