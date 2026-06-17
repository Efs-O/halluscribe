use crate::gemma::InferenceBackend;
use crate::readers::ChatProvider;
use crate::settings::parse_schedule_time;

/// Returns true when a sweep is due: it has not already run today and the
/// current local time is at or after the scheduled time. This is a catch-up
/// model - if the app was closed, asleep, or busy during the scheduled minute,
/// the sweep still runs the next time the loop ticks that day, instead of being
/// missed until tomorrow (audit A-2). `today` and `last_sweep_date` are
/// `YYYY-MM-DD` local dates. Extracted for unit-testability (no real clock).
pub(crate) fn is_sweep_due(
    current_hour: u32,
    current_minute: u32,
    today: &str,
    schedule_time: &str,
    last_sweep_date: &str,
) -> bool {
    let Some((target_hour, target_minute)) = parse_schedule_time(schedule_time) else {
        return false;
    };
    if last_sweep_date == today {
        return false;
    }
    (current_hour, current_minute) >= (target_hour, target_minute)
}

/// Map InferenceBackend to the short label stored in archive metadata.
pub(super) fn backend_display_name(backend: &InferenceBackend) -> String {
    match backend {
        InferenceBackend::LlamaCpp { .. } => "llama.cpp",
        InferenceBackend::Ollama { .. } => "Ollama",
    }
    .to_string()
}

pub(super) fn provider_display_name(provider: &ChatProvider) -> String {
    provider.display_name().to_string()
}
