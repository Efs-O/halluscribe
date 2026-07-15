// HalluScribe - sweep lifecycle Tauri command handlers.

use crate::app_state::SweepCancel;
use crate::app_support::{archive_dir, clear_first_run, default_archive_dir, record_sweep_date};
use crate::{scheduler, settings};
use std::sync::atomic::Ordering;
use tauri::{Emitter, Manager};

/// Run a sweep immediately ("Run Now" button). Bypasses the time-window check.
#[tauri::command]
pub(crate) fn trigger_sweep(app: tauri::AppHandle) -> Result<(), String> {
    let dir = archive_dir(&app)?;
    let default_root = default_archive_dir(&app)?;
    let import_only = crate::workspace::is_active_import_only(&default_root, &dir);
    let settings = settings::load_settings(&dir);
    settings.generation_limits()?;
    let mut config = settings
        .to_sweep_config(dir, true)
        .ok_or_else(|| "backend is not configured (check Settings)".to_string())?;
    config.import_only = import_only;
    let cancel = app.state::<SweepCancel>().0.clone();
    cancel.store(false, Ordering::Relaxed);
    std::thread::spawn(move || {
        let result = scheduler::run_sweep(&app, &config, cancel);
        if result.busy {
            let _ = app.emit(
                "sweep-done",
                "A sweep or other model job is already running. Try again once it finishes."
                    .to_string(),
            );
            return;
        }
        clear_first_run(&app);
        record_sweep_date(&app);
        let mut message = format!(
            "Sweep complete - processed: {}, skipped: {}, deferred: {}",
            result.processed, result.skipped, result.deferred,
        );
        if !result.errors.is_empty() {
            message.push_str(&format!(", errors: {}", result.errors.len()));
        }
        if result.flagged > 0 {
            message.push_str(&format!(", possible secrets flagged: {}", result.flagged));
        }
        let _ = app.emit("sweep-done", message);
    });
    Ok(())
}

/// Signal the active sweep to stop after the current session finishes.
#[tauri::command]
pub(crate) fn cancel_sweep(app: tauri::AppHandle) {
    app.state::<SweepCancel>().0.store(true, Ordering::Relaxed);
}
