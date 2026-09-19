// HalluScribe - Tauri command handlers for the profile distiller (Persona
// Protocol Phase 2): trigger a refresh, read the profile, read the latest
// weekly digest.

use crate::app_state::ProfileCancel;
use crate::app_support::{archive_dir, default_archive_dir};
use crate::profile::ProfileScope;
use crate::{archive, gemma, pack, profile, settings};
use serde::Serialize;
use serde_json::Value;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

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

/// D13: a business (import-only) workspace must never be able to write a
/// profile — no refresh, merge, watermark, or pending-facts change. The host
/// owner's profile is read-only from here. Pure (paths in, verdict out) so the
/// read-only guarantee is unit-tested without a Tauri handle.
fn profile_refresh_gate(
    default_root: &std::path::Path,
    dir: &std::path::Path,
) -> Result<(), String> {
    if crate::workspace::is_active_import_only(default_root, dir)? {
        return Err(
            "Business workspaces are read-only for profiles. Switch to the host workspace to refresh a profile."
                .to_string(),
        );
    }
    Ok(())
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
    /// Hard failures AND non-fatal warnings, mixed — see
    /// `profile::RefreshOutcome::errors`. The UI must not treat a non-empty
    /// list as a failed run; that is what `failed_batches` is for.
    errors: Vec<String>,
    /// Map batches that failed outright. 0 = the profile was written.
    failed_batches: usize,
    scope: String,
    /// The user stopped the run. Not an error: partial work was saved and the
    /// next run resumes from it.
    cancelled: bool,
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
/// session_count, facts_count, errors, scope, cancelled }` when finished — the `scope`
/// field lets the UI ignore events for a scope other than the one displayed.
#[tauri::command]
pub(crate) fn run_profile_refresh(
    app: tauri::AppHandle,
    full: bool,
    scope: String,
) -> Result<(), String> {
    let profile_scope = parse_scope(&scope)?;
    let dir = archive_dir(&app)?;
    profile_refresh_gate(&default_archive_dir(&app)?, &dir)?;
    archive::ensure_index_readable(&dir).map_err(|error| error.to_string())?;
    let settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    let backend = settings
        .to_inference_backend()
        .ok_or_else(|| "backend not configured (check Settings)".to_string())?;
    let (ctx_size, max_tokens) = settings.generation_limits()?;
    let profile_sources = settings.profile_sources.clone();
    // A fresh cancel flag per run, so a stop pressed during a previous run can
    // never abort this one, and this run's flag is not a shared one a second
    // trigger could clear out from under it.
    let cancel = app
        .state::<ProfileCancel>()
        .0
        .try_begin_run()
        .ok_or_else(|| "A profile refresh is already running.".to_string())?;

    std::thread::spawn(move || {
        // Top-level inference lock, exactly like the sweep: the profile code
        // below never re-acquires it (the guard is non-reentrant). The batch
        // variant also reclaims a warm interactive server first, so a long
        // refresh never runs beside an idle chat model the watchdog cannot
        // unload while this lock is held.
        let Some(_inference_guard) = crate::infer_lock::acquire_for_batch() else {
            let _ = app.emit(
                "profile-done",
                ProfileDonePayload {
                    busy: true,
                    scope: scope.clone(),
                    ..Default::default()
                },
            );
            app.state::<ProfileCancel>().0.finish_run(&cancel);
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
            app.state::<ProfileCancel>().0.finish_run(&cancel);
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
                    // The run never reached a single batch — the exact case
                    // that used to look like "nothing happened" in the panel.
                    ProfileDonePayload {
                        errors: vec![format!("failed to start model: {error}")],
                        failed_batches: 1,
                        scope: scope.clone(),
                        ..Default::default()
                    },
                );
                app.state::<ProfileCancel>().0.finish_run(&cancel);
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
            &cancel,
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
                failed_batches: outcome.failed_batches,
                scope: scope.clone(),
                cancelled: outcome.cancelled,
            },
            // The refresh aborted before producing an outcome: nothing was
            // written, so this is unambiguously a failed run.
            Err(error) => ProfileDonePayload {
                errors: vec![error.to_string()],
                failed_batches: 1,
                scope: scope.clone(),
                ..Default::default()
            },
        };
        let _ = app.emit("profile-done", payload);
        app.state::<ProfileCancel>().0.finish_run(&cancel);
    });
    Ok(())
}

/// Signal the running profile refresh to stop at the next safe boundary: the
/// end of the current map batch, or the end of the current reduce call. One
/// flag covers both scopes - only one profile job can run at a time.
#[tauri::command]
pub(crate) fn cancel_profile_refresh(app: tauri::AppHandle) {
    app.state::<ProfileCancel>().0.request_cancel();
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

/// One scope's owner-profile status for the business (D13) settings UI.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct OwnerProfileStatus {
    pub scope: String,
    /// "off" (toggle off), "loaded" (owner profile present), or
    /// "owner_profile_not_found" (toggle on but the owner has no profile).
    pub state: String,
}

