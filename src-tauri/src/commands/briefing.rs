// HalluScribe - briefing generation Tauri command handlers.

use crate::app_state::BriefingCancel;
use crate::app_support::archive_dir;
use crate::{archive, briefing, settings};
use tauri::{Emitter, Manager};

const BRIEFING_MIN_WORDS_PER_SESSION: usize = 200;
const BRIEFING_MAX_WORDS_PER_SESSION: usize = 300;
const BRIEFING_WARN_SESSION_COUNT: usize = 15;
const BRIEFING_CONTEXT_RISK_SESSION_COUNT: usize = 30;

/// Trigger the auto briefing stream. Emits "briefing-token" and "briefing-done" events.
#[tauri::command]
pub(crate) fn run_briefing(
    app: tauri::AppHandle,
    date_from: Option<String>,
    date_to: Option<String>,
    fill_min: Option<f64>,
    fill_max: Option<f64>,
    keyword: Option<String>,
    session_ids: Option<Vec<String>>,
) -> Result<(), String> {
    let dir = archive_dir(&app)?;
    archive::ensure_index_readable(&dir).map_err(|error| error.to_string())?;
    let settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    let backend = settings
        .to_inference_backend()
        .ok_or_else(|| "backend not configured (check Settings)".to_string())?;
    let (ctx_size, max_tokens) = settings.generation_limits()?;

    let filters = briefing::BriefingFilters {
        date_from: date_from
            .as_deref()
            .and_then(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()),
        date_to: date_to
            .as_deref()
            .and_then(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()),
        fill_min,
        fill_max,
        keyword,
    };

    let scope = match session_ids {
        Some(ids) if !ids.is_empty() => briefing::BriefingScope::SelectedSessionIds(ids),
        _ if filters.date_from.is_some()
            || filters.date_to.is_some()
            || filters.fill_min.is_some()
            || filters.fill_max.is_some()
            || filters
                .keyword
                .as_deref()
                .is_some_and(|text| !text.is_empty()) =>
        {
            briefing::BriefingScope::FilterBased(filters)
        }
        _ => briefing::BriefingScope::ArchiveWide,
    };

    let (content, header, session_count) = briefing::collect_briefing_sessions(&dir, &scope, 6)?;
    let min_word_limit = session_count * BRIEFING_MIN_WORDS_PER_SESSION;
    let max_word_limit = session_count * BRIEFING_MAX_WORDS_PER_SESSION;
    let warning = if session_count > BRIEFING_CONTEXT_RISK_SESSION_COUNT {
        Some(format!(
            "Selected {session_count} sessions. Context-size risk is high for larger scoped briefings and some detail may be dropped."
        ))
    } else if session_count > BRIEFING_WARN_SESSION_COUNT {
        Some(format!(
            "Selected {session_count} sessions. Briefing detail per session will be compressed to stay readable."
        ))
    } else {
        None
    };

    // A fresh cancel flag per run, so a stop pressed during a prior briefing
    // cannot abort this one and a second trigger cannot clear this run's
    // in-flight cancel.
    let cancel = app
        .state::<BriefingCancel>()
        .0
        .try_begin_run()
        .ok_or_else(|| "A briefing is already running.".to_string())?;
    let _ = app.emit("briefing-warning", warning);

    let app_clone = app.clone();
    std::thread::spawn(move || {
        briefing::run_briefing_stream(
            &app_clone,
            &backend,
            ctx_size,
            max_tokens,
            session_count,
            &content,
            &header,
            min_word_limit,
            max_word_limit,
            cancel.clone(),
        );
        app_clone.state::<BriefingCancel>().0.finish_run(&cancel);
    });
    Ok(())
}

/// Signal the active briefing stream to stop. Safe to call when no stream is running.
#[tauri::command]
pub(crate) fn cancel_briefing(app: tauri::AppHandle) {
    app.state::<BriefingCancel>().0.request_cancel();
}
