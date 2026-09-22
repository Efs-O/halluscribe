// HalluScribe - scheduler helpers shared by the manual and scheduled sweeps.

use super::types::SweepResult;
use crate::gemma::InferenceBackend;
use crate::readers::ChatProvider;
use crate::settings::parse_schedule_time;

/// Build the human-readable `sweep-done` message for a finished sweep.
///
/// Both sweep entry points report through this one function: the manual
/// "Run Now" command (`commands::sweep`) and the nightly scheduled tick
/// (`lib.rs`). They previously built the same message independently, which made
/// every change to sweep reporting a chance to update only one of them — and
/// the one easier to forget is the scheduled sweep, the unattended one whose
/// message the user reads after the fact rather than watching it happen.
///
/// `marker_errors` are failures to persist the daily success marker. They are
/// logged to stderr here rather than by the callers, so the two paths cannot
/// diverge on the diagnostic either.
pub fn sweep_done_message(result: &SweepResult, marker_errors: &[String]) -> String {
    let mut message = format!(
        "Sweep {} - processed: {}, skipped: {}, deferred: {}",
        if result.completed_successfully() {
            "complete"
        } else {
            "incomplete"
        },
        result.processed,
        result.skipped,
        result.deferred,
    );
    if result.cancelled {
        message.push_str(", cancelled");
    }
    if result.low_signal_skipped > 0 {
        message.push_str(&format!(
            ", low-signal skipped: {} (greetings or unsupported capability probes)",
            result.low_signal_skipped
        ));
    }
    if result.business_filtered > 0 {
        message.push_str(&format!(
            ", phone chats skipped: {} (you never replied, or under 3 messages)",
            result.business_filtered
        ));
    }
    if !result.errors.is_empty() {
        message.push_str(&format!(", errors: {}", result.errors.len()));
    }
    if !marker_errors.is_empty() {
        message.push_str(&format!(", settings errors: {}", marker_errors.len()));
        eprintln!(
            "[sweep] failed to persist completion state: {}",
            marker_errors.join("; ")
        );
    }
    if result.flagged > 0 {
        message.push_str(&format!(", possible secrets flagged: {}", result.flagged));
    }
    if result.superseded > 0 {
        message.push_str(&format!(
            ", earlier raw copies kept in raw/superseded: {}",
            result.superseded
        ));
    }
    message
}

/// Returns true when a sweep has not completed today and the current local time
/// is at or after the scheduled time. The application-level scheduler adds the
/// separate persisted one-attempt-per-day guard before calling this runner.
/// `today` and `last_sweep_date` are `YYYY-MM-DD` local dates. Extracted for
/// unit-testability (no real clock).
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

/// Display the actual configured model identity in archive provenance.
pub(super) fn model_display_name(backend: &InferenceBackend) -> String {
    match backend {
        InferenceBackend::LlamaCpp { model, .. } => model
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("llama.cpp model")
            .to_string(),
        InferenceBackend::Ollama { model, .. } => model.clone(),
    }
}

pub(super) fn provider_display_name(provider: &ChatProvider) -> String {
    provider.display_name().to_string()
}
