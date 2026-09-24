// HalluScribe - persisted once-daily automatic sweep runner.
//
// The loop checks the clock frequently for catch-up scheduling, but records an
// attempt before inference so a failed or busy run cannot become a retry loop.

use crate::app_state::SweepCancel;
use crate::app_support::{
    automatic_sweep_admission, mark_auto_sweep_attempt, mark_auto_sweep_exhausted,
    mark_auto_sweep_success, sweep_config,
};
use crate::scheduler;
use chrono::{Datelike, Local, Timelike};
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

            // Claim a fresh cancel flag so Cancel stops this automatic run.
            // It is not shared with a concurrent manual sweep, so neither can
            // clear the other's cancel (the old reset-before-spawn race).
            let Some(cancel) = handle.state::<SweepCancel>().0.try_begin_run() else {
                let _ = handle.emit(
                    "sweep-done",
                    "Sweep incomplete - automatic attempt was busy",
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
