// HalluScribe - which settings belong to the machine and which to the person.
//
// A guest workspace used to receive a COPY of the machine's inference config at
// creation time (`seed_workspace_settings`) and nothing ever reconciled the two
// again. Change the model on the host and every guest kept pointing at whatever
// was current the day it was made - a stale `gemma_model_path` then surfaces as
// a sweep that finds nothing and a profile build that fails, nowhere near
// settings. See docs/internal/IMPORT_PATHS_PLAN.md Phase D.
//
// Machine fields are therefore read THROUGH to the host at load time instead of
// being copied. The host's `settings.json` is the single authority; a guest file
// stops carrying those keys at all, so there is no second copy left to drift.

use super::{persistence, HalluScribeSettings, SettingsError};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn host_root() -> &'static OnceLock<PathBuf> {
    static HOST_ROOT: OnceLock<PathBuf> = OnceLock::new();
    &HOST_ROOT
}

/// Record the DEFAULT archive root (`<home>/.halluscribe`) as the machine's
/// settings authority. Called once during setup, exactly as
/// `llama_tuning::set_host_root` and `llama_runtime::set_log_dir` are, because
/// the sweep thread and the briefing chat carry no app handle of their own.
/// Later calls are ignored so a workspace switch cannot move the authority.
pub fn set_host_root(dir: PathBuf) {
    if host_root().set(dir).is_err() {
        eprintln!("[settings] host root was already configured; keeping the original location");
    }
}

/// True when `archive_dir` IS the host root, i.e. the settings there are the
/// authority and nothing needs reading through.
///
/// An unset root means setup never resolved one, which only happens outside the
/// running app (tests, examples). There is no registry in that case and so no
/// guest can exist: the file on disk is the only authority there is.
fn is_host_root(archive_dir: &Path) -> bool {
    match host_root().get() {
        Some(root) => same_dir(root, archive_dir),
        None => true,
    }
}

/// Compare two archive roots. Canonicalising first so `C:\Users\x\.halluscribe`
/// and a differently-cased or `..`-relative spelling of it are recognised as one
/// place; a path that cannot be canonicalised (not yet created) falls back to a
/// literal comparison rather than reporting a false match.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => a == b,
    }
}

/// The directory whose `settings.json` owns the MACHINE fields governing
/// `archive_dir` - the host root once setup has resolved one, otherwise
/// `archive_dir` itself.
///
/// Code that PERSISTS a host-owned value must target this rather than the
/// active workspace. Writing window geometry into a guest's file would be
/// dropped again by `strip_host_owned` on the way to disk, so the size would
/// silently never stick.
pub fn owning_dir(archive_dir: &Path) -> PathBuf {
    match host_root().get() {
        Some(root) => root.clone(),
        None => archive_dir.to_path_buf(),
    }
}

/// Overlay the machine's settings onto a guest's own file.
///
/// The host is read with `persistence::load_own` - never through this function -
/// so the authority is read exactly as written and the resolution cannot recurse.
pub(super) fn resolve(
    archive_dir: &Path,
    own: HalluScribeSettings,
) -> Result<HalluScribeSettings, SettingsError> {
    if is_host_root(archive_dir) {
        return Ok(own);
    }
    let Some(root) = host_root().get() else {
        return Ok(own);
    };
    let host = persistence::load_own(root)?;
    Ok(merge_host_owned(&host, own))
}

/// Take the machine fields from `host` and the person fields from `guest`.
///
/// The guest is destructured exhaustively on purpose: adding a field to
/// `HalluScribeSettings` breaks this function until its owner has been decided
/// here. Defaulting silently either way is the bug this module exists to fix -
/// a new machine field that stayed workspace-owned would drift, and a new
/// consent field that became host-owned would leak one person's choice to
/// everybody on the machine.
pub(super) fn merge_host_owned(
    host: &HalluScribeSettings,
    guest: HalluScribeSettings,
) -> HalluScribeSettings {
    let HalluScribeSettings {
        // --- machine: the host's value wins, the guest's copy is discarded ---
        always_on_top: _,
        window_width: _,
        window_height: _,
        backend: _,
        llama_server_bin: _,
        gemma_model_path: _,
        embedding_model_path: _,
        gpu_layers: _,
        gpu_devices: _,
        gpu_split_mode: _,
        gpu_tensor_split: _,
        gpu_main_index: _,
        llama_server_port: _,
        ollama_host: _,
        ollama_port: _,
        ollama_model: _,
        ollama_api_key: _,
        tavily_api_key: _,
        summary_min_fill_pct: _,
        lookback_hours: _,
        idle_threshold_mins: _,
        ctx_size: _,
        max_tokens: _,
        briefing_window_hours: _,
        tts_piper_bin: _,
        tts_voice: _,
        // --- person: kept, and never shared with another workspace ---
        chatgpt_import_path,
        claudeai_import_path,
        gemini_import_path,
        grok_import_path,
        ollama_chat_db_path,
        forge_sessions_path,
        scheduled_processing_enabled,
        schedule_time,
        last_sweep_date,
        last_auto_sweep_attempt_at,
        auto_sweep_retry_count,
        auto_sweep_exhausted_date,
        first_run,
        // `profile_sources` is a CONSENT setting - which of this person's
        // providers the distiller may read. Sharing it across people would be a
        // privacy regression, so it can never become host-owned.
        profile_sources,
        // This person's business contacts' default country code - a
        // person-owned value, never shared across people.
        business_default_country_code,
    } = guest;

    HalluScribeSettings {
        chatgpt_import_path,
        claudeai_import_path,
        gemini_import_path,
        grok_import_path,
        ollama_chat_db_path,
        forge_sessions_path,
        scheduled_processing_enabled,
        schedule_time,
        last_sweep_date,
        last_auto_sweep_attempt_at,
        auto_sweep_retry_count,
        auto_sweep_exhausted_date,
        first_run,
        profile_sources,
        business_default_country_code,
        ..host.clone()
    }
}

