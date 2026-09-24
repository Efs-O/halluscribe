// HalluScribe - scheduled sweep execution and completion reporting.

use super::eligibility::{is_low_signal_business_session, is_low_signal_codex_session};
use super::helpers::{
    backend_display_name, is_sweep_due, model_display_name, provider_display_name,
};
use super::{SweepConfig, SweepProgress, SweepResult};
use crate::archive::{self, ArchiveError, SessionMeta};
use crate::gemma::GemmaError;
use crate::readers::{self, ChatProvider, ParsedSession};
use crate::retrieval;
use crate::scanner::{scan_sessions, ScanTargetKind};
use chrono::{Local, Timelike, Utc};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::Emitter;

/// The project a session is archived under. A per-session override (business
/// messaging) wins over the provider-derived label; otherwise the provider
/// label is used. Kept as a pure helper so the override precedence is testable
/// without driving a full sweep.
fn resolve_project(provider: &ChatProvider, source_path: &Path, override_: Option<&str>) -> String {
    override_
        .map(str::to_string)
        .unwrap_or_else(|| provider.project_label(source_path))
}

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
    // embedding code must never re-acquire it. `acquire_for_batch` additionally
    // reclaims any warm interactive server, so the sweep's model is the only
    // one resident instead of loading beside a chat model the idle watchdog
    // cannot reach while this lock is held.
    let Some(_inference_guard) = crate::infer_lock::acquire_for_batch() else {
        return SweepResult {
            busy: true,
            ..Default::default()
        };
    };

    let mut result = SweepResult {
        ran: true,
        ..Default::default()
    };
    if let Err(error) = archive::ensure_index_readable(&config.archive_dir) {
        // Global: every source, the business ones included, is affected.
        result.push_error(format!("archive index: {error}"), true);
        record_sweep_errors(&result);
        return result;
    }
    // Unreadable delete records would let every deleted session back in.
    if let Err(error) = archive::ensure_deleted_readable(&config.archive_dir) {
        result.push_error(format!("deleted sessions: {error}"), true);
        record_sweep_errors(&result);
        return result;
    }
    // Keep the backup's manifest index alive for the whole sweep, so the scan
    // and every backup reader share one read of `Manifest.db`. A failure here
    // is not reported: each reader opens the backup and surfaces its own error.
    let _backup_manifest = crate::scanner::resolve_apple_backup_path(&config.settings)
        .and_then(|dir| crate::apple_backup::open_backup(&dir).ok());
    let sources = scan_sessions(
        &config.archive_dir,
        &config.settings,
        config.lookback_secs,
        config.min_fill_pct,
        config.import_only,
    );
    let mut worklist = Vec::new();
    // One read of the index + captured manifest for the whole worklist, not
    // one per session (see `archive::SessionLookup`).
    let lookup = archive::SessionLookup::load(&config.archive_dir);

    for source in &sources {
        if cancel.load(Ordering::Relaxed) {
            result.cancelled = true;
            break;
        }

        let source_label = source
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("source");
        let default_cc = config.settings.business_default_country_code_opt();
        let parsed_sessions = match read_source_guarded(source_label, &source.path, || {
            readers::read_target(source, default_cc)
        }) {
            Ok(parsed_sessions) => parsed_sessions,
            Err(error) => {
                let business = matches!(&source.kind, ScanTargetKind::Import(p) if p.is_business());
                result.push_error(error, business);
                continue;
            }
        };

        if parsed_sessions.is_empty() {
            result.skipped += 1;
            continue;
        }

        for mut session in parsed_sessions {
            session.id = lookup.resolve_session_id(&session.source_path, &session.id);
            // The user deleted this session for good; its source is still here.
            if lookup.is_deleted(&session.id, &session.source_path) {
                result.skipped += 1;
                continue;
            }
            if is_unchanged_session(&lookup, &session) {
                result.skipped += 1;
                continue;
            }
            if is_low_signal_codex_session(&session) {
                result.skipped += 1;
                result.low_signal_skipped += 1;
                continue;
            }
            if is_low_signal_business_session(&session) {
                result.skipped += 1;
                result.business_filtered += 1;
                continue;
            }
            worklist.push(session);
        }
    }

    if cancel.load(Ordering::Relaxed) {
        result.cancelled = true;
        record_sweep_errors(&result);
        return result;
    }

    // Avoid a cold Qwen load when every discovered source was unchanged or
    // deliberately classified as low-signal during worklist construction.
    if worklist.is_empty() {
        return result;
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
            record_sweep_errors(&result);
            return result;
        }
    };

    // Sessions written this sweep, embedded in one batched pass at the end.
    let mut written_ids: Vec<String> = Vec::new();
    // The written business sessions, so an embedding failure on one counts as
    // a business error too.
    let mut business_written: HashSet<String> = HashSet::new();

    for (idx, session) in worklist.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            result.cancelled = true;
            break;
        }

        let current = idx + 1;

        emit_progress(app, current, total, &session.id, "processing");
        let transcript = session.transcript();
        let output = if crate::gemma::large_session::needs_chunking(
            &transcript,
            config.ctx_size,
            config.max_tokens,
        ) {
            let units = session.transcript_units();
            crate::gemma::large_session::summarize(
                &sweep_session,
                &session.provider,
                &units,
                config.ctx_size,
                config.max_tokens,
                &|| cancel.load(Ordering::Relaxed),
                &mut |chunk, chunks| {
                    emit_progress(
                        app,
                        current,
                        total,
                        &session.id,
                        &format!("chunk {chunk} of {chunks}"),
                    );
                },
            )
        } else {
            sweep_session.infer(config.max_tokens, &session.provider, &transcript)
        };
        let output = match output {
            Ok(output) => output,
            Err(GemmaError::Cancelled) => {
                result.cancelled = true;
                break;
            }
            Err(GemmaError::Conflict(_)) => {
                result.deferred += 1;
                emit_progress(app, current, total, &session.id, "deferred");
                continue;
            }
            Err(error) => {
                result.push_error(
                    format!(
                        "{} ({}): inference: {error}",
                        session.id,
                        session.source_path.display()
                    ),
                    session.provider.is_business(),
                );
                emit_progress(app, current, total, &session.id, "error");
                continue;
            }
        };

        // Preserve the untouched source transcript (compressed) so raw detail
        // survives the source tool pruning its own logs (Phase 1). Always on
        // (L3): a copy failure is non-fatal, the summary still archives, just
        // without a raw pointer.
        //
        // Readers whose source file holds many sessions (chat exports, the
        // Ollama DB) carry their own per-session slice; preserving
        // `source_path` for those would store the whole export once per
        // conversation. Everything else is one file per session, where the
        // file copy is the correct raw.
        //
        // A chat slice that replaces DIFFERENT stored bytes parks the previous
        // copy under `raw/superseded/` instead of destroying it, and reports
        // that it did — a later export can hand back a trimmed conversation.
        let preserved = match &session.raw_slice {
            Some(slice) => {
                archive::preserve_raw_bytes(&config.archive_dir, &session.id, slice.as_bytes()).map(
                    |preserved| {
                        if preserved.superseded.is_some() {
                            result.superseded += 1;
                        }
                        preserved.rel
                    },
                )
            }
            None => archive::preserve_raw(&config.archive_dir, &session.id, &session.source_path),
        };
        let raw_path = match preserved {
            Ok(rel) => Some(rel),
            Err(error) => {
                result.push_error(
                    format!(
                        "{} ({}): raw preserve: {error}",
                        session.id,
                        session.source_path.display()
                    ),
                    session.provider.is_business(),
                );
                None
            }
        };

        let meta = SessionMeta {
            id: session.id.clone(),
            source: session.source_path.clone(),
            project: resolve_project(
                &session.provider,
                &session.source_path,
                session.project_override.as_deref(),
            ),
            tool: provider_display_name(&session.provider),
            provider: session.provider.provider_key().to_string(),
            fill_pct: session.fill_pct,
            fill_estimated: session.fill_estimated,
            output_tokens: session.tokens.output,
            tokens_estimated: session.tokens.estimated,
            backend: backend_display_name(&config.backend),
            model: model_display_name(&config.backend),
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
                for warning in &written.warnings {
                    result.push_error(
                        format!(
                            "{} ({}): {warning}",
                            session.id,
                            session.source_path.display()
                        ),
                        session.provider.is_business(),
                    );
                }
                written_ids.push(session.id.clone());
                if session.provider.is_business() {
                    business_written.insert(session.id.clone());
                }
                emit_progress(app, current, total, &session.id, "done");
            }
            Err(ArchiveError::Io(error)) => {
                result.push_error(
                    format!(
                        "{} ({}): archive I/O: {error}",
                        session.id,
                        session.source_path.display()
                    ),
                    session.provider.is_business(),
                );
                emit_progress(app, current, total, &session.id, "error");
            }
            Err(error) => {
                result.push_error(
                    format!(
                        "{} ({}): archive: {error}",
                        session.id,
                        session.source_path.display()
                    ),
                    session.provider.is_business(),
                );
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
        let business = business_written.contains(&session_id);
        result.push_error(format!("{session_id}: embedding: {error}"), business);
    }

    record_sweep_errors(&result);
    result
}

