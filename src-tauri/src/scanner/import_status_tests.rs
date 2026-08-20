// HalluScribe - tests for the import-path diagnostic (GROK_IMPORT_PLAN
// § 12.4.4). The case that matters most is `NoExportFound`: a path that is set,
// exists, and still yields nothing. That is what silently cost a sweep cycle.

use super::import_status::*;
use crate::settings::HalluScribeSettings;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn grok_settings(path: &Path) -> HalluScribeSettings {
    HalluScribeSettings {
        grok_import_path: path.display().to_string(),
        ..HalluScribeSettings::default()
    }
}

/// A provider the user does not use reports `Unset`, not a problem to fix.
#[test]
fn a_blank_path_is_unset_rather_than_an_error() {
    let status = import_status(&HalluScribeSettings::default(), "grok");
    assert_eq!(status.state, ImportPathState::Unset);
    assert_eq!(status.file_count, 0);
}

/// Whitespace is not a configured path - the sweep trims before using it, so
/// the diagnostic must agree or it would report a path the sweep ignores.
#[test]
fn a_whitespace_only_path_is_unset() {
    let settings = HalluScribeSettings {
        grok_import_path: "   ".to_string(),
        ..HalluScribeSettings::default()
    };
    assert_eq!(
        import_status(&settings, "grok").state,
        ImportPathState::Unset
    );
}

/// A folder that has been renamed, deleted, or lives on an unmounted drive is
/// a different problem from an empty one, and gets its own state.
#[test]
fn a_path_that_does_not_exist_reports_missing_folder() {
    let dir = tempdir().unwrap();
    let gone = dir.path().join("not-here");
    let status = import_status(&grok_settings(&gone), "grok");
    assert_eq!(status.state, ImportPathState::MissingFolder);
}

/// THE case this feature exists for: the folder is real, it was searched, and
/// there is no export in it. Yesterday this was completely silent.
#[test]
fn a_real_folder_with_no_export_reports_no_export_found() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("readme.txt"), "not an export").unwrap();

    let status = import_status(&grok_settings(dir.path()), "grok");
    assert_eq!(status.state, ImportPathState::NoExportFound);
    assert_eq!(status.file_count, 0);
    assert!(!status.path.is_empty(), "the path is still reported back");
}

/// An export buried deeper than the discovery walk goes reads as "no export
/// found" rather than as success. This is exactly the shape of the live Grok
/// failure, one level past the cap, and the reason the diagnostic matters more
/// than the cap's exact value.
#[test]
fn an_export_below_the_depth_cap_reports_no_export_found() {
    let dir = tempdir().unwrap();
    let deep = dir
        .path()
        .join("a")
        .join("b")
        .join("c")
        .join("d")
        .join("e")
        .join("f");
    fs::create_dir_all(&deep).unwrap();
    fs::write(deep.join("prod-grok-backend.json"), "{}").unwrap();

    assert_eq!(
        import_status(&grok_settings(dir.path()), "grok").state,
        ImportPathState::NoExportFound
    );
}

/// The healthy case reports the file, its size, and where it actually is -
/// "found" on a stale copy is its own confusion, so the path is shown.
#[test]
fn a_found_export_reports_its_file_and_size() {
    let dir = tempdir().unwrap();
    let nested = dir
        .path()
        .join("GROK")
        .join("ttl")
        .join("30d")
        .join("export_data")
        .join("5134baa3");
    fs::create_dir_all(&nested).unwrap();
    let export = nested.join("prod-grok-backend.json");
    fs::write(&export, "{\"conversations\":[]}").unwrap();

    let status = import_status(&grok_settings(dir.path()), "grok");
    assert_eq!(status.state, ImportPathState::Found);
    assert_eq!(status.file_count, 1);
    assert_eq!(status.total_bytes, 20);
    assert_eq!(status.files, vec![export.display().to_string()]);
}

/// A split ChatGPT export is several files and is still healthy - the count is
/// information, not a warning.
#[test]
fn a_split_export_counts_every_chunk() {
    let dir = tempdir().unwrap();
    for idx in 0..3 {
        fs::write(
            dir.path().join(format!("conversations-{idx:03}.json")),
            "[]",
        )
        .unwrap();
    }
    let settings = HalluScribeSettings {
        chatgpt_import_path: dir.path().display().to_string(),
        ..HalluScribeSettings::default()
    };

    let status = import_status(&settings, "chatgpt");
    assert_eq!(status.state, ImportPathState::Found);
    assert_eq!(status.file_count, 3);
}

/// One provider's broken path must not colour another's - the whole point is
/// telling the user WHICH one needs attention.
#[test]
fn statuses_are_reported_per_provider() {
    let good = tempdir().unwrap();
    fs::write(good.path().join("conversations.json"), "[]").unwrap();
    let empty = tempdir().unwrap();

    let settings = HalluScribeSettings {
        chatgpt_import_path: good.path().display().to_string(),
        grok_import_path: empty.path().display().to_string(),
        ..HalluScribeSettings::default()
    };

    let all = all_import_statuses(&settings);
    assert_eq!(all.len(), 4, "one row per chat provider");
    let by_key = |key: &str| {
        all.iter()
            .find(|status| status.provider == key)
            .unwrap()
            .state
    };
    assert_eq!(by_key("chatgpt"), ImportPathState::Found);
    assert_eq!(by_key("grok"), ImportPathState::NoExportFound);
    assert_eq!(by_key("claude_ai"), ImportPathState::Unset);
    assert_eq!(by_key("gemini"), ImportPathState::Unset);
}

/// The rows come back in the order Settings lays the fields out, so the UI can
/// render them positionally without re-sorting.
#[test]
fn statuses_come_back_in_settings_field_order() {
    let keys: Vec<String> = all_import_statuses(&HalluScribeSettings::default())
        .into_iter()
        .map(|status| status.provider)
        .collect();
    assert_eq!(keys, ["chatgpt", "claude_ai", "gemini", "grok"]);
}