/// Drop the machine fields before a guest's file is written, so the only copy of
/// them on disk stays the host's. Reuses `merge_host_owned` against the defaults
/// rather than repeating the field list, which keeps the two in step.
pub(super) fn strip_host_owned(
    archive_dir: &Path,
    settings: &HalluScribeSettings,
) -> Option<HalluScribeSettings> {
    if is_host_root(archive_dir) {
        return None;
    }
    Some(merge_host_owned(
        &HalluScribeSettings::default(),
        settings.clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_fixture() -> HalluScribeSettings {
        HalluScribeSettings {
            llama_server_bin: "C:/llamacpp/b10430/llama-server.exe".to_string(),
            gemma_model_path: "N:/QWEN/Qwen3.8-27B.gguf".to_string(),
            ctx_size: 60000,
            max_tokens: 32768,
            gpu_layers: 999,
            lookback_hours: 20000,
            ..HalluScribeSettings::default()
        }
    }

    fn drifted_guest() -> HalluScribeSettings {
        HalluScribeSettings {
            llama_server_bin: "C:/llamacpp/b10237/llama-server.exe".to_string(),
            gemma_model_path: "N:/GEMMA/gone-IQ3_S.gguf".to_string(),
            ctx_size: 102400,
            max_tokens: 65536,
            gpu_layers: -1,
            lookback_hours: 24000,
            chatgpt_import_path: "C:/guest/imports/chatgpt".to_string(),
            profile_sources: vec!["claude_code".to_string()],
            last_sweep_date: "2026-08-07".to_string(),
            last_auto_sweep_attempt_at: "2026-08-08T03:30:00+03:00".to_string(),
            auto_sweep_retry_count: 2,
            auto_sweep_exhausted_date: "2026-08-08".to_string(),
            first_run: false,
            schedule_time: "03:30".to_string(),
            ..HalluScribeSettings::default()
        }
    }

    #[test]
    fn machine_fields_come_from_the_host() {
        let merged = merge_host_owned(&host_fixture(), drifted_guest());
        // The exact drift measured on chara 2026-08-17.
        assert_eq!(
            merged.llama_server_bin,
            "C:/llamacpp/b10430/llama-server.exe"
        );
        assert_eq!(merged.gemma_model_path, "N:/QWEN/Qwen3.8-27B.gguf");
        assert_eq!(merged.ctx_size, 60000);
        assert_eq!(merged.max_tokens, 32768);
        assert_eq!(merged.gpu_layers, 999);
        assert_eq!(merged.lookback_hours, 20000);
    }

    #[test]
    fn person_fields_stay_with_the_workspace() {
        let merged = merge_host_owned(&host_fixture(), drifted_guest());
        assert_eq!(merged.chatgpt_import_path, "C:/guest/imports/chatgpt");
        assert_eq!(merged.profile_sources, vec!["claude_code".to_string()]);
        assert_eq!(merged.last_sweep_date, "2026-08-07");
        assert_eq!(
            merged.last_auto_sweep_attempt_at,
            "2026-08-08T03:30:00+03:00"
        );
        assert_eq!(merged.auto_sweep_retry_count, 2);
        assert_eq!(merged.auto_sweep_exhausted_date, "2026-08-08");
        assert_eq!(merged.schedule_time, "03:30");
        assert!(!merged.first_run);
    }

    #[test]
    fn consent_is_never_taken_from_the_host() {
        // The host allows six providers by default; the guest allows one. A
        // merge must not widen the guest's consent.
        let host = host_fixture();
        assert!(host.profile_sources.len() > 1);
        let merged = merge_host_owned(&host, drifted_guest());
        assert_eq!(merged.profile_sources, vec!["claude_code".to_string()]);
    }

    #[test]
    fn a_stripped_guest_file_carries_no_machine_fields() {
        let stripped = merge_host_owned(&HalluScribeSettings::default(), drifted_guest());
        let defaults = HalluScribeSettings::default();
        assert_eq!(stripped.llama_server_bin, defaults.llama_server_bin);
        assert_eq!(stripped.gemma_model_path, defaults.gemma_model_path);
        assert_eq!(stripped.ctx_size, defaults.ctx_size);
        // ...while the person's own fields survive the round trip.
        assert_eq!(stripped.chatgpt_import_path, "C:/guest/imports/chatgpt");
        assert_eq!(stripped.last_sweep_date, "2026-08-07");
        assert_eq!(
            stripped.last_auto_sweep_attempt_at,
            "2026-08-08T03:30:00+03:00"
        );
        assert_eq!(stripped.auto_sweep_retry_count, 2);
        assert_eq!(stripped.auto_sweep_exhausted_date, "2026-08-08");
    }

    #[test]
    fn host_owned_writes_fall_back_to_the_archive_with_no_root_set() {
        // Outside the running app there is no registry and so no guest; the
        // archive in hand owns its own machine fields.
        assert_eq!(owning_dir(Path::new("C:/guest")), PathBuf::from("C:/guest"));
    }

    #[test]
    fn an_unset_host_root_leaves_settings_untouched() {
        // Tests and examples never call `set_host_root`; with no registry there
        // is no guest, so the file on disk is the only authority.
        let guest = drifted_guest();
        let resolved = resolve(Path::new("C:/anywhere"), guest.clone()).expect("resolve");
        assert_eq!(resolved.llama_server_bin, guest.llama_server_bin);
        assert!(strip_host_owned(Path::new("C:/anywhere"), &guest).is_none());
    }
}
