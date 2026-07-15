use crate::archive::CaptureStatus;
use std::sync::{atomic::AtomicBool, Arc, Mutex};

/// Shared cancel flag for the briefing stream. Set to true by `cancel_briefing`,
/// reset to false at the start of each `run_briefing` call.
pub(crate) struct BriefingCancel(pub(crate) Arc<AtomicBool>);

/// Shared cancel flag for the sweep. Set to true by `cancel_sweep`,
/// reset to false at the start of each `trigger_sweep` call.
pub(crate) struct SweepCancel(pub(crate) Arc<AtomicBool>);

/// Shared cancel flag for chat turns. Set to true by `cancel_chat`,
/// reset to false at the start of each `send_chat_message` call.
pub(crate) struct ChatCancel(pub(crate) Arc<AtomicBool>);

/// Shared cancel flag for the startup raw capture pass. Set to true by
/// `cancel_capture`; there is no "restart" call - capture only ever runs once
/// per launch, so this flag is never reset back to false.
pub(crate) struct CaptureCancel(pub(crate) Arc<AtomicBool>);

/// Latest known status of the startup raw capture pass, updated from the
/// background capture thread and read by the `get_capture_status` command so
/// a remounted status line can rediscover an in-flight or finished pass.
pub(crate) struct CaptureStatusState(pub(crate) Arc<Mutex<CaptureStatus>>);