/// Run one source's reader, turning both a reader error and a reader panic
/// (data the reader did not expect) into a per-source sweep error. A panic in
/// one source must not abort the whole sweep: the other sources still run and
/// the run still reaches its `sweep-done`.
fn read_source_guarded<F>(
    source_label: &str,
    path: &Path,
    read: F,
) -> Result<Vec<ParsedSession>, String>
where
    F: FnOnce() -> Result<Vec<ParsedSession>, readers::ReaderError>,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)) {
        Ok(Ok(parsed_sessions)) => Ok(parsed_sessions),
        Ok(Err(error)) => Err(format!(
            "{source_label} ({}): parse: {error}",
            path.display()
        )),
        Err(payload) => Err(format!(
            "{source_label} ({}): reader crashed: {}",
            path.display(),
            super::panic_message(&*payload)
        )),
    }
}

/// Persist all per-session errors because the UI completion toast deliberately
/// stays compact and would otherwise discard the actionable session details.
fn record_sweep_errors(result: &SweepResult) {
    for error in &result.errors {
        crate::llama_runtime::record_diagnostic("sweep", error);
    }
}

fn is_unchanged_session(lookup: &archive::SessionLookup, session: &ParsedSession) -> bool {
    let Some(existing) = lookup.find(&session.id) else {
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

#[cfg(test)]
mod tests {
    use super::{read_source_guarded, resolve_project};
    use crate::readers::{ChatProvider, ReaderError};
    use std::path::Path;

    fn path() -> &'static Path {
        Path::new("/tmp/nowhere/session.jsonl")
    }

    #[test]
    fn resolve_project_uses_override_when_set() {
        let project = resolve_project(&ChatProvider::AppleMessages, path(), Some("Client Acme"));
        assert_eq!(project, "Client Acme");
    }

    #[test]
    fn resolve_project_falls_back_to_provider_label_when_unset() {
        // AppleMessages has no per-path label, so the provider label is the
        // fixed "Messages" string.
        let project = resolve_project(&ChatProvider::AppleMessages, path(), None);
        assert_eq!(project, "Messages");
    }

    #[test]
    fn resolve_project_override_wins_over_provider_label() {
        // Even when the provider has a label, a set override wins.
        let project = resolve_project(&ChatProvider::AppleMessages, path(), Some("Other"));
        assert_eq!(project, "Other");
    }

    #[test]
    fn a_reader_panic_becomes_a_per_source_error() {
        let err =
            read_source_guarded("backup", path(), || panic!("index out of bounds")).unwrap_err();
        assert!(err.contains("reader crashed: index out of bounds"), "{err}");
        assert!(err.starts_with("backup ("), "{err}");
    }

    #[test]
    fn a_reader_error_keeps_the_parse_prefix() {
        let err = read_source_guarded("backup", path(), || {
            Err(ReaderError::Database("bad".to_string()))
        })
        .unwrap_err();
        assert!(err.contains("): parse: "), "{err}");
    }
}
