// HalluScribe - Tauri command handlers for the profile distiller (Persona
// Protocol Phase 2): trigger a refresh, read the profile, read the latest
// weekly digest.

use crate::app_support::archive_dir;
use crate::profile::ProfileScope;
use crate::{gemma, pack, profile, settings};
use serde::Serialize;
use serde_json::Value;
use std::sync::Mutex;
use tauri::Emitter;

/// Scope key ("work" | "personal") of the refresh currently running in its
/// detached thread, or None when idle. Lets a remounting ProfilePanel (the
/// component is destroyed on tab switch) rediscover an in-flight run.
static REFRESH_SCOPE: Mutex<Option<String>> = Mutex::new(None);

/// RAII marker for the running refresh: clears `REFRESH_SCOPE` on drop, so
/// panics and early returns in the worker thread can never leave a stale
/// "still running" status behind.
struct RefreshScopeGuard;

impl RefreshScopeGuard {
    fn set(scope: &str) -> Self {
        *lock_refresh_scope() = Some(scope.to_string());
        Self
    }
}

impl Drop for RefreshScopeGuard {
    fn drop(&mut self) {
        *lock_refresh_scope() = None;
    }
}

fn lock_refresh_scope() -> std::sync::MutexGuard<'static, Option<String>> {
    // A poisoned lock only means a panic between set and drop; the value is
    // a plain Option<String>, always safe to reuse.
    REFRESH_SCOPE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, Serialize)]
struct ProfileProgressPayload {
    current: usize,
    total: usize,
    stage: String,
    scope: String,
}

#[derive(Debug, Clone, Default, Serialize)]
struct ProfileDonePayload {
    busy: bool,
    session_count: usize,
    facts_count: usize,
    errors: Vec<String>,
    scope: String,
}

/// Parse the `scope` command argument ("work" | "personal").
fn parse_scope(scope: &str) -> Result<ProfileScope, String> {
    ProfileScope::from_key(scope).ok_or_else(|| format!("unknown profile scope: {scope}"))
}

/// Run a profile refresh for `scope` ("work" | "personal") in a background
/// thread. `full=true` ignores the watermark (complete rebuild); `false`
/// distills only sessions newer than that scope's `profile_meta.json`
/// watermark. Emits `profile-progress` events `{ current, total, stage,
/// scope }` while running and one `profile-done` event `{ busy,
/// session_count, facts_count, errors, scope }` when finished — the `scope`
/// field lets the UI ignore events for a scope other than the one displayed.
#[tauri::command]
pub(crate) fn run_profile_refresh(
    app: tauri::AppHandle,
    full: bool,
    scope: String,
) -> Result<(), String> {
    let profile_scope = parse_scope(&scope)?;
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
                    scope: scope.clone(),
                    ..Default::default()
                },
            );
            return;
        };

        // Skip the model load entirely when an incremental run has nothing new
        // to distill (checked under the lock so a concurrent sweep can't be
        // mid-write while we count) — unless a pending-facts snapshot from a
        // failed run exists, in which case the reduce must still run.
        if profile::pending_session_count(&dir, profile_scope, &profile_sources, full) == 0
            && !profile::has_pending_facts(&dir, profile_scope)
        {
            let _ = app.emit(
                "profile-done",
                ProfileDonePayload {
                    scope: scope.clone(),
                    ..Default::default()
                },
            );
            return;
        }

        // Published for `get_profile_refresh_status`; cleared on drop (RAII)
        // so this thread can never leave a stale "running" status behind.
        let _refresh_scope_guard = RefreshScopeGuard::set(&scope);

        // One warm model serves every map batch plus the reduce call, then is
        // unloaded when `session` drops at the end (sweep unload policy).
        let session = match gemma::start_tool_session(&backend, ctx_size) {
            Ok(session) => session,
            Err(error) => {
                let _ = app.emit(
                    "profile-done",
                    ProfileDonePayload {
                        errors: vec![format!("failed to start model: {error}")],
                        scope: scope.clone(),
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
        let progress_scope = scope.clone();
        let result = profile::run_refresh(
            &dir,
            profile_scope,
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
                        scope: progress_scope.clone(),
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
                scope: scope.clone(),
            },
            Err(error) => ProfileDonePayload {
                errors: vec![error.to_string()],
                scope: scope.clone(),
                ..Default::default()
            },
        };
        let _ = app.emit("profile-done", payload);
    });
    Ok(())
}

