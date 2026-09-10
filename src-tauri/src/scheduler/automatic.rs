// HalluScribe - pure automatic-sweep retry admission and state transitions.

use crate::settings::{parse_schedule_time, HalluScribeSettings};
use chrono::{DateTime, FixedOffset, Local, Timelike};

const RETRY_DELAY_SECS: i64 = 60 * 60;
const MAX_RETRIES: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutomaticAdmission {
    NotDue,
    WaitingForRetry,
    AlreadyComplete,
    Exhausted,
    Exhausting,
    AttemptInitial,
    AttemptRetry,
}

/// Decide whether the scheduler may consume an automatic-sweep attempt at
/// `now`. The input clock makes the policy independently testable; persistence
/// happens only after an attempt-producing outcome has been chosen.
pub(crate) fn admit(
    settings: &HalluScribeSettings,
    now: DateTime<FixedOffset>,
) -> AutomaticAdmission {
    let today = now.format("%Y-%m-%d").to_string();
    let Some((hour, minute)) = parse_schedule_time(&settings.schedule_time) else {
        return AutomaticAdmission::NotDue;
    };
    if settings.last_sweep_date == today {
        return AutomaticAdmission::AlreadyComplete;
    }
    if (now.hour(), now.minute()) < (hour, minute) {
        return AutomaticAdmission::NotDue;
    }
    if settings.auto_sweep_exhausted_date == today {
        return AutomaticAdmission::Exhausted;
    }
    let previous = chrono::DateTime::parse_from_rfc3339(&settings.last_auto_sweep_attempt_at);
    let same_day = previous
        .as_ref()
        .map(|value| {
            value
                .with_timezone(now.offset())
                .format("%Y-%m-%d")
                .to_string()
                == today
        })
        .unwrap_or(false);
    if !same_day {
        return AutomaticAdmission::AttemptInitial;
    }
    if settings.auto_sweep_retry_count >= MAX_RETRIES {
        return AutomaticAdmission::Exhausting;
    }
    let elapsed = previous
        .ok()
        .map(|value| now.signed_duration_since(value).num_seconds());
    if elapsed.is_some_and(|seconds| seconds >= RETRY_DELAY_SECS) {
        AutomaticAdmission::AttemptRetry
    } else {
        AutomaticAdmission::WaitingForRetry
    }
}

/// Persist an admitted attempt before it competes for the inference token.
pub(crate) fn record_attempt(
    settings: &mut HalluScribeSettings,
    now: DateTime<FixedOffset>,
    admission: AutomaticAdmission,
) {
    match admission {
        AutomaticAdmission::AttemptInitial => settings.auto_sweep_retry_count = 0,
        AutomaticAdmission::AttemptRetry => {
            settings.auto_sweep_retry_count = settings.auto_sweep_retry_count.saturating_add(1)
        }
        _ => return,
    }
    settings.last_auto_sweep_attempt_at = now.to_rfc3339();
    settings.auto_sweep_exhausted_date.clear();
}

pub(crate) fn record_exhaustion(settings: &mut HalluScribeSettings, now: DateTime<FixedOffset>) {
    settings.auto_sweep_exhausted_date = now.format("%Y-%m-%d").to_string();
}

pub(crate) fn record_success(settings: &mut HalluScribeSettings, now: DateTime<FixedOffset>) {
    settings.first_run = false;
    settings.last_sweep_date = now.format("%Y-%m-%d").to_string();
    settings.last_auto_sweep_attempt_at.clear();
    settings.auto_sweep_retry_count = 0;
    settings.auto_sweep_exhausted_date.clear();
}

pub(crate) fn now_fixed() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(value: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(value).unwrap()
    }

    fn settings() -> HalluScribeSettings {
        HalluScribeSettings {
            schedule_time: "02:00".into(),
            ..Default::default()
        }
    }

    #[test]
    fn first_attempt_and_hourly_retry_are_admitted() {
        let mut state = settings();
        let first = clock("2026-09-10T02:00:00+03:00");
        assert_eq!(admit(&state, first), AutomaticAdmission::AttemptInitial);
        record_attempt(&mut state, first, AutomaticAdmission::AttemptInitial);
        assert_eq!(
            admit(&state, clock("2026-09-10T02:59:59+03:00")),
            AutomaticAdmission::WaitingForRetry
        );
        assert_eq!(
            admit(&state, clock("2026-09-10T03:00:00+03:00")),
            AutomaticAdmission::AttemptRetry
        );
    }

    #[test]
    fn fourth_attempt_is_last_and_next_candidate_exhausts() {
        let mut state = settings();
        let first = clock("2026-09-10T02:00:00+03:00");
        record_attempt(&mut state, first, AutomaticAdmission::AttemptInitial);
        for hour in [3, 4, 5] {
            let now = clock(&format!("2026-09-10T{hour:02}:00:00+03:00"));
            assert_eq!(admit(&state, now), AutomaticAdmission::AttemptRetry);
            record_attempt(&mut state, now, AutomaticAdmission::AttemptRetry);
        }
        let exhausted = clock("2026-09-10T06:00:00+03:00");
        assert_eq!(admit(&state, exhausted), AutomaticAdmission::Exhausting);
        record_exhaustion(&mut state, exhausted);
        assert_eq!(admit(&state, exhausted), AutomaticAdmission::Exhausted);
    }

    #[test]
    fn success_resets_retry_state_and_completes_the_day() {
        let mut state = settings();
        state.auto_sweep_retry_count = 2;
        state.last_auto_sweep_attempt_at = "2026-09-10T04:00:00+03:00".into();
        let now = clock("2026-09-10T05:00:00+03:00");
        record_success(&mut state, now);
        assert_eq!(state.auto_sweep_retry_count, 0);
        assert!(state.last_auto_sweep_attempt_at.is_empty());
        assert_eq!(admit(&state, now), AutomaticAdmission::AlreadyComplete);
    }
}
