use std::sync::{atomic::AtomicBool, Arc};

/// Shared cancel flag for the briefing stream. Set to true by `cancel_briefing`,
/// reset to false at the start of each `run_briefing` call.
pub(crate) struct BriefingCancel(pub(crate) Arc<AtomicBool>);

/// Shared cancel flag for the sweep. Set to true by `cancel_sweep`,
/// reset to false at the start of each `trigger_sweep` call.
pub(crate) struct SweepCancel(pub(crate) Arc<AtomicBool>);

/// Shared cancel flag for chat turns. Set to true by `cancel_chat`,
/// reset to false at the start of each `send_chat_message` call.
pub(crate) struct ChatCancel(pub(crate) Arc<AtomicBool>);