/// The scope key ("work" | "personal") of a profile refresh currently
/// running, or None when idle. Lets the PROFILE panel restore its busy state
/// after being destroyed and remounted by a tab switch.
#[tauri::command]
pub(crate) fn get_profile_refresh_status() -> Option<String> {
    lock_refresh_scope().clone()
}

/// Return the distilled profile.md content for `scope`, or None if no
/// profile exists yet.
#[tauri::command]
pub(crate) fn get_profile(app: tauri::AppHandle, scope: String) -> Result<Option<String>, String> {
    let profile_scope = parse_scope(&scope)?;
    let dir = archive_dir(&app)?;
    Ok(profile::read_profile_md(&dir, profile_scope))
}

/// Return the newest weekly digest content for `scope`, or None if none
/// exists yet.
#[tauri::command]
pub(crate) fn get_latest_digest(
    app: tauri::AppHandle,
    scope: String,
) -> Result<Option<String>, String> {
    let profile_scope = parse_scope(&scope)?;
    let dir = archive_dir(&app)?;
    Ok(profile::latest_digest(&dir, profile_scope))
}

/// What a Persona Pack export produced, returned to the UI.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PackResult {
    path: String,
    session_count: usize,
    digest_count: usize,
    raw_count: usize,
    includes_raw: bool,
}

/// Export the Work Persona Pack to `<archive>/exports/<user>-persona-<date>.zip`
/// and return the written path. Always Work scope (personal chat exports never
/// leave the machine). `include_raw` opts the un-redacted raw transcripts in.
#[tauri::command]
pub(crate) fn export_persona_pack(
    app: tauri::AppHandle,
    include_raw: bool,
) -> Result<PackResult, String> {
    let dir = archive_dir(&app)?;
    let settings = settings::load_settings(&dir);
    let work_sources = profile::sources_for_scope(&settings.profile_sources, ProfileScope::Work);
    let profile_md = profile::read_profile_md(&dir, ProfileScope::Work)
        .ok_or_else(|| pack::PackError::NoProfile.to_string())?;
    let digests = profile::all_digests(&dir, ProfileScope::Work);

    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let now = chrono::Utc::now();
    let dest = dir
        .join("exports")
        .join(pack::default_pack_name(&user, now));

    let embedding_model = std::path::Path::new(&settings.embedding_model_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .to_string();

    let summary = pack::export_persona_pack(
        &dir,
        &dest,
        &work_sources,
        &profile_md,
        &digests,
        include_raw,
        env!("CARGO_PKG_VERSION"),
        &embedding_model,
        now,
    )
    .map_err(|error| error.to_string())?;

    Ok(PackResult {
        path: summary.path.to_string_lossy().into_owned(),
        session_count: summary.session_count,
        digest_count: summary.digest_count,
        raw_count: summary.raw_count,
        includes_raw: summary.includes_raw,
    })
}

/// Number of Work sessions with a preserved raw transcript on disk — drives the
/// "incl. raw (N available)" hint on the Work profile panel. Cheap: reads the
/// archive index only, no inference.
#[tauri::command]
pub(crate) fn count_available_raw(app: tauri::AppHandle) -> Result<usize, String> {
    let dir = archive_dir(&app)?;
    let settings = settings::load_settings(&dir);
    let work_sources = profile::sources_for_scope(&settings.profile_sources, ProfileScope::Work);
    Ok(pack::count_available_raw(&dir, &work_sources))
}
