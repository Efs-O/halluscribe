use super::helpers::{backend_display_name, is_sweep_due, provider_display_name};
use super::{SweepConfig, SweepProgress, SweepResult};
use crate::archive::{self, ArchiveError, SessionMeta};
use crate::gemma::GemmaError;
use crate::readers::{self, ParsedSession};
use crate::retrieval;
use crate::scanner::scan_sessions;
use chrono::{Local, Timelike, Utc};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::Emitter;

/// Run one sweep pass according to `config`.
///
/// Returns a `SweepResult` in all cases - callers do not need to match on Err.
/// Non-fatal per-session failures are collected into `result.errors`.
/// Emits "sweep-progress" Tauri events after each session so the UI can show
/// a live counter (e.g. "Processing 47 of 312...").
pub fn run_sweep(
    app: &tauri::AppHandle,
    config: &SweepConfig,
    cancel: Arc<AtomicBool>,
) -> SweepResult {
    let now = Local::now();
    let today = now.format("%Y-%m-%d").to_string();
    if !config.force
        && !is_sweep_due(
            now.hour(),
            now.minute(),
            &today,
            &config.schedule_time,
            &config.last_sweep_date,
        )
    {
        return SweepResult::default();
    }

    // CLAUDE.md: never run inference concurrently. Hold the process-wide
    // inference lock for the whole sweep so a manual "Run Now", a second
    // scheduled tick, or an active briefing/chat cannot load a second model.
    // The guard also covers retrieval::index_sessions below (same thread), so
    // embedding code must never re-acquire it.
    let Some(_inference_guard) = crate::infer_lock::try_acquire() else {
        return SweepResult {
            busy: true,
            ..Default::default()
        };
    };

    let mut result = SweepResult {
        ran: true,
        ..Default::default()
    };
    let sources = scan_sessions(
        &config.archive_dir,
        &config.settings,
        config.lookback_secs,
        config.min_fill_pct,
        config.import_only,
    );
    let mut worklist = Vec::new();

    for source in &sources {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let source_label = source
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("source");
        let parsed_sessions = match readers::read_target(source) {
            Ok(parsed_sessions) => parsed_sessions,
            Err(error) => {
                result
                    .errors
                    .push(format!("{source_label}: parse: {error}"));
                continue;
            }
        };

        if parsed_sessions.is_empty() {
            result.skipped += 1;
            continue;
        }

        for session in parsed_sessions {
            if is_unchanged_session(&config.archive_dir, &session) {
                result.skipped += 1;
                continue;
            }
            worklist.push(session);
        }
    }

    let total = worklist.len();

    // Load the model once for the whole sweep instead of a cold start per
    // session (audit P-1). Dropping `sweep_session` at the end unloads it.
    let sweep_session = match crate::gemma::start_sweep_session(&config.backend, config.ctx_size) {
        Ok(sweep_session) => sweep_session,
        Err(error) => {
            result
                .errors
                .push(format!("inference: failed to start model: {error}"));
            return result;
        }
    };

    // Sessions written this sweep, embedded in one batched pass at the end.
    let mut written_ids: Vec<String> = Vec::new();

    for (idx, session) in worklist.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        let current = idx + 1;

        emit_progress(app, current, total, &session.id, "processing");
        let transcript = session.transcript();
        let output = match sweep_session.infer(config.max_tokens, &session.provider, &transcript) {
            Ok(output) => output,
            Err(GemmaError::Conflict(_)) => {
                result.deferred += 1;
                emit_progress(app, current, total, &session.id, "deferred");
                continue;
            }
            Err(error) => {
                result
                    .errors
                    .push(format!("{}: inference: {error}", session.id));
                emit_progress(app, current, total, &session.id, "error");
                continue;
            }
        };

        // Preserve the untouched source transcript (compressed) so raw detail
        // survives the source tool pruning its own logs (Phase 1). Always on
        // (L3): a copy failure is non-fatal, the summary still archives, just
        // without a raw pointer.
        let raw_path =
            match archive::preserve_raw(&config.archive_dir, &session.id, &session.source_path) {
                Ok(rel) => Some(rel),
                Err(error) => {
                    result
                        .errors
                        .push(format!("{}: raw preserve: {error}", session.id));
                    None
                }
            };

        let meta = SessionMeta {
            id: session.id.clone(),
            source: session.source_path.clone(),
            project: session.provider.project_label(&session.source_path),
            tool: provider_display_name(&session.provider),
            provider: session.provider.provider_key().to_string(),
            fill_pct: session.fill_pct,
            fill_estimated: session.fill_estimated,
            backend: backend_display_name(&config.backend),
            session_timestamp: session.created_at,
            updated_at: session.updated_at,
            transcript_hash: session.transcript_hash.clone(),
            raw_path,
        };

        match archive::write_session(&config.archive_dir, &meta, &output, Utc::now()) {
            Ok(written) => {
                result.processed += 1;
                if !written.secret_flags.is_empty() {
                    result.flagged += 1;
                }
                written_ids.push(session.id.clone());
                emit_progress(app, current, total, &session.id, "done");
            }
            Err(ArchiveError::Io(error)) => {
                result
                    .errors
                    .push(format!("{}: archive I/O: {error}", session.id));
                emit_progress(app, current, total, &session.id, "error");
            }
            Err(error) => {
                result
                    .errors
                    .push(format!("{}: archive: {error}", session.id));
                emit_progress(app, current, total, &session.id, "error");
            }
        }
    }

    // Unload the Gemma model before loading the embedding model so the two are
    // never resident at once, then embed everything this sweep wrote in one
    // batched pass against a single embedding-server lifetime (audit P-2).
    drop(sweep_session);
    for (session_id, error) in
        retrieval::index_sessions(&config.archive_dir, &config.settings, &written_ids)
    {
        result
            .errors
            .push(format!("{session_id}: embedding: {error}"));
    }

    result
}

fn is_unchanged_session(archive_dir: &std::path::Path, session: &ParsedSession) -> bool {
    let Some(existing) = archive::find_session(archive_dir, &session.id) else {
        return false;
    };

    // The transcript hash is the only authoritative signal: when both sides
    // have one, trust it alone. A weaker signal (e.g. a byte size that happens
    // to be unchanged after an in-place edit) must never override a hash that
    // says the content changed, or a stale summary would be kept forever.
    if !existing.transcript_hash.is_empty() && !session.transcript_hash.is_empty() {
        return existing.transcript_hash == session.transcript_hash;
    }

    // Legacy archives without a stored hash fall back to updated_at, then size.
    if !existing.updated_at.is_empty() {
        return session.updated_at.map(|date| date.to_rfc3339()).as_deref()
            == Some(existing.updated_at.as_str());
    }
    let current_size = std::fs::metadata(&session.source_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    existing.source_size_bytes == current_size
}

fn emit_progress(
    app: &tauri::AppHandle,
    current: usize,
    total: usize,
    session_id: &str,
    status: &str,
) {
    let _ = app.emit(
        "sweep-progress",
        SweepProgress {
            current,
            total,
            session_id: session_id.to_string(),
            status: status.to_string(),
        },
    );
}
