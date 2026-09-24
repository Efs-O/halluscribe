// HalluScribe - pure automatic-sweep retry admission and state transitions.

use crate::settings::{parse_schedule_time, HalluScribeSettings};
use chrono::{DateTime, Duration, FixedOffset, Local};

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

/// Start of the schedule cycle `now` falls in: today's scheduled time once it
/// has passed, otherwise yesterday's. Retries belong to a cycle rather than a
/// calendar date, so a late schedule (say 23:30) can still retry after midnight.
fn cycle_start(
    settings: &HalluScribeSettings,
    now: DateTime<FixedOffset>,
) -> Option<DateTime<FixedOffset>> {
    let (hour, minute) = parse_schedule_time(&settings.schedule_time)?;
    let today = now
        .date_naive()
        .and_hms_opt(hour, minute, 0)?
        .and_local_timezone(*now.offset())
        .single()?;
    Some(if now >= today {
        today
    } else {
        today - Duration::days(1)
    })
}

/// The local date (`YYYY-MM-DD`) of the schedule cycle `now` falls in.
fn cycle_day(settings: &HalluScribeSettings, now: DateTime<FixedOffset>) -> Option<String> {
    cycle_start(settings, now).map(|start| start.format("%Y-%m-%d").to_string())
}

/// Decide whether the scheduler may consume an automatic-sweep attempt at
/// `now`. The input clock makes the policy independently testable; persistence
/// happens only after an attempt-producing outcome has been chosen.
pub(crate) fn admit(
    settings: &HalluScribeSettings,
    now: DateTime<FixedOffset>,
) -> AutomaticAdmission {
    let Some(cycle) = cycle_start(settings, now) else {
        return AutomaticAdmission::NotDue;
    };
    let day = cycle.format("%Y-%m-%d").to_string();
    // Any successful sweep dated on or after the cycle's day covers it: a
    // manual sweep earlier that day has always counted as that day's sweep.
    if settings.last_sweep_date.as_str() >= day.as_str() {
        return AutomaticAdmission::AlreadyComplete;
    }
    let previous = DateTime::parse_from_rfc3339(&settings.last_auto_sweep_attempt_at).ok();
    let attempted_this_cycle = previous.is_some_and(|value| value >= cycle);
    // Before today's scheduled time only an unfinished cycle from yesterday
    // may continue; a cycle that never started waits for its own time.
    if cycle.date_naive() != now.date_naive() && !attempted_this_cycle {
        return AutomaticAdmission::NotDue;
    }
    if settings.auto_sweep_exhausted_date == day {
        return AutomaticAdmission::Exhausted;
    }
    let Some(previous) = previous.filter(|_| attempted_this_cycle) else {
        return AutomaticAdmission::AttemptInitial;
    };
    if settings.auto_sweep_retry_count >= MAX_RETRIES {
        return AutomaticAdmission::Exhausting;
    }
    if now.signed_duration_since(previous).num_seconds() >= RETRY_DELAY_SECS {
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

/// The retry state an attempt overwrites. An attempt that never ran because
/// another job held the sweep or the model puts this back, so being busy does
/// not spend the day's retry budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttemptState {
    last_attempt_at: String,
    retry_count: u8,
    exhausted_date: String,
}

impl AttemptState {
    pub(crate) fn of(settings: &HalluScribeSettings) -> Self {
        Self {
            last_attempt_at: settings.last_auto_sweep_attempt_at.clone(),
            retry_count: settings.auto_sweep_retry_count,
            exhausted_date: settings.auto_sweep_exhausted_date.clone(),
        }
    }

    pub(crate) fn restore(self, settings: &mut HalluScribeSettings) {
        settings.last_auto_sweep_attempt_at = self.last_attempt_at;
        settings.auto_sweep_retry_count = self.retry_count;
        settings.auto_sweep_exhausted_date = self.exhausted_date;
    }
}

pub(crate) fn record_exhaustion(settings: &mut HalluScribeSettings, now: DateTime<FixedOffset>) {
    settings.auto_sweep_exhausted_date =
        cycle_day(settings, now).unwrap_or_else(|| now.format("%Y-%m-%d").to_string());
}

/// Mark the cycle complete. The cycle's own date is recorded, not today's: a
/// 23:30 cycle that succeeds at 00:30 must not also mark the new day done.
pub(crate) fn record_success(settings: &mut HalluScribeSettings, now: DateTime<FixedOffset>) {
    let day = cycle_day(settings, now).unwrap_or_else(|| now.format("%Y-%m-%d").to_string());
    settings.first_run = false;
    if day > settings.last_sweep_date {
        settings.last_sweep_date = day;
    }
    settings.last_auto_sweep_attempt_at.clear();
    settings.auto_sweep_retry_count = 0;
    settings.auto_sweep_exhausted_date.clear();
}

pub(crate) fn now_fixed() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}
