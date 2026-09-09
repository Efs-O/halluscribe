// HalluScribe - markdown archive writer and archive-specific tests.

use super::index::{append_index, ensure_index_readable};
use super::redact::{apply_rules, rules_for_session};
use super::{ArchiveError, IndexEntry, SessionMeta, WrittenSession};
use crate::gemma::{GemmaOutput, SessionType};
use crate::scanner::secrets::scan_for_secrets;
use chrono::{DateTime, Local, Utc};
use std::fs;
use std::path::{Path, PathBuf};

pub fn write_session(
    archive_dir: &Path,
    meta: &SessionMeta,
    output: &GemmaOutput,
    now: DateTime<Utc>,
) -> Result<WrittenSession, ArchiveError> {
    // Refuse before creating a summary file when the existing archive index
    // cannot be read. Otherwise a corrupt index leaves an unindexed markdown
    // orphan even though `append_index` correctly declines to overwrite it.
    ensure_index_readable(archive_dir)?;
    // Archive names and user-facing dates follow the computer's local clock.
    // The index retains UTC RFC 3339 timestamps as canonical metadata, so
    // sessions remain unambiguous across time zones and daylight-saving shifts.
    let local_now = now.with_timezone(&Local);
    let local_session_timestamp = meta.session_timestamp.with_timezone(&Local);
    let sweep_date_str = local_now.format("%Y-%m-%d").to_string();
    let time_str = local_now.format("%H-%M-%S").to_string();
    let session_date_str = local_session_timestamp.format("%Y-%m-%d").to_string();
    let slug = tool_slug(&meta.tool);

    // The session id is part of the file name so two sessions swept in the
    // same second (a batch sweep iterates fast) cannot collide onto the same
    // path and silently overwrite each other's markdown while the index keeps
    // two rows pointing at one file.
    let rel = PathBuf::from("sessions")
        .join(&meta.project)
        .join(&sweep_date_str)
        .join(format!("{time_str}-{slug}-{}-sweep.md", meta.id));
    let abs = archive_dir.join(&rel);

    fs::create_dir_all(abs.parent().expect("path has parent"))?;

    let title = if output.title.is_empty() {
        format!(
            "{} - {session_date_str} {}",
            meta.project,
            local_session_timestamp.format("%H:%M")
        )
    } else {
        output.title.clone()
    };

    let markdown = build_markdown(&title, meta, output, local_now);
    let rules = rules_for_session(archive_dir, &meta.id);
    let markdown = if rules.is_empty() {
        markdown
    } else {
        apply_rules(&markdown, &rules)
    };
    fs::write(&abs, &markdown)?;
    // A new body on disk must not be answered for out of the search cache.
    crate::search::invalidate_body_cache(archive_dir);

    let secret_flags = scan_for_secrets(&markdown);

    append_index(
        archive_dir,
        IndexEntry {
            id: meta.id.clone(),
            project: meta.project.clone(),
            date: session_date_str,
            title,
            tool: meta.tool.clone(),
            fill_pct: meta.fill_pct,
            session_timestamp: meta.session_timestamp.to_rfc3339(),
            updated_at: meta
                .updated_at
                .map(|date| date.to_rfc3339())
                .unwrap_or_default(),
            session_type: session_type_str(&output.session_type),
            error_tags: output.error_tags.clone(),
            topic_tags: output.topic_tags.clone(),
            archive_path: rel.to_string_lossy().replace('\\', "/"),
            source_jsonl: meta.source.to_string_lossy().into_owned(),
            source_size_bytes: fs::metadata(&meta.source).map(|m| m.len()).unwrap_or(0),
            provider: meta.provider.clone(),
            fill_estimated: meta.fill_estimated,
            output_tokens: meta.output_tokens,
            tokens_estimated: meta.tokens_estimated,
            transcript_hash: meta.transcript_hash.clone(),
            secret_flags: secret_flags.clone(),
            raw_path: meta.raw_path.clone().unwrap_or_default(),
        },
    )?;

    Ok(WrittenSession {
        path: abs,
        secret_flags,
    })
}

fn build_markdown(
    title: &str,
    meta: &SessionMeta,
    output: &GemmaOutput,
    now: DateTime<Local>,
) -> String {
    let session_dt = meta.session_timestamp.with_timezone(&Local);
    format!(
        "# {title}\n\n\
         **Tool:** {tool}  \n\
         **Provider:** {provider}  \n\
         **Date created:** {session_dt}  \n\
         {updated_line}\
         **Archived:** {archived_dt}  \n\
         **Fill at compact:** {fill_label}  \n\
         **Source:** {source}  \n\
         \n---\n\n\
         {summary}\n\n\
         {highlights}\
         ---\n\
         *Archived by HalluScribe - Gemma 4 26B via {backend}*\n",
        tool = meta.tool,
        provider = meta.provider,
        session_dt = session_dt.format("%Y-%m-%d %H:%M %:z"),
        updated_line = meta
            .updated_at
            .map(|date| {
                format!(
                    "**Last modified:** {}  \n",
                    date.with_timezone(&Local).format("%Y-%m-%d %H:%M %:z")
                )
            })
            .unwrap_or_default(),
        archived_dt = now.format("%Y-%m-%d %H:%M %:z"),
        fill_label = if meta.fill_estimated {
            format!("~{:.1}%", meta.fill_pct)
        } else {
            format!("{:.1}%", meta.fill_pct)
        },
        source = meta.source.display(),
        summary = output.summary,
        highlights = highlights_section(&output.verbatim_highlights),
        backend = meta.backend,
    )
}

/// Render verbatim highlights as their own section, or nothing at all when the
/// session had none. Kept out of the summary prose so this content does not
/// compete for the summariser's word budget, and written into the same `.md`
/// the search body matcher reads, so it is keyword-searchable like the rest.
fn highlights_section(highlights: &[String]) -> String {
    if highlights.is_empty() {
        return String::new();
    }
    let body = highlights
        .iter()
        .map(|h| h.trim())
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("## Highlights\n\n{body}\n\n")
}

fn tool_slug(tool: &str) -> &str {
    let t = tool.to_lowercase();
    if t.contains("claude") {
        "claudecode"
    } else if t.contains("codex") {
        "codex"
    } else if t.contains("forge") {
        "forge"
    } else if t.contains("chatgpt") {
        "chatgpt"
    } else if t.contains("gemma") {
        "gemma4"
    } else if t.contains("gemini") {
        "gemini"
    } else if t.contains("grok") {
        "grok"
    } else if t.contains("claude.ai") || t == "claude" {
        "claude"
    } else {
        "continue"
    }
}

fn session_type_str(t: &SessionType) -> String {
    match t {
        SessionType::Debugging => "debugging",
        SessionType::Building => "building",
        SessionType::Refactoring => "refactoring",
        SessionType::Exploration => "exploration",
    }
    .to_string()
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod tests;
