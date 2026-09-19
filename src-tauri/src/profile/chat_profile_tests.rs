// HalluScribe - tests for the chat-profile resolver core (D13, Phase 9b): the
// business toggles decide whether the host owner's profile is read, a disabled
// scope is not loaded at all, and a missing owner profile is a distinct status,
// not an empty profile. Uses the pure core so no process host-root global is
// needed (which is set-once and would make tests order-dependent).

use super::resolve_chat_profile_core;
use super::ChatProfile;
use crate::profile::ProfileScope;
use std::fs;
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "halluscribe_chat_profile_{}_{}_{}",
        std::process::id(),
        name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&d).unwrap();
    d
}

/// Write a profile.md for `scope` under `dir` (the `<dir>/profile/<scope>/` layout).
fn write_profile(dir: &Path, scope: ProfileScope, body: &str) {
    let scope_dir = dir.join("profile").join(scope.dir_name());
    fs::create_dir_all(&scope_dir).unwrap();
    fs::write(scope_dir.join("profile.md"), body).unwrap();
}

#[test]
fn a_normal_workspace_reads_its_own_profile() {
    let dir = tmp("own");
    write_profile(&dir, ProfileScope::Work, "my own work profile");
    let got = resolve_chat_profile_core(&dir, ProfileScope::Work, false, true);
    assert!(matches!(got, ChatProfile::Loaded(s) if s == "my own work profile"));
}

#[test]
fn a_normal_workspace_with_no_profile_is_absent() {
    let dir = tmp("own_absent");
    let got = resolve_chat_profile_core(&dir, ProfileScope::Work, false, true);
    assert_eq!(got, ChatProfile::Absent);
}

#[test]
fn a_business_workspace_reads_the_owner_profile_not_its_own() {
    let owner = tmp("owner");
    write_profile(&owner, ProfileScope::Work, "owner work profile");
    // The guest's own profile must NOT be the one read: the core is handed the
    // owner's dir, so the guest's file is irrelevant to the result.
    let guest = tmp("guest");
    write_profile(&guest, ProfileScope::Work, "guest work profile");
    let got = resolve_chat_profile_core(&owner, ProfileScope::Work, true, true);
    assert!(matches!(got, ChatProfile::Loaded(s) if s == "owner work profile"));
}

#[test]
fn a_business_personal_scope_reads_the_owner_personal_profile() {
    let owner = tmp("owner_p");
    write_profile(&owner, ProfileScope::Personal, "owner personal profile");
    let got = resolve_chat_profile_core(&owner, ProfileScope::Personal, true, true);
    assert!(matches!(got, ChatProfile::Loaded(s) if s == "owner personal profile"));
}

#[test]
fn a_disabled_business_scope_is_not_loaded_at_all() {
    let owner = tmp("owner_off");
    write_profile(&owner, ProfileScope::Personal, "owner personal profile");
    // Personal is requested but its toggle is off -> Absent, even though the
    // owner has one. (business=true, enabled=false.)
    let got = resolve_chat_profile_core(&owner, ProfileScope::Personal, true, false);
    assert_eq!(got, ChatProfile::Absent);
}

#[test]
fn a_missing_owner_profile_is_owner_profile_not_found() {
    let owner = tmp("owner_missing");
    // No profile written for the owner.
    let got = resolve_chat_profile_core(&owner, ProfileScope::Work, true, true);
    assert_eq!(got, ChatProfile::OwnerProfileNotFound);
}

#[test]
fn a_blank_owner_profile_is_owner_profile_not_found() {
    let owner = tmp("owner_blank");
    write_profile(&owner, ProfileScope::Work, "   ");
    let got = resolve_chat_profile_core(&owner, ProfileScope::Work, true, true);
    assert_eq!(got, ChatProfile::OwnerProfileNotFound);
}

#[test]
fn a_blank_own_profile_in_a_normal_workspace_is_absent_not_not_found() {
    let dir = tmp("own_blank");
    write_profile(&dir, ProfileScope::Work, "   ");
    let got = resolve_chat_profile_core(&dir, ProfileScope::Work, false, true);
    assert_eq!(got, ChatProfile::Absent);
}

#[test]
fn an_import_only_workspace_with_both_toggles_off_reads_no_profile_even_if_the_guest_has_one() {
    let owner = tmp("owner_both_off");
    write_profile(&owner, ProfileScope::Work, "owner work profile");
    let guest = tmp("guest_both_off");
    write_profile(&guest, ProfileScope::Work, "guest work profile");
    // For an import-only workspace, resolve_chat_profile hands the core the
    // OWNER's dir (via owning_dir), business=true. Both toggles off => the
    // requested scope is not enabled. The guest's own profile is never read:
    // it lives in `guest`, not in the owner's dir the core is given. (plan:
    // "both off => no profile in context"; DEFECT 1: identity is the
    // workspace, not the toggles.)
    let got = resolve_chat_profile_core(&owner, ProfileScope::Work, true, false);
    assert_eq!(got, ChatProfile::Absent);
}

#[test]
fn a_normal_workspace_reads_its_own_profile_even_with_the_toggle_off() {
    let dir = tmp("own_toggle_off");
    write_profile(&dir, ProfileScope::Work, "my own work profile");
    // Non-import-only (business=false): the toggle is ignored, so even with
    // enabled=false the workspace's own profile is read. (DEFECT 1: the toggles
    // are consent, not identity — they only gate the owner's profile in an
    // import-only workspace.)
    let got = resolve_chat_profile_core(&dir, ProfileScope::Work, false, false);
    assert!(matches!(got, ChatProfile::Loaded(s) if s == "my own work profile"));
}

#[test]
fn a_work_only_business_workspace_loads_work_but_not_personal() {
    let owner = tmp("owner_work_only");
    write_profile(&owner, ProfileScope::Work, "owner work only");
    write_profile(&owner, ProfileScope::Personal, "owner personal only");
    // use_work=true, use_personal=false: Work is the enabled scope, Personal
    // is not. Requesting Work loads the owner's; requesting Personal is not
    // loaded at all, even though the owner has one.
    let work = resolve_chat_profile_core(&owner, ProfileScope::Work, true, true);
    assert!(matches!(work, ChatProfile::Loaded(s) if s == "owner work only"));
    let personal = resolve_chat_profile_core(&owner, ProfileScope::Personal, true, false);
    assert_eq!(personal, ChatProfile::Absent);
}

#[test]
fn a_personal_only_business_workspace_loads_personal_but_not_work() {
    let owner = tmp("owner_personal_only");
    write_profile(&owner, ProfileScope::Work, "owner work only");
    write_profile(&owner, ProfileScope::Personal, "owner personal only");
    // use_work=false, use_personal=true: Personal is the enabled scope, Work
    // is not. Requesting Personal loads the owner's; requesting Work is not
    // loaded at all.
    let personal = resolve_chat_profile_core(&owner, ProfileScope::Personal, true, true);
    assert!(matches!(personal, ChatProfile::Loaded(s) if s == "owner personal only"));
    let work = resolve_chat_profile_core(&owner, ProfileScope::Work, true, false);
    assert_eq!(work, ChatProfile::Absent);
}
