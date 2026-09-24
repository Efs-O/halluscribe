// HalluScribe - tests for the Messages DB reader's multi-chat joins and
// unreadable attachments. All fixtures are synthetic: an in-memory sms.db with
// invented guids and handles. No real data.

use super::load;
use rusqlite::Connection;

/// The sms.db schema the reader depends on (same shape as the main tests).
const SCHEMA: &str = r#"
CREATE TABLE message (
    ROWID INTEGER PRIMARY KEY, guid TEXT, text TEXT, attributedBody BLOB,
    handle_id INTEGER, date INTEGER, is_from_me INTEGER, service TEXT,
    associated_message_type INTEGER, item_type INTEGER
);
CREATE TABLE handle (ROWID INTEGER PRIMARY KEY, id TEXT, service TEXT);
CREATE TABLE chat (
    ROWID INTEGER PRIMARY KEY, guid TEXT, chat_identifier TEXT,
    display_name TEXT, service_name TEXT
);
CREATE TABLE chat_message_join (chat_id INTEGER, message_id INTEGER);
CREATE TABLE chat_handle_join (chat_id INTEGER, handle_id INTEGER);
CREATE TABLE attachment (
    ROWID INTEGER PRIMARY KEY, filename TEXT, mime_type TEXT,
    transfer_name TEXT, total_bytes INTEGER
);
CREATE TABLE message_attachment_join (message_id INTEGER, attachment_id INTEGER);
"#;

/// One handle, two chats and one text message from that handle.
fn fixture() -> Connection {
    let c = Connection::open_in_memory().expect("in-memory db");
    c.execute_batch(SCHEMA).expect("create schema");
    c.execute_batch(
        "INSERT INTO handle VALUES (1, '+15550000001', 'SMS');
         INSERT INTO chat VALUES (10, 'chat-a', 'a', NULL, 'SMS');
         INSERT INTO chat VALUES (20, 'chat-b', 'b', NULL, 'SMS');
         INSERT INTO message VALUES (1, 'guid-1', 'hello', NULL, 1, 700000000, 0, 'SMS', 0, 0);",
    )
    .expect("insert rows");
    c
}

#[test]
fn a_message_joined_to_two_chats_is_loaded_once() {
    let c = fixture();
    c.execute_batch(
        "INSERT INTO chat_message_join VALUES (20, 1);
         INSERT INTO chat_message_join VALUES (10, 1);",
    )
    .expect("join");

    let db = load(&c).expect("load");

    assert_eq!(db.messages.len(), 1);
    assert_eq!(
        db.messages[0].conversation,
        super::ConversationKey::Chat(10)
    );
    assert_eq!(db.skipped.duplicate_joins, 1);
    assert_eq!(db.conversations.len(), 1);
}

#[test]
fn an_unreadable_attachment_is_counted_and_the_message_kept() {
    let c = fixture();
    c.execute_batch(
        "INSERT INTO chat_message_join VALUES (10, 1);
         INSERT INTO attachment VALUES (1, '/x/a.jpg', 'image/jpeg', 'a.jpg', 'abc');
         INSERT INTO attachment VALUES (2, '/x/b.jpg', 'image/jpeg', 'b.jpg', 42);
         INSERT INTO message_attachment_join VALUES (1, 1);
         INSERT INTO message_attachment_join VALUES (1, 2);",
    )
    .expect("attachments");

    let db = load(&c).expect("an unreadable attachment must not fail the load");

    assert_eq!(db.messages.len(), 1);
    assert_eq!(db.skipped.attachment_errors, 1);
    let names: Vec<_> = db.messages[0]
        .attachments
        .iter()
        .map(|a| a.transfer_name.as_deref())
        .collect();
    assert_eq!(names, vec![Some("b.jpg")]);
}
