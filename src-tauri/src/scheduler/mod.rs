// HalluScribe - nightly sweep scheduler.
// Scans recent JSONL sessions, deduplicates against the archive, and runs
// sequential Gemma inference for each new session.
// When an automatic run is due is decided by `automatic` before a run starts.
// No concurrent inference - one session at a time, always.

mod eligibility;
mod helpers;
mod runner;
mod tests;
mod types;

pub(crate) use automatic::{
    admit, now_fixed, record_attempt, record_exhaustion, record_success, AttemptState,
    AutomaticAdmission,
};
pub use helpers::{panic_message, sweep_done_message};
pub use runner::run_sweep;
pub use types::{SweepConfig, SweepProgress, SweepResult};
mod automatic;
#[cfg(test)]
mod automatic_tests;
