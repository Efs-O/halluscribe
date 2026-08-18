// HalluScribe - tests for the uniform import-discovery rule (GROK_IMPORT_PLAN
// Part III): one bounded-depth, shallowest-wins search anchored on candidate
// file names, replacing the per-provider literal joins. The provider-specific
// layout tests (Grok's ttl/30d nesting, Gemini's Takeout, split ChatGPT
// exports) stay in tests.rs and must keep passing unmodified - they are what
// proves the generic mechanism covers the real layouts.

#[cfg(test)]
mod tests {
    use super::super::{chat_import_sources, scan_chat_imports};
    use crate::settings::HalluScribeSettings;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn chatgpt_settings(base: &Path) -> HalluScribeSettings {
        HalluScribeSettings {
            chatgpt_import_path: base.display().to_string(),
            ..HalluScribeSettings::default()
        }
    }

    fn swept(settings: &HalluScribeSettings) -> Vec<PathBuf> {
        scan_chat_imports(settings, u64::MAX)
            .into_iter()
            .map(|target| target.path)
            .collect()
    }

    /// The live trap this rewrite exists for: unzipping a ChatGPT export
    /// produces a wrapper folder, and the old literal join found nothing at
    /// all - silently, with no diagnostic.
    #[test]
    fn chatgpt_export_inside_a_wrapper_folder_is_found() {
        let dir = tempdir().unwrap();
        let wrapper = dir.path().join("chatgpt-export-2026-08-18");
        fs::create_dir_all(&wrapper).unwrap();
        fs::write(wrapper.join("conversations.json"), "[]").unwrap();

        let settings = chatgpt_settings(dir.path());
        assert_eq!(swept(&settings), vec![wrapper.join("conversations.json")]);
        // Both authorities, one answer.
        assert_eq!(
            chat_import_sources(&settings, "chatgpt"),
            vec![wrapper.join("conversations.json")]
        );
    }

    /// Shallowest-wins is the safety property: a stale copy kept in a
    /// subfolder must never be imported alongside the current export, or the
    /// archive takes conflicting writes for the same conversation ids.
    #[test]
    fn a_stale_copy_deeper_in_the_tree_is_ignored() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("conversations.json"), "[]").unwrap();
        let backup = dir.path().join("old-backup");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("conversations.json"), "[]").unwrap();

        let settings = chatgpt_settings(dir.path());
        assert_eq!(
            swept(&settings),
            vec![dir.path().join("conversations.json")]
        );
    }

    /// Two copies at the SAME depth are both returned - that is what split
    /// export chunks need, and there is no basis for preferring one folder.
    #[test]
    fn matches_at_the_same_depth_are_all_returned() {
        let dir = tempdir().unwrap();
        for name in ["part-a", "part-b"] {
            let sub = dir.path().join(name);
            fs::create_dir_all(&sub).unwrap();
            fs::write(sub.join("conversations.json"), "[]").unwrap();
        }

        let settings = chatgpt_settings(dir.path());
        assert_eq!(swept(&settings).len(), 2);
    }

    /// Anchors are file names, not arbitrary JSON: an unrelated export sitting
    /// in the same folder is never mistaken for a conversation file.
    #[test]
    fn unrelated_json_files_are_not_mistaken_for_an_export() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("user.json"), "{}").unwrap();
        fs::write(dir.path().join("message_feedback.json"), "[]").unwrap();

        assert!(swept(&chatgpt_settings(dir.path())).is_empty());
    }

    /// The real Gemini Takeout layout, at its real depth - this is the one
    /// the literal `Takeout/My Activity/Gemini Apps/My Activity.json`
    /// candidate used to cover, and the archives on disk actually use.
    #[test]
    fn gemini_takeout_layout_is_found_at_its_real_depth() {
        let dir = tempdir().unwrap();
        let takeout = dir
            .path()
            .join("Takeout")
            .join("My Activity")
            .join("Gemini Apps");
        fs::create_dir_all(&takeout).unwrap();
        fs::write(takeout.join("My Activity.json"), "[]").unwrap();

        let settings = HalluScribeSettings {
            gemini_import_path: dir.path().display().to_string(),
            ..HalluScribeSettings::default()
        };
        assert_eq!(swept(&settings), vec![takeout.join("My Activity.json")]);
        assert_eq!(
            chat_import_sources(&settings, "gemini"),
            vec![takeout.join("My Activity.json")]
        );
    }

    /// Split ChatGPT chunks with no un-suffixed `conversations.json` beside
    /// them - the layout the chara workspace actually has on disk.
    #[test]
    fn split_chunks_without_a_base_file_are_all_found() {
        let dir = tempdir().unwrap();
        for idx in 0..5 {
            fs::write(
                dir.path().join(format!("conversations-{idx:03}.json")),
                "[]",
            )
            .unwrap();
        }

        assert_eq!(swept(&chatgpt_settings(dir.path())).len(), 5);
    }

    /// Case-insensitive matching. On Windows and macOS this passes whatever
    /// the comparison does; on Linux (which CI runs) it fails the moment the
    /// comparison becomes case-sensitive. See HALLUSCRIBE_PLAN § "CI vs Local".
    #[test]
    fn candidate_names_match_case_insensitively() {
        let dir = tempdir().unwrap();
        let takeout = dir.path().join("Takeout").join("Gemini Apps");
        fs::create_dir_all(&takeout).unwrap();
        fs::write(takeout.join("my activity.json"), "[]").unwrap();

        let settings = HalluScribeSettings {
            gemini_import_path: dir.path().display().to_string(),
            ..HalluScribeSettings::default()
        };
        assert_eq!(swept(&settings), vec![takeout.join("my activity.json")]);
    }

    /// The depth cap: an export buried deeper than `MAX_IMPORT_DEPTH` is not
    /// found, which is deliberate - the alternative is walking a whole drive.
    #[test]
    fn an_export_below_the_depth_cap_is_not_searched_for() {
        let dir = tempdir().unwrap();
        let deep = dir.path().join("a").join("b").join("c").join("d").join("e");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("conversations.json"), "[]").unwrap();

        assert!(swept(&chatgpt_settings(dir.path())).is_empty());
    }

    /// A wide, export-free tree must return promptly rather than walking
    /// forever - the case where a user points the setting at their Desktop.
    #[test]
    fn a_wide_empty_tree_returns_promptly() {
        let dir = tempdir().unwrap();
        for outer in 0..40 {
            for inner in 0..40 {
                fs::create_dir_all(
                    dir.path()
                        .join(format!("d{outer}"))
                        .join(format!("e{inner}")),
                )
                .unwrap();
            }
        }
        let started = std::time::Instant::now();
        assert!(swept(&chatgpt_settings(dir.path())).is_empty());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "the directory-visit cap did not bound the walk"
        );
    }

    /// Dot-directories are tool state, not exports, and are skipped.
    #[test]
    fn hidden_directories_are_skipped() {
        let dir = tempdir().unwrap();
        let hidden = dir.path().join(".trash");
        fs::create_dir_all(&hidden).unwrap();
        fs::write(hidden.join("conversations.json"), "[]").unwrap();

        assert!(swept(&chatgpt_settings(dir.path())).is_empty());
    }

    /// A base path pointing straight at the export file still works, which is
    /// what a user who picked the file rather than the folder ends up with.
    #[test]
    fn a_base_pointing_at_the_file_itself_is_used_as_is() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("conversations.json");
        fs::write(&file, "[]").unwrap();

        let settings = chatgpt_settings(&file);
        assert_eq!(swept(&settings), vec![file]);
    }
}
