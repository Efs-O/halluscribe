// HalluScribe - read-only resolver for iPhone (Apple Devices) backups.
//
// A backup is a directory of SQLite `Manifest.db` plus hashed payload files.
// This module opens a backup, resolves logical files (domain + relativePath)
// to their on-disk location, and hands out temp copies so callers can open the
// SQLite files read-only without ever touching the original directory.
//
// Phase 1 of the Business Messaging ingestion plan: this is the probe layer
// only - it reports metadata, never message content.

pub mod info_plist;
pub mod manifest;
pub mod typedstream;

pub use info_plist::plist_value;
pub use manifest::{open_backup, open_sqlite_read_only, BackupError, BackupHandle, TempCopy};
pub use typedstream::decode_attributed_body;
