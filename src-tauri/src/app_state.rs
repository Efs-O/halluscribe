// HalluScribe - managed cancellation and startup-capture application state.

use crate::archive::CaptureStatus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A cancel signal handed out fresh for each run of a long-running job.
///
/// The previous design kept one shared `AtomicBool` per job and `store(false)`
/// reset it at the start of every trigger. That reset raced with a prior
/// instance of the same job that was still reading the flag, so an in-flight
/// Cancel could be silently cleared, and (for the sweep) the daily scheduler
/// and a manual *Run Now* clobbered each other's cancel.
///
/// `begin_run` instead installs a *brand-new* flag and returns it, so a run
/// only ever observes Cancels issued while it is the current run. A cancel
/// arriving after a newer run started targets that newer flag, never an
/// already-running one. This makes the reset-before-spawn race structurally
/// impossible without adding a busy guard.
pub(crate) struct CancelState {
    current: Mutex<Option<Arc<AtomicBool>>>,
}

impl CancelState {
    pub(crate) fn new() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }

    /// Start a new run: install a fresh, un-cancelled flag and return it. The
    /// caller moves this flag into its job thread; it is the *only* handle by
    /// which that run can be cancelled.
    pub(crate) fn try_begin_run(&self) -> Option<Arc<AtomicBool>> {
        let token = Arc::new(AtomicBool::new(false));
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if current.is_some() {
            return None;
        }
        *current = Some(token.clone());
        Some(token)
    }

    /// Mark `token`'s run complete. A stale worker must never clear a newer
    /// run, so identity rather than the flag value determines ownership.
    pub(crate) fn finish_run(&self, token: &Arc<AtomicBool>) {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if current
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, token))
        {
            *current = None;
        }
    }

    /// Signal the current run (if any) to stop. A flag captured by an
    /// already-finished or superseded run is untouched, so a late Cancel can
    /// never abort a run that had not started yet.
    pub(crate) fn request_cancel(&self) {
        let token = self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .cloned();
        if let Some(token) = token {
            token.store(true, Ordering::Relaxed);
        }
    }
}

// Each job keeps its own `CancelState` so cancelling one job never touches
// another. The shared logic lives in `CancelState`; these are distinct types
// purely so a command cancels exactly the job it names.
macro_rules! cancel_state_types {
    ($($(#[doc = $doc:expr])* $name:ident),* $(,)?) => {
        $(
            $(#[doc = $doc])*
            pub(crate) struct $name(pub(crate) CancelState);
        )*
    };
}

cancel_state_types! {
    /// Cancel signal for the briefing stream. `cancel_briefing` requests a
    /// stop; `run_briefing` claims a fresh flag per run via `begin_run`.
    BriefingCancel,
    /// Cancel signal for the sweep. `cancel_sweep` requests a stop; both the
    /// manual command and the daily scheduler claim their own fresh flag per
    /// run, so neither can clear the other's cancel.
    SweepCancel,
    /// Cancel signal for a profile refresh / full rebuild. One flag serves both
    /// scopes: only one profile job can run at a time (the inference lock is
    /// global), so a per-scope flag would be dead weight -
    /// `ProfileDonePayload::scope` says which one stopped.
    ProfileCancel,
    /// Cancel signal for chat turns. `cancel_chat` requests a stop;
    /// `send_chat_message` claims a fresh flag per turn via `begin_run`.
    ChatCancel,
    /// Cancel signal for the startup raw capture pass. `cancel_capture` requests
    /// a stop; capture runs once per launch, so its flag is claimed once.
    CaptureCancel,
}

/// Latest known status of the startup raw capture pass, updated from the
/// background capture thread and read by the `get_capture_status` command so
/// a remounted status line can rediscover an in-flight or finished pass.
pub(crate) struct CaptureStatusState(pub(crate) Arc<Mutex<CaptureStatus>>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_completed_run_allows_a_fresh_uncancelled_token() {
        let state = CancelState::new();
        let first = state.try_begin_run().unwrap();
        state.request_cancel();
        assert!(first.load(Ordering::Relaxed));

        state.finish_run(&first);
        let second = state.try_begin_run().unwrap();
        assert!(!second.load(Ordering::Relaxed));
        assert!(first.load(Ordering::Relaxed));
    }

    #[test]
    fn a_second_run_is_rejected_until_the_current_one_finishes() {
        let state = CancelState::new();
        let first = state.try_begin_run().unwrap();
        assert!(state.try_begin_run().is_none());

        state.request_cancel();
        assert!(first.load(Ordering::Relaxed));
    }
}
