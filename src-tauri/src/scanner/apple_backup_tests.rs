// HalluScribe - scanner tests for the Apple backup Messages target: a backup
// only yields a Messages target when its manifest lists `sms.db`. Synthetic
// fixtures only (a hand-built `Manifest.db`), no real backup data.

#[cfg(test)]
mod tests {
    use super::super::{scan_sessions, ScanTargetKind};
    use crate::readers::ChatProvider;
    use crate::settings::HalluScribeSettings;
    use rusqlite::Connection;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    /// A backup dir whose `Manifest.db` lists the given (domain, path) rows,
    /// each with its hashed payload file present.
    fn make_manifest(root: &Path, rows: &[(&str, &str)]) {
        let conn = Connection::open(root.join("Manifest.db")).expect("open manifest");
        conn.execute_batch("CREATE TABLE Files (fileID TEXT, domain TEXT, relativePath TEXT);")
            .expect("create Files");
        for (i, (domain, path)) in rows.iter().enumerate() {
            let file_id = format!("{i:040x}");
            conn.execute(
                "INSERT INTO Files VALUES (?1, ?2, ?3)",
                rusqlite::params![file_id, domain, path],
            )
            .expect("insert row");
            let payload = root.join(&file_id[..2]).join(&file_id);
            fs::create_dir_all(payload.parent().unwrap()).expect("create hash dir");
            fs::write(&payload, b"fake payload").expect("write payload");
        }
    }

    fn apple_targets(root: &Path) -> usize {
        let settings = HalluScribeSettings {
            apple_backup_path: root.display().to_string(),
            business_ingestion_enabled: true,
            ..HalluScribeSettings::default()
        };
        scan_sessions(root, &settings, u64::MAX, 0.0, false)
            .iter()
            .filter(|t| matches!(t.kind, ScanTargetKind::Import(ChatProvider::AppleMessages)))
            .count()
    }

    #[test]
    fn a_backup_without_sms_db_yields_no_messages_target() {
        let dir = tempdir().unwrap();
        make_manifest(
            dir.path(),
            &[(
                "AppDomain-group.net.whatsapp.WhatsApp.shared",
                "ChatStorage.sqlite",
            )],
        );
        assert_eq!(apple_targets(dir.path()), 0);
    }

    #[test]
    fn a_backup_with_sms_db_yields_one_messages_target() {
        let dir = tempdir().unwrap();
        make_manifest(dir.path(), &[("HomeDomain", "Library/SMS/sms.db")]);
        assert_eq!(apple_targets(dir.path()), 1);
    }
}
