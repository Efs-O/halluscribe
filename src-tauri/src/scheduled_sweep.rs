// HalluScribe - persisted once-daily automatic sweep runner.
//
// The loop checks the clock frequently for catch-up scheduling, but records an
// attempt before inference so a failed or busy run cannot become a retry loop.

use crate::app_state::SweepCancel;
use crate::app_support::{mark_auto_sweep_attempt, mark_sweep_success, sweep_config};
use crate::scheduler;
use chrono::{Datelike, Local, Timelike};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};

/// Start the app-lifetime scheduler thread. Automatic attempts are persisted
/// once per local day; the manual command remains available for explicit retry.
pub(crate) fn start(handle: AppHandle) {
    std::thread::spawn(move || {
        let mut last_checked_minute: Option<(i32, u32, u32, u32, u32)> = None;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let now = Local::now();
            let current_minute = (now.year(), now.month(), now.day(), now.hour(), now.minute());
            if last_checked_minute == Some(current_minute) {
                continue;
            }
            last_checked_minute = Some(current_minute);

            let config = match sweep_config(&handle, false) {
                Ok(config) => config,
                Err(error) => {
                    eprintln!("[scheduler] could not load sweep configuration: {error}");
                    continue;
                }
            };
            let Some(config) = config else {
                continue;
            };
            if let Err(error) = mark_auto_sweep_attempt(&handle) {
                eprintln!("[scheduler] could not record automatic sweep attempt: {error}");
                continue;
            }

            // Use the shared cancel flag so Cancel stops automatic work too.
            let cancel = handle.state::<SweepCancel>().0.clone();
            cancel.store(false, Ordering::Relaxed);
            let result = scheduler::run_sweep(&handle, &config, cancel);
            if result.ran {
                let marker_errors = if result.completed_successfully() {
                    mark_sweep_success(&handle)
                        .err()
                        .into_iter()
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let message = scheduler::sweep_done_message(&result, &marker_errors);
                let _ = handle.emit("sweep-done", message);
            }
        }
    });
}