/// The per-scope owner-profile status for the active workspace, so the
/// settings UI can show "Owner profile not found" under a toggle that is on
/// but whose owner profile is missing. Pure mapping in `businessProfileStatus`
/// (src/lib) turns this into the note text.
#[tauri::command]
pub(crate) fn business_profile_status(
    app: tauri::AppHandle,
) -> Result<Vec<OwnerProfileStatus>, String> {
    let dir = archive_dir(&app)?;
    let default_root = default_archive_dir(&app)?;
    let import_only = crate::workspace::is_active_import_only(&default_root, &dir)?;
    let settings = settings::load_settings(&dir).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for scope in [ProfileScope::Work, ProfileScope::Personal] {
        // The same resolver the chat uses, so the note can never disagree
        // with what the prompt actually carries.
        let resolved = profile::resolve_chat_profile(
            &dir,
            scope,
            import_only,
            settings.business_use_work_profile,
            settings.business_use_personal_profile,
        );
        let state = match resolved {
            _ if !import_only => "off",
            profile::ChatProfile::Loaded(_) => "loaded",
            profile::ChatProfile::Absent => "off",
            profile::ChatProfile::OwnerProfileNotFound => "owner_profile_not_found",
        }
        .to_string();
        out.push(OwnerProfileStatus {
            scope: scope.dir_name().to_string(),
            state,
        });
    }
    Ok(out)
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

/// Export a Persona Pack for `scope` to
/// `<archive>/exports/<user>-<scope>-persona-<date>.zip` and return the
/// written path. Bundles that scope's distilled profile plus its consented
/// archive (`sources_for_scope`) — Personal's consent list is a superset of
/// Work's, so a Personal pack additionally carries the chat-export-derived
/// life context. `include_raw` opts the un-redacted raw transcripts in.
#[tauri::command]
pub(crate) fn export_persona_pack(
    app: tauri::AppHandle,
    include_raw: bool,
    scope: String,
) -> Result<PackResult, String> {
    let profile_scope = parse_scope(&scope)?;
    let dir = archive_dir(&app)?;
    archive::ensure_index_readable(&dir).map_err(|error| error.to_string())?;
    let settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    let sources = profile::sources_for_scope(&settings.profile_sources, profile_scope);
    let profile_md = profile::read_profile_md(&dir, profile_scope)
        .ok_or_else(|| pack::PackError::NoProfile.to_string())?;
    let digests = profile::all_digests(&dir, profile_scope);

    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let now = chrono::Utc::now();
    let dest = dir
        .join("exports")
        .join(pack::default_pack_name(&user, profile_scope, now));

    let embedding_model = std::path::Path::new(&settings.embedding_model_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("")
        .to_string();

    let summary = pack::export_persona_pack(
        &dir,
        &dest,
        &sources,
        &profile_md,
        &digests,
        include_raw,
        env!("CARGO_PKG_VERSION"),
        &embedding_model,
        profile_scope,
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

/// Number of sessions in `scope`'s consented sources with a preserved raw
/// transcript on disk — drives the "incl. raw (N available)" hint on that
/// scope's profile panel. Cheap: reads the archive index only, no inference.
#[tauri::command]
pub(crate) fn count_available_raw(app: tauri::AppHandle, scope: String) -> Result<usize, String> {
    let profile_scope = parse_scope(&scope)?;
    let dir = archive_dir(&app)?;
    archive::ensure_index_readable(&dir).map_err(|error| error.to_string())?;
    let settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    let sources = profile::sources_for_scope(&settings.profile_sources, profile_scope);
    Ok(pack::count_available_raw(&dir, &sources))
}

/// One-time, non-destructive backfill (Persona Parity Phase A): recover raw
/// transcripts for already-archived sessions whose original `source_jsonl`
/// still exists on disk. Raw preservation is unconditional (L3), so this
/// always runs when invoked.
#[tauri::command]
pub(crate) fn backfill_raw(app: tauri::AppHandle) -> Result<archive::BackfillResult, String> {
    let dir = archive_dir(&app)?;
    archive::backfill_raw(&dir).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{save_registry, Workspace, WorkspaceRegistry};
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "halluscribe_refresh_gate_{}_{}_{}",
            std::process::id(),
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_import_only_workspace_cannot_trigger_a_profile_refresh() {
        let default_root = tmp("gate_default");
        let guest = tmp("gate_guest");
        let mut reg = WorkspaceRegistry::default();
        let _ = crate::workspace::add_workspace(
            &mut reg,
            Workspace {
                name: "business".to_string(),
                path: guest.clone(),
                import_only: true,
            },
        );
        let _ = crate::workspace::set_active(&mut reg, Some(guest.clone()));
        save_registry(&default_root, &reg).unwrap();
        let err = profile_refresh_gate(&default_root, &guest).unwrap_err();
        assert!(err.contains("read-only"), "{err}");
    }

    #[test]
    fn a_normal_workspace_can_trigger_a_profile_refresh() {
        let default_root = tmp("gate_default_ok");
        let guest = tmp("gate_guest_ok");
        let mut reg = WorkspaceRegistry::default();
        let _ = crate::workspace::add_workspace(
            &mut reg,
            Workspace {
                name: "person".to_string(),
                path: guest.clone(),
                import_only: false,
            },
        );
        let _ = crate::workspace::set_active(&mut reg, Some(guest.clone()));
        save_registry(&default_root, &reg).unwrap();
        assert!(profile_refresh_gate(&default_root, &guest).is_ok());
    }
}
