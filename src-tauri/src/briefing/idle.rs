// HalluScribe - idle-timeout watchdog that unloads the warm briefing server.
// Interactive chat keeps one llama-server warm across turns for responsiveness;
// this watchdog bounds that warmth so a model is never parked on the GPU
// indefinitely (audit A-1). It only unloads when it can take the process-wide
// inference lock, guaranteeing it never kills a server mid-response.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Unload the warm server after this many seconds without a briefing/chat turn.
const IDLE_TIMEOUT_SECS: u64 = 300;
/// How often the watchdog wakes to check the idle elapsed time.
const POLL_INTERVAL: Duration = Duration::from_secs(30);

fn last_activity() -> &'static AtomicU64 {
    static LAST: OnceLock<AtomicU64> = OnceLock::new();
    LAST.get_or_init(|| AtomicU64::new(0))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Record briefing/chat activity and ensure the idle watchdog is running.
/// Call at the start and end of every interactive turn so the idle clock is
/// measured from the last moment the user was actually being served.
pub(crate) fn mark_active() {
    last_activity().store(now_secs(), Ordering::Relaxed);
    ensure_watchdog();
}

fn ensure_watchdog() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let _ = thread::Builder::new()
            .name("briefing-idle-watchdog".into())
            .spawn(watchdog_loop);
    });
}

fn watchdog_loop() {
    loop {
        thread::sleep(POLL_INTERVAL);
        let last = last_activity().load(Ordering::Relaxed);
        // 0 means "already unloaded / never active" - nothing to do.
        if last == 0 || now_secs().saturating_sub(last) < IDLE_TIMEOUT_SECS {
            continue;
        }
        // Only unload when no inference job is active, so we never kill a server
        // mid-response. If the lock is held, retry on the next tick.
        if let Some(_guard) = crate::infer_lock::try_acquire() {
            super::kill_server();
            last_activity().store(0, Ordering::Relaxed);
        }
    }
}
