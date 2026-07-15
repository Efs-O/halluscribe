// HalluScribe - Tauri command handlers for the startup raw capture pass:
// read the latest status (for a remounted status line) and request an early
// stop (mirrors `cancel_sweep`).

use crate::app_state::{CaptureCancel, CaptureStatusState};
use crate::archive::CaptureStatus;
use std::sync::atomic::Ordering;
use tauri::Manager;

/// Return the most recent status of the startup capture pass. Used by the UI
/// on mount so a status line that wasn't present when the pass started can
/// still show its progress or final result.
#[tauri::command]
pub(crate) fn get_capture_status(app: tauri::AppHandle) -> CaptureStatus {
    let state = app.state::<CaptureStatusState>();
    let guard = state
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.clone()
}

/// Signal the running capture pass to stop after the current file finishes.
/// Safe to call when no pass is running.
#[tauri::command]
pub(crate) fn cancel_capture(app: tauri::AppHandle) {
    app.state::<CaptureCancel>()
        .0
        .store(true, Ordering::Relaxed);
}
