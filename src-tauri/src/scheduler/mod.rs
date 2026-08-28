// HalluScribe - nightly sweep scheduler.
// Scans recent JSONL sessions, deduplicates against the archive, and runs
// sequential Gemma inference for each new session.
// force=true bypasses the time-window check ("Run Now" user action).
// No concurrent inference - one session at a time, always.

mod eligibility;
mod helpers;
mod runner;
mod tests;
mod types;

pub(crate) use helpers::is_sweep_due;
pub use helpers::sweep_done_message;
pub use runner::run_sweep;
pub use types::{SweepConfig, SweepProgress, SweepResult};
