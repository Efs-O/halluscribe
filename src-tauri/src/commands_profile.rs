// HalluScribe - Tauri command handlers for the profile distiller (Persona
// Protocol Phase 2): trigger a refresh, read the profile, read the latest
// weekly digest.

use crate::app_support::archive_dir;
use crate::{gemma, profile, settings};
use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;

#[derive(Debug, Clone, Serialize)]
struct ProfileProgressPayload {
    current: usize,
    total: usize,
    stage: String,
}

#[derive(Debug, Clone, Default, Serialize)]
struct ProfileDonePayload {
    busy: bool,
    session_count: usize,
    facts_count: usize,
    errors: Vec<String>,
}

/// Run a profile refresh in a background thread. `full=true` ignores the
/// watermark (complete rebuild); `false` distills only sessions newer than
/// `profile_meta.json`'s watermark. Emits `profile-progress` events
/// `{ current, total, stage }` while running and one `profile-done` event
/// `{ busy, session_count, facts_count, errors }` when finished.
#[tauri::command]
pub(crate) fn run_profile_refresh(app: tauri::AppHandle, full: bool) -> Result<(), String> {
    let dir = archive_dir(&app)?;
    let settings = settings::load_settings(&dir);
    let backend = settings
        .to_inference_backend()
        .ok_or_else(|| "backend not configured (check Settings)".to_string())?;
    let (ctx_size, max_tokens) = settings.generation_limits()?;
    let profile_sources = settings.profile_sources.clone();

    std::thread::spawn(move || {
        // Top-level inference lock, exactly like the sweep: the profile code
        // below never re-acquires it (the guard is non-reentrant).
        let Some(_inference_guard) = crate::infer_lock::try_acquire() else {
            let _ = app.emit(
                "profile-done",
                ProfileDonePayload {
                    busy: true,
                    ..Default::default()
                },
            );
            return;
        };

        // Skip the model load entirely when an incremental run has nothing new
        // to distill (checked under the lock so a concurrent sweep can't be
        // mid-write while we count).
        if profile::pending_session_count(&dir, &profile_sources, full) == 0 {
            let _ = app.emit("profile-done", ProfileDonePayload::default());
            return;
        }

        // One warm model serves every map batch plus the reduce call, then is
        // unloaded when `session` drops at the end (sweep unload policy).
        let session = match gemma::start_tool_session(&backend, ctx_size) {
            Ok(session) => session,
            Err(error) => {
                let _ = app.emit(
                    "profile-done",
                    ProfileDonePayload {
                        errors: vec![format!("failed to start model: {error}")],
                        ..Default::default()
                    },
                );
                return;
            }
        };

        let tool_call = |system_prompt: &str,
                         user_content: &str,
                         tool: &Value,
                         call_max_tokens: u32|
         -> Result<Value, profile::ProfileError> {
            session
                .call_tool(
                    system_prompt,
                    user_content,
                    tool,
                    call_max_tokens.min(max_tokens),
                )
                .map_err(profile::ProfileError::from)
        };

        let app_progress = app.clone();
        let result = profile::run_refresh(
            &dir,
            &profile_sources,
            full,
            &tool_call,
            move |current, total, stage| {
                let _ = app_progress.emit(
                    "profile-progress",
                    ProfileProgressPayload {
                        current,
                        total,
                        stage: stage.as_str().to_string(),
                    },
                );
            },
        );
        drop(session);

        let payload = match result {
            Ok(outcome) => ProfileDonePayload {
                busy: false,
                session_count: outcome.session_count,
                facts_count: outcome.facts_count,
                errors: outcome.errors,
            },
            Err(error) => ProfileDonePayload {
                errors: vec![error.to_string()],
                ..Default::default()
            },
        };
        let _ = app.emit("profile-done", payload);
    });
    Ok(())
}

/// Return the distilled profile.md content, or None if no profile exists yet.
#[tauri::command]
pub(crate) fn get_profile(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let dir = archive_dir(&app)?;
    Ok(profile::read_profile_md(&dir))
}

/// Return the newest weekly digest content, or None if none exists yet.
#[tauri::command]
pub(crate) fn get_latest_digest(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let dir = archive_dir(&app)?;
    Ok(profile::latest_digest(&dir))
}
