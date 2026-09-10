// HalluScribe - process-wide guard that serialises all model inference.
// CLAUDE.md hard rule: "Process jobs sequentially - never concurrent Gemma
// inference." Every entry point that loads a model (the sweep, briefing, chat,
// and the embedding runtime) takes this single exclusive token for its whole
// duration so two llama-server / Ollama jobs can never run at once.
//
// Non-reentrant: acquire only at top-level entry points, never inside a path
// that is already running under a held guard (e.g. the sweep already holds the
// guard while it calls retrieval::index_session, so embedding code must not
// re-acquire it).

use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};

fn inference_mutex() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Exclusive token proving the holder is the only active inference job.
/// Inference access is released when this value is dropped.
pub struct InferenceGuard(#[allow(dead_code)] MutexGuard<'static, ()>);

/// Try to take exclusive inference access without blocking.
///
/// Returns `None` if another job (sweep, briefing, chat, or embedding) is
/// already running. A poisoned lock (a previous holder panicked) is recovered
/// rather than propagated, so a single crashed job cannot wedge inference for
/// the rest of the process lifetime.
pub fn try_acquire() -> Option<InferenceGuard> {
    match inference_mutex().try_lock() {
        Ok(guard) => Some(InferenceGuard(guard)),
        Err(TryLockError::WouldBlock) => None,
        Err(TryLockError::Poisoned(poison)) => {
            reconcile_after_panic();
            inference_mutex().clear_poison();
            Some(InferenceGuard(poison.into_inner()))
        }
    }
}

/// A poisoned mutex proves an inference holder panicked. Reclaim every model
/// process owned by this app before handing its recovered guard to new work.
fn reconcile_after_panic() {
    #[cfg(test)]
    RECOVERY_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::briefing::kill_server();
    crate::retrieval::kill_embedding_server();
    crate::llama_pids::reap_orphans();
    crate::llama_runtime::record_diagnostic(
        "inference-lock",
        "recovered poisoned lock; reclaimed managed inference processes",
    );
}

#[cfg(test)]
static RECOVERY_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Take exclusive inference access for a *batch* job (the sweep, a profile
/// refresh) and reclaim the warm interactive server before the caller loads a
/// model of its own. Returns `None` for the same reasons `try_acquire` does.
///
/// Interactive chat deliberately keeps one llama-server warm across turns, and
/// `briefing::idle`'s watchdog can only reclaim it during a moment when this
/// lock is free. A long batch job therefore starves that watchdog for its
/// entire duration. Observed live 2026-08-04: an idle 26B chat server sat on
/// the GPU for ~2 hours holding 10.3 GB, and because it still owned the
/// configured port the following sweep loaded a *second* full copy of the same
/// model beside it (~30 GB resident, both competing for the GPU).
///
/// Killing it here is safe precisely because the guard is already held: no
/// chat can be mid-stream, so this never cuts off a response — the same
/// invariant the idle watchdog relies on. A no-op when no warm server exists
/// (the Ollama backend bounds warmth with `keep_alive` instead).
pub fn acquire_for_batch() -> Option<InferenceGuard> {
    let guard = try_acquire()?;
    crate::briefing::kill_server();
    crate::retrieval::kill_embedding_server();
    Some(guard)
}

#[cfg(test)]
mod tests {
    use super::{acquire_for_batch, inference_mutex, try_acquire, RECOVERY_COUNT};
    use std::sync::atomic::Ordering;

    // Deliberately ONE test: the lock is process-wide, so a second #[test]
    // taking it would race this one under cargo's parallel test runner.
    #[test]
    fn second_acquire_is_blocked_while_first_is_held() {
        let first = try_acquire().expect("first acquire should succeed");
        assert!(
            try_acquire().is_none(),
            "a second concurrent acquire must be refused"
        );
        assert!(
            acquire_for_batch().is_none(),
            "a batch acquire must be refused while the lock is held"
        );
        drop(first);
        assert!(
            try_acquire().is_some(),
            "acquire should succeed again once the guard is dropped"
        );

        // The batch variant takes the same lock and releases it on drop; its
        // server reclaim is a no-op here since no warm server was ever spawned.
        let batch = acquire_for_batch().expect("batch acquire should succeed when free");
        assert!(
            try_acquire().is_none(),
            "a batch guard must exclude other jobs just like try_acquire"
        );
        drop(batch);
        assert!(
            try_acquire().is_some(),
            "acquire should succeed again once the batch guard is dropped"
        );

        RECOVERY_COUNT.store(0, Ordering::Relaxed);
        std::panic::catch_unwind(|| {
            let _guard = inference_mutex().lock().unwrap();
            panic!("intentional poison");
        })
        .expect_err("intentional poison must panic");
        let recovered = try_acquire().expect("poisoned lock recovers");
        assert_eq!(RECOVERY_COUNT.load(Ordering::Relaxed), 1);
        drop(recovered);
    }
}
