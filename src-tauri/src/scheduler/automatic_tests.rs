// HalluScribe - tests for automatic-sweep retry admission and state transitions.

use super::automatic::*;
use crate::settings::HalluScribeSettings;
use chrono::{DateTime, FixedOffset};

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

fn late_settings() -> HalluScribeSettings {
    HalluScribeSettings {
        schedule_time: "23:30".into(),
        ..Default::default()
    }
}

#[test]
fn a_late_cycle_keeps_retrying_after_midnight() {
    let mut state = late_settings();
    let first = clock("2026-09-10T23:30:00+03:00");
    assert_eq!(admit(&state, first), AutomaticAdmission::AttemptInitial);
    record_attempt(&mut state, first, AutomaticAdmission::AttemptInitial);

    let retry = clock("2026-09-11T00:30:00+03:00");
    assert_eq!(admit(&state, retry), AutomaticAdmission::AttemptRetry);
    record_attempt(&mut state, retry, AutomaticAdmission::AttemptRetry);

    // Success after midnight completes the 10th's cycle, not the 11th's.
    let done = clock("2026-09-11T00:45:00+03:00");
    record_success(&mut state, done);
    assert_eq!(state.last_sweep_date, "2026-09-10");
    assert_eq!(
        admit(&state, clock("2026-09-11T23:29:00+03:00")),
        AutomaticAdmission::AlreadyComplete
    );
    assert_eq!(
        admit(&state, clock("2026-09-11T23:30:00+03:00")),
        AutomaticAdmission::AttemptInitial
    );
}

#[test]
fn a_cycle_that_never_started_waits_for_its_own_time() {
    // Opening the app in the morning does not start yesterday's missed run.
    let state = late_settings();
    assert_eq!(
        admit(&state, clock("2026-09-11T09:00:00+03:00")),
        AutomaticAdmission::NotDue
    );
}

#[test]
fn exhaustion_after_midnight_belongs_to_the_late_cycle() {
    let mut state = late_settings();
    record_attempt(
        &mut state,
        clock("2026-09-10T23:30:00+03:00"),
        AutomaticAdmission::AttemptInitial,
    );
    for hour in [0, 1, 2] {
        let now = clock(&format!("2026-09-11T{hour:02}:30:00+03:00"));
        assert_eq!(admit(&state, now), AutomaticAdmission::AttemptRetry);
        record_attempt(&mut state, now, AutomaticAdmission::AttemptRetry);
    }
    let exhausted = clock("2026-09-11T03:30:00+03:00");
    assert_eq!(admit(&state, exhausted), AutomaticAdmission::Exhausting);
    record_exhaustion(&mut state, exhausted);
    assert_eq!(state.auto_sweep_exhausted_date, "2026-09-10");
    assert_eq!(
        admit(&state, clock("2026-09-11T12:00:00+03:00")),
        AutomaticAdmission::Exhausted
    );
    // The 11th's own cycle still gets its full budget.
    assert_eq!(
        admit(&state, clock("2026-09-11T23:30:00+03:00")),
        AutomaticAdmission::AttemptInitial
    );
}

#[test]
fn a_given_back_attempt_leaves_the_budget_untouched() {
    let mut state = settings();
    let first = clock("2026-09-10T02:00:00+03:00");
    let before = AttemptState::of(&state);
    record_attempt(&mut state, first, AutomaticAdmission::AttemptInitial);
    before.restore(&mut state);
    assert_eq!(admit(&state, first), AutomaticAdmission::AttemptInitial);

    record_attempt(&mut state, first, AutomaticAdmission::AttemptInitial);
    let retry = clock("2026-09-10T03:00:00+03:00");
    let before = AttemptState::of(&state);
    record_attempt(&mut state, retry, AutomaticAdmission::AttemptRetry);
    assert_eq!(state.auto_sweep_retry_count, 1);
    before.restore(&mut state);
    assert_eq!(state.auto_sweep_retry_count, 0);
    assert_eq!(admit(&state, retry), AutomaticAdmission::AttemptRetry);
}

#[test]
fn a_manual_sweep_earlier_that_day_covers_the_cycle() {
    let mut state = late_settings();
    state.last_sweep_date = "2026-09-10".into();
    assert_eq!(
        admit(&state, clock("2026-09-10T23:30:00+03:00")),
        AutomaticAdmission::AlreadyComplete
    );
}
