// HalluScribe - sweep lifecycle Tauri command handlers.

use crate::app_state::SweepCancel;
use crate::app_support::{archive_dir, default_archive_dir, mark_sweep_success};
use crate::{scheduler, settings};
use std::path::Path;
use tauri::{Emitter, Manager};

/// Run a sweep immediately ("Run Now" button). Bypasses the time-window check.
#[tauri::command]
pub(crate) fn trigger_sweep(app: tauri::AppHandle) -> Result<(), String> {
    let config = build_sweep_config(&app)?;
    spawn_sweep(app, config, false)
}

/// Run a business import now. This is the business consent gate: it refuses
/// when business ingestion is disabled, and records the outcome (last import
/// time / last error) in the settings so the business settings section can show
/// it. The sweep itself is the same one `trigger_sweep` runs — business
/// sessions are picked up because `business_ingestion_enabled` is on, which is
/// what `scanner::resolve_apple_backup_path` requires. One gate, no bypass:
/// the button is also disabled in the UI when the toggle is off.
#[tauri::command]
pub(crate) fn trigger_business_import(app: tauri::AppHandle) -> Result<(), String> {
    let dir = archive_dir(&app)?;
    let settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    business_import_gate(&settings)?;
    let config = build_sweep_config(&app)?;
    spawn_sweep(app, config, true)
}

/// The business consent gate: a business import is refused while business
/// ingestion is disabled. Pure (settings in, verdict out) so the one-consent-gate
/// rule is unit-tested without a Tauri handle.
fn business_import_gate(settings: &settings::HalluScribeSettings) -> Result<(), String> {
    if !settings.business_ingestion_enabled {
        return Err("Business ingestion is disabled. Enable it in Settings first.".to_string());
    }
    Ok(())
}

/// Load the settings and build the sweep config for an on-demand run, applying
/// the import-only flag for the active workspace. Shared by `trigger_sweep` and
/// `trigger_business_import` so the two cannot diverge on what a run does.
fn build_sweep_config(app: &tauri::AppHandle) -> Result<scheduler::SweepConfig, String> {
    let dir = archive_dir(app)?;
    let default_root = default_archive_dir(app)?;
    let import_only = crate::workspace::is_active_import_only(&default_root, &dir)?;
    let settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    settings.generation_limits()?;
    let mut config = settings
        .to_sweep_config(dir, true)
        .ok_or_else(|| "backend is not configured (check Settings)".to_string())?;
    config.import_only = import_only;
    Ok(config)
}

/// Spawn the sweep on a background thread, emitting `sweep-done` when it
/// finishes. A fresh cancel flag per run: a Cancel issued for a prior sweep can
/// never be cleared by this trigger, and this run's flag is not shared with the
/// daily scheduler or a concurrent *Run Now*. When `record_business` is set (the
/// business import path) the outcome is also written back to the business
/// settings before the event is emitted.
fn spawn_sweep(
    app: tauri::AppHandle,
    config: scheduler::SweepConfig,
    record_business: bool,
) -> Result<(), String> {
    let dir = archive_dir(&app)?;
    let cancel = app
        .state::<SweepCancel>()
        .0
        .try_begin_run()
        .ok_or_else(|| "A sweep is already running.".to_string())?;
    std::thread::spawn(move || {
        let result = scheduler::run_sweep(&app, &config, cancel.clone());
        if result.busy {
            let _ = app.emit(
                "sweep-done",
                "A sweep or other model job is already running. Try again once it finishes."
                    .to_string(),
            );
        } else {
            let mut marker_errors = if result.completed_successfully() {
                mark_sweep_success(&app)
                    .err()
                    .into_iter()
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            if record_business {
                // A failure to record the outcome is not noise: the UI would
                // otherwise keep showing a stale last-import/last-error. Push it
                // into the marker errors so it reaches the sweep-done message.
                if let Err(error) = record_business_outcome(&dir, &result) {
                    marker_errors.push(error);
                }
            }
            let message = scheduler::sweep_done_message(&result, &marker_errors);
            let _ = app.emit("sweep-done", message);
        }
        app.state::<SweepCancel>().0.finish_run(&cancel);
    });
    Ok(())
}

/// Write the outcome of a business import back to the settings: the UTC
/// completion time on a clean run, or the first error otherwise. A busy or
/// cancelled run writes neither — it is not an import result (a legit `Ok(())`).
/// A settings load or save failure is surfaced, not swallowed: the caller folds
/// it into the sweep-done message so the user is told the last-import/last-error
/// could not be updated.
fn record_business_outcome(dir: &Path, result: &scheduler::SweepResult) -> Result<(), String> {
    if result.busy || result.cancelled {
        return Ok(());
    }
    let mut settings = settings::load_settings(dir)
        .map_err(|error| format!("could not record business import status: {error}"))?;
    if result.completed_successfully() {
        settings.business_last_import = chrono::Utc::now().to_rfc3339();
        settings.business_last_error = String::new();
    } else {
        settings.business_last_error = result
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| "The import did not complete.".to_string());
    }
    settings::save_settings(dir, &settings)
        .map_err(|error| format!("could not record business import status: {error}"))?;
    Ok(())
}

/// Signal the active sweep to stop after the current session finishes.
#[tauri::command]
pub(crate) fn cancel_sweep(app: tauri::AppHandle) {
    app.state::<SweepCancel>().0.request_cancel();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_business_gate_refuses_when_ingestion_is_disabled() {
        let settings = settings::HalluScribeSettings::default();
        assert!(!settings.business_ingestion_enabled);
        let err = business_import_gate(&settings).unwrap_err();
        assert!(err.contains("disabled"), "{err}");
    }

    #[test]
    fn the_business_gate_allows_when_ingestion_is_enabled() {
        let settings = settings::HalluScribeSettings {
            business_ingestion_enabled: true,
            ..Default::default()
        };
        assert!(business_import_gate(&settings).is_ok());
    }

    #[test]
    fn a_business_outcome_cannot_be_recorded_when_settings_are_unreadable() {
        // Make settings.json a directory so load_settings fails to read it: the
        // outcome write must surface that error rather than swallow it.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("settings.json")).unwrap();
        let result = scheduler::SweepResult::default();
        let err = record_business_outcome(dir.path(), &result).unwrap_err();
        assert!(
            err.contains("could not record business import status"),
            "{err}"
        );
    }
}
