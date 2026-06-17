// HalluScribe - semantic retrieval over archived session summaries.

mod embed;
mod search;
mod store;
#[cfg(test)]
mod tests;
mod types;

use crate::archive::{self, IndexEntry};
use crate::settings::HalluScribeSettings;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub use embed::kill_embedding_server;
pub use types::{EmbeddingRebuildResult, SemanticSearchResult};

const REBUILD_EMBED_BATCH_SIZE: usize = 16;

pub fn index_session(
    archive_dir: &Path,
    settings: &HalluScribeSettings,
    session_id: &str,
) -> Result<(), String> {
    let entry = archive::find_session(archive_dir, session_id)
        .ok_or_else(|| format!("session '{session_id}' not found in archive index"))?;
    let input = store::build_embedding_input(archive_dir, &entry).map_err(|error| {
        format!("failed to build embedding input for session '{session_id}': {error}")
    })?;
    let summary_hash = store::summary_hash(&input);
    let mut runner = embed::start_runner(settings).map_err(|error| {
        format!("failed to start embedding runtime for session '{session_id}': {error}")
    })?;
    let model_name = runner.model_name().to_string();
    if store::has_current_embedding(archive_dir, session_id, &model_name, &summary_hash) {
        return Ok(());
    }
    let embedding = runner
        .embed_documents(&[input])?
        .into_iter()
        .next()
        .ok_or_else(|| "embedding runtime returned no document vector".to_string())?;
    store::upsert_embedding(
        archive_dir,
        types::EmbeddingRecord {
            session_id: session_id.to_string(),
            model_name,
            summary_hash,
            embedding,
        },
    )
    .map_err(|error| format!("failed to persist embedding for session '{session_id}': {error}"))
}

/// Embed and persist the given sessions in one batched pass against a single
/// embedding-server lifetime (audit P-2). Already-current embeddings are
/// skipped. Returns `(session_id, error)` pairs for any that failed; the rest
/// are persisted. Callers must already hold the inference lock - this does not
/// re-acquire it (it runs inside the sweep's held guard).
pub fn index_sessions(
    archive_dir: &Path,
    settings: &HalluScribeSettings,
    session_ids: &[String],
) -> Vec<(String, String)> {
    let mut errors = Vec::new();
    let mut pending = Vec::new(); // (id, input, summary_hash)
    for id in session_ids {
        let Some(entry) = archive::find_session(archive_dir, id) else {
            errors.push((id.clone(), "session not found in archive index".to_string()));
            continue;
        };
        match store::build_embedding_input(archive_dir, &entry) {
            Ok(input) => {
                let summary_hash = store::summary_hash(&input);
                pending.push((id.clone(), input, summary_hash));
            }
            Err(error) => {
                errors.push((
                    id.clone(),
                    format!("failed to build embedding input: {error}"),
                ));
            }
        }
    }
    if pending.is_empty() {
        return errors;
    }

    let mut runner = match embed::start_runner(settings) {
        Ok(runner) => runner,
        Err(error) => {
            for (id, _, _) in pending {
                errors.push((id, format!("failed to start embedding runtime: {error}")));
            }
            return errors;
        }
    };
    let model_name = runner.model_name().to_string();

    for batch in pending.chunks(REBUILD_EMBED_BATCH_SIZE) {
        let fresh = batch
            .iter()
            .filter(|(id, _, summary_hash)| {
                !store::has_current_embedding(archive_dir, id, &model_name, summary_hash)
            })
            .collect::<Vec<_>>();
        if fresh.is_empty() {
            continue;
        }
        let inputs = fresh
            .iter()
            .map(|(_, input, _)| input.clone())
            .collect::<Vec<_>>();
        match runner.embed_documents(&inputs) {
            Ok(embeddings) if embeddings.len() == fresh.len() => {
                for ((id, _, summary_hash), embedding) in fresh.into_iter().zip(embeddings) {
                    let record = types::EmbeddingRecord {
                        session_id: id.clone(),
                        model_name: model_name.clone(),
                        summary_hash: summary_hash.clone(),
                        embedding,
                    };
                    if let Err(error) = store::upsert_embedding(archive_dir, record) {
                        errors.push((id.clone(), format!("failed to persist embedding: {error}")));
                    }
                }
            }
            Ok(_) => {
                for (id, _, _) in fresh {
                    errors.push((
                        id.clone(),
                        "embedding runtime returned a mismatched row count".to_string(),
                    ));
                }
            }
            Err(error) => {
                for (id, _, _) in fresh {
                    errors.push((id.clone(), error.clone()));
                }
            }
        }
    }
    errors
}

pub fn rebuild_embeddings(
    archive_dir: &Path,
    settings: &HalluScribeSettings,
) -> Result<EmbeddingRebuildResult, String> {
    rebuild_embeddings_with_progress(archive_dir, settings, |_, _, _| {})
}

