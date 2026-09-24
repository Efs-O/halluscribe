// HalluScribe - persisted once-daily automatic sweep runner.
//
// The loop checks the clock frequently for catch-up scheduling, but records an
// attempt before inference so a failed run cannot become a retry loop. A run
// that finds the sweep or the model busy gives its attempt back and waits
// `BUSY_RETRY_MINUTES` in memory instead of spending the day's retry budget.

use crate::app_state::SweepCancel;
use crate::app_support::{
    automatic_sweep_admission, mark_auto_sweep_attempt, mark_auto_sweep_exhausted,
    mark_auto_sweep_success, release_auto_sweep_attempt, sweep_config,
};
use crate::scheduler;
use chrono::{Datelike, Local, Timelike};
use tauri::{AppHandle, Emitter, Manager};

const BUSY_RETRY_MINUTES: i64 = 15;

/// Give back an attempt that never ran and wait before trying again. Nothing
/// is emitted: `sweep-done` would tell the UI that a running manual sweep has
/// finished.
fn postpone(
    handle: &AppHandle,
    previous: scheduler::AttemptState,
    busy_until: &mut Option<chrono::DateTime<Local>>,
    reason: &str,
) {
    if let Err(error) = release_auto_sweep_attempt(handle, previous) {
        eprintln!("[scheduler] could not give back the busy automatic attempt: {error}");
    }
    *busy_until = Some(Local::now() + chrono::Duration::minutes(BUSY_RETRY_MINUTES));
    eprintln!("[scheduler] automatic sweep postponed {BUSY_RETRY_MINUTES} minutes: {reason}");
}

/// Start the app-lifetime scheduler thread. Automatic attempts are persisted
/// once per local day; the manual command remains available for explicit retry.
pub(crate) fn start(handle: AppHandle) {
    std::thread::spawn(move || {
        let mut last_checked_minute: Option<(i32, u32, u32, u32, u32)> = None;
        let mut busy_until: Option<chrono::DateTime<Local>> = None;
        // The configuration problem last reported, so it is shown once rather
        // than every minute, and again only once it changes.
        let mut reported_config_error: Option<String> = None;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let now = Local::now();
            let current_minute = (now.year(), now.month(), now.day(), now.hour(), now.minute());
            if last_checked_minute == Some(current_minute) {
                continue;
            }
            last_checked_minute = Some(current_minute);
            if busy_until.is_some_and(|until| now < until) {
                continue;
            }

            let admission = match automatic_sweep_admission(&handle) {
                Ok(admission) => admission,
                Err(error) => {
                    eprintln!("[scheduler] could not load automatic sweep state: {error}");
                    continue;
                }
            };
            match admission {
                scheduler::AutomaticAdmission::Exhausting => {
                    if let Err(error) = mark_auto_sweep_exhausted(&handle) {
                        eprintln!(
                            "[scheduler] could not persist automatic sweep exhaustion: {error}"
                        );
                    } else {
                        let _ = handle.emit(
                            "sweep-done",
                            "Sweep incomplete - automatic retry budget exhausted for today",
                        );
                    }
                    continue;
                }
                scheduler::AutomaticAdmission::AttemptInitial
                | scheduler::AutomaticAdmission::AttemptRetry => {}
                _ => continue,
            }
            let config = match sweep_config(&handle, false) {
                Ok(config) => {
                    reported_config_error = None;
                    config
                }
                Err(error) => {
                    if reported_config_error.as_deref() != Some(error.as_str()) {
                        eprintln!("[scheduler] scheduled sweep skipped: {error}");
                        let _ =
                            handle.emit("sweep-done", format!("Scheduled sweep skipped - {error}"));
                        reported_config_error = Some(error);
                    }
                    continue;
                }
            };
            // Scheduled processing is switched off.
            let Some(config) = config else {
                continue;
            };
            let previous = match mark_auto_sweep_attempt(&handle) {
                Ok(previous) => previous,
                Err(error) => {
                    eprintln!("[scheduler] could not record automatic sweep attempt: {error}");
                    continue;
                }
            };

            // Claim a fresh cancel flag so Cancel stops this automatic run.
            // It is not shared with a concurrent manual sweep, so neither can
            // clear the other's cancel (the old reset-before-spawn race).
            let Some(cancel) = handle.state::<SweepCancel>().0.try_begin_run() else {
                postpone(
                    &handle,
                    previous,
                    &mut busy_until,
                    "a sweep is already running",
                );
                continue;
            };
            // A panic inside the sweep must still release the run slot and
            // report, or every later sweep is refused as "busy" until restart.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                scheduler::run_sweep(&handle, &config, cancel.clone())
            }));
            let result = match outcome {
                Ok(result) => result,
                Err(payload) => {
                    let message =
                        format!("The sweep crashed: {}", scheduler::panic_message(&*payload));
                    eprintln!("[scheduler] {message}");
                    let _ = handle.emit("sweep-done", message);
                    handle.state::<SweepCancel>().0.finish_run(&cancel);
                    continue;
                }
            };
            if result.busy {
                handle.state::<SweepCancel>().0.finish_run(&cancel);
                postpone(
                    &handle,
                    previous,
                    &mut busy_until,
                    "another job is using the model",
                );
                continue;
            }
            busy_until = None;
            {
                let marker_errors = if result.completed_successfully() {
                    mark_auto_sweep_success(&handle)
                        .err()
                        .into_iter()
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let message = scheduler::sweep_done_message(&result, &marker_errors);
                let _ = handle.emit("sweep-done", message);
            }
            handle.state::<SweepCancel>().0.finish_run(&cancel);
        }
    });
}
