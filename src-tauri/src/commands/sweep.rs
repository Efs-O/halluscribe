// HalluScribe - sweep lifecycle Tauri command handlers.

use crate::app_state::SweepCancel;
use crate::app_support::{archive_dir, default_archive_dir, mark_sweep_success};
use crate::{scheduler, settings};
use tauri::{Emitter, Manager};

/// Run a sweep immediately ("Run Now" button). Bypasses the time-window check.
#[tauri::command]
pub(crate) fn trigger_sweep(app: tauri::AppHandle) -> Result<(), String> {
    let dir = archive_dir(&app)?;
    let default_root = default_archive_dir(&app)?;
    let import_only = crate::workspace::is_active_import_only(&default_root, &dir)?;
    let settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    settings.generation_limits()?;
    let mut config = settings
        .to_sweep_config(dir, true)
        .ok_or_else(|| "backend is not configured (check Settings)".to_string())?;
    config.import_only = import_only;
    // A fresh cancel flag per run: a Cancel issued for a prior sweep can never
    // be cleared by this trigger, and this run's flag is not shared with the
    // daily scheduler or a concurrent *Run Now*.
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
            let marker_errors = if result.completed_successfully() {
                mark_sweep_success(&app)
                    .err()
                    .into_iter()
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let message = scheduler::sweep_done_message(&result, &marker_errors);
            let _ = app.emit("sweep-done", message);
        }
        app.state::<SweepCancel>().0.finish_run(&cancel);
    });
    Ok(())
}

/// Signal the active sweep to stop after the current session finishes.
#[tauri::command]
pub(crate) fn cancel_sweep(app: tauri::AppHandle) {
    app.state::<SweepCancel>().0.request_cancel();
}