pub fn rebuild_embeddings_with_progress<F>(
    archive_dir: &Path,
    settings: &HalluScribeSettings,
    mut on_batch: F,
) -> Result<EmbeddingRebuildResult, String>
where
    F: FnMut(usize, usize, &EmbeddingRebuildResult),
{
    let sessions = archive::read_sessions(archive_dir);
    let live_ids = sessions
        .iter()
        .map(|entry| entry.id.clone())
        .collect::<HashSet<_>>();
    let mut result = EmbeddingRebuildResult::default();
    let existing_records = store::load_embeddings(archive_dir)
        .ok()
        .unwrap_or_default()
        .into_iter()
        .filter(|record| live_ids.contains(&record.session_id))
        .collect::<Vec<_>>();
    let mut record_map = existing_records
        .into_iter()
        .map(|record| (record.session_id.clone(), record))
        .collect::<HashMap<_, _>>();

    let mut pending_entries = Vec::new();
    let mut pending_inputs = Vec::new();

    for entry in sessions {
        let input = match store::build_embedding_input(archive_dir, &entry) {
            Ok(input) => input,
            Err(_) => {
                result.failed += 1;
                continue;
            }
        };
        let summary_hash = store::summary_hash(&input);
        pending_inputs.push(input);
        pending_entries.push((entry.id, summary_hash));
    }

    if pending_entries.is_empty() {
        on_batch(0, 0, &result);
        return Ok(result);
    }

    let mut runner = match embed::start_runner(settings) {
        Ok(runner) => runner,
        Err(_) => {
            result.failed += pending_entries.len() as u32;
            on_batch(0, pending_entries.len(), &result);
            return Ok(result);
        }
    };
    let model_name = runner.model_name().to_string();

    let mut offset = 0usize;
    let total_pending = pending_entries.len();
    while offset < pending_entries.len() {
        let end = (offset + REBUILD_EMBED_BATCH_SIZE).min(pending_entries.len());
        let batch_inputs = pending_inputs[offset..end]
            .iter()
            .zip(pending_entries[offset..end].iter())
            .filter_map(|(input, (session_id, summary_hash))| {
                (!has_current_embedding(&record_map, session_id, &model_name, summary_hash))
                    .then_some(input.clone())
            })
            .collect::<Vec<_>>();
        let batch_entries = pending_entries[offset..end]
            .iter()
            .filter(|(session_id, summary_hash)| {
                !has_current_embedding(&record_map, session_id, &model_name, summary_hash)
            })
            .cloned()
            .collect::<Vec<_>>();
        if batch_entries.is_empty() {
            result.skipped += (end - offset) as u32;
            offset = end;
            on_batch(offset, total_pending, &result);
            continue;
        }
        match runner.embed_documents(&batch_inputs) {
            Ok(embeddings) if embeddings.len() == batch_entries.len() => {
                for ((session_id, summary_hash), embedding) in
                    batch_entries.into_iter().zip(embeddings)
                {
                    record_map.insert(
                        session_id.clone(),
                        types::EmbeddingRecord {
                            session_id,
                            model_name: model_name.clone(),
                            summary_hash,
                            embedding,
                        },
                    );
                    result.indexed += 1;
                }
            }
            Ok(_) => {
                result.failed += batch_entries.len() as u32;
            }
            Err(_) => {
                result.failed += batch_entries.len() as u32;
            }
        }
        offset = end;
        on_batch(offset, total_pending, &result);
    }

    store::replace_embeddings(archive_dir, record_map.into_values().collect())?;
    Ok(result)
}

fn has_current_embedding(
    record_map: &HashMap<String, types::EmbeddingRecord>,
    session_id: &str,
    model_name: &str,
    summary_hash: &str,
) -> bool {
    record_map.get(session_id).is_some_and(|record| {
        record.model_name == model_name && record.summary_hash == summary_hash
    })
}

pub fn semantic_search(
    archive_dir: &Path,
    settings: &HalluScribeSettings,
    query: &str,
    limit: usize,
    allowed_ids: Option<&HashSet<String>>,
) -> Result<Vec<SemanticSearchResult>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let query_embedding = embed::embed_query(settings, query)?;
    let model_name = embed::embedding_model_name(settings)?;
    semantic_search_for_embedding(
        archive_dir,
        &query_embedding,
        &model_name,
        limit,
        allowed_ids,
    )
}

fn semantic_search_for_embedding(
    archive_dir: &Path,
    query_embedding: &[f32],
    model_name: &str,
    limit: usize,
    allowed_ids: Option<&HashSet<String>>,
) -> Result<Vec<SemanticSearchResult>, String> {
    let records = store::load_embeddings(archive_dir)?
        .into_iter()
        .filter(|record| record.model_name == model_name)
        .collect::<Vec<_>>();
    let matches = search::rank_matches(query_embedding, records, limit, allowed_ids);
    Ok(search::attach_entries(archive_dir, matches))
}

pub fn remove_embeddings(archive_dir: &Path, session_ids: &[String]) -> Result<(), String> {
    store::remove_embeddings(archive_dir, session_ids)
}

pub fn semantic_scope_ids(
    archive_dir: &Path,
    settings: &HalluScribeSettings,
    query: &str,
    limit: usize,
    allowed_ids: Option<&HashSet<String>>,
) -> Result<Vec<String>, String> {
    let results = semantic_search(archive_dir, settings, query, limit, allowed_ids)?;
    semantic_scope_ids_from_results(results)
}

fn semantic_scope_ids_from_results(
    results: Vec<SemanticSearchResult>,
) -> Result<Vec<String>, String> {
    if results.is_empty() {
        return Err(
            "Semantic search found no relevant archived sessions. Try archive mode or rebuild embeddings."
                .to_string(),
        );
    }
    Ok(results.into_iter().map(|result| result.entry.id).collect())
}

pub fn embedding_runtime_ready(settings: &HalluScribeSettings) -> bool {
    !settings.llama_server_bin.trim().is_empty() && !settings.embedding_model_path.trim().is_empty()
}

pub fn semantic_index_entry(archive_dir: &Path, session_id: &str) -> Option<IndexEntry> {
    archive::find_session(archive_dir, session_id)
}
