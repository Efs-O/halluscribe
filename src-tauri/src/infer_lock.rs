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
        Err(TryLockError::Poisoned(poison)) => Some(InferenceGuard(poison.into_inner())),
    }
}

#[cfg(test)]
mod tests {
    use super::try_acquire;

    #[test]
    fn second_acquire_is_blocked_while_first_is_held() {
        let first = try_acquire().expect("first acquire should succeed");
        assert!(
            try_acquire().is_none(),
            "a second concurrent acquire must be refused"
        );
        drop(first);
        assert!(
            try_acquire().is_some(),
            "acquire should succeed again once the guard is dropped"
        );
    }
}
