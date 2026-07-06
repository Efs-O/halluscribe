// HalluScribe - tests for AND-of-tokens query matching and paged-search
// ranking (SEARCH_TOKENIZATION_PLAN.md). Split out of search/tests.rs to
// stay under the 350 LOC file cap.

use super::*;
use std::{fs, path::Path};
use tempfile::TempDir;

fn tmp() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// Write a minimal index.json from a slice of (id, title, date, tool) tuples.
fn make_index(dir: &Path, entries: &[(&str, &str, &str, &str)]) {
    let sessions: Vec<serde_json::Value> = entries
        .iter()
        .map(|(id, title, date, tool)| {
            serde_json::json!({
                "id": id,
                "project": "proj",
                "date": date,
                "title": title,
                "tool": tool,
                "fill_pct": 60.0,
                "session_type": "debugging",
                "error_tags": ["jwt"],
                "topic_tags": ["rust"],
                "archive_path": format!("sessions/proj/{date}/12-00-00-claudecode-sweep.md"),
                "source_jsonl": "/fake/path.jsonl"
            })
        })
        .collect();
    let idx = serde_json::json!({ "sessions": sessions });
    fs::write(dir.join("index.json"), serde_json::to_string(&idx).unwrap()).unwrap();
}

fn write_md(dir: &Path, date: &str, body: &str) {
    let md_path = dir
        .join("sessions/proj")
        .join(date)
        .join("12-00-00-claudecode-sweep.md");
    fs::create_dir_all(md_path.parent().unwrap()).unwrap();
    fs::write(&md_path, body).unwrap();
}

/// The synthetic entry mirroring the real `c47cc0fa` incident (plan §A1):
/// title has "Forge", "MCP", "Bridge", "Implementation" as separate words;
/// only the body contains the contiguous identifier "mcpBridge" and the word
/// "client"/"implemented".
const FORGE_TITLE: &str = "Forge MCP Bridge Implementation and HalluScribe Sync";
// Deliberately does NOT contain the contiguous run "Forge MCP client bridge"
// anywhere (word order is shuffled) so T3's quoted-phrase test can prove the
// unquoted AND-of-tokens match and the quoted exact-substring match diverge.
const FORGE_BODY: &str = "Implemented Forge's MCP client integration; the bridge (mcpBridge.ts) \
     now connects to HalluScribe's read-only tools. Part B implemented and released as Forge v0.12.27.";

fn make_forge_entry(dir: &Path) {
    make_index(dir, &[("c47cc0fa", FORGE_TITLE, "2026-07-06", "Forge")]);
    write_md(dir, "2026-07-06", FORGE_BODY);
}

// --- T1: the three incident queries all find the Forge entry -------------

#[test]
fn t1_unquoted_multiword_query_and_client_bridge() {
    let dir = tmp();
    make_forge_entry(dir.path());
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            query: Some("Forge MCP client bridge".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "c47cc0fa");
}

#[test]
fn t1_camelcase_identifier_and_implementation() {
    let dir = tmp();
    make_forge_entry(dir.path());
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            query: Some("mcpBridge implementation".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "c47cc0fa");
}

#[test]
fn t1_stop_word_laden_question_form() {
    let dir = tmp();
    make_forge_entry(dir.path());
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            query: Some("has the Forge bridge been implemented".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "c47cc0fa");
}

// --- T2: single-word queries are unchanged --------------------------------

#[test]
fn t2_single_word_query_result_set_unchanged() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[
            ("a", "Auth refactor", "2026-04-15", "Claude Code"),
            ("b", "Codex session", "2026-04-14", "Codex"),
        ],
    );
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            query: Some("auth".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "a");
}

// --- T3: quoted phrase = exact substring only ------------------------------

#[test]
fn t3_quoted_non_contiguous_phrase_does_not_match() {
    let dir = tmp();
    make_forge_entry(dir.path());
    // "Forge MCP client bridge" is never contiguous in title or body - the
    // real title has "MCP Bridge" and the body has "MCP client bridge", not
    // "MCP client bridge" as a run that includes "Forge" immediately before.
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            query: Some("\"Forge MCP client bridge\"".into()),
            ..Default::default()
        },
    );
    assert!(results.is_empty());
}

#[test]
fn t3_quoted_contiguous_phrase_matches() {
    let dir = tmp();
    make_forge_entry(dir.path());
    // "MCP Bridge Implementation" is a literal contiguous run in the title.
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            query: Some("\"MCP Bridge Implementation\"".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "c47cc0fa");
}

// --- T6: metadata-satisfied query never reads the (missing) body ----------

#[test]
fn t6_metadata_only_match_does_not_require_body_file() {
    let dir = tmp();
    // No write_md call at all - the .md file never exists on disk.
    make_index(
        dir.path(),
        &[("a", "Forge MCP Bridge session", "2026-04-15", "Forge")],
    );
    let results = search_sessions(
        dir.path(),
        &SearchParams {
            query: Some("forge bridge".into()),
            ..Default::default()
        },
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "a");
}

// --- T7: ranking on the paged path -----------------------------------------

#[test]
fn t7_title_hit_outranks_body_only_hit() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[
            // Older, but the query terms are in the title -> higher score.
            (
                "title-hit",
                "Auth token rotation",
                "2026-01-01",
                "Claude Code",
            ),
            // Newer, but the query terms only appear in the body -> lower score.
            ("body-hit", "Unrelated session", "2026-06-01", "Claude Code"),
        ],
    );
    write_md(dir.path(), "2026-01-01", "nothing relevant here");
    write_md(
        dir.path(),
        "2026-06-01",
        "discussion of auth token rotation deep in the notes",
    );

    let page = search_sessions_page(
        dir.path(),
        &SearchParams {
            query: Some("auth token".into()),
            ..Default::default()
        },
        None,
    );
    assert_eq!(page.total_matches, 2);
    assert_eq!(page.results[0].id, "title-hit");
    assert_eq!(page.results[1].id, "body-hit");
}

#[test]
fn t7_equal_scores_fall_back_to_recency() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[
            ("older", "Auth token work", "2026-01-01", "Claude Code"),
            ("newer", "Auth token work", "2026-02-01", "Claude Code"),
        ],
    );
    let page = search_sessions_page(
        dir.path(),
        &SearchParams {
            query: Some("auth token".into()),
            ..Default::default()
        },
        None,
    );
    assert_eq!(page.total_matches, 2);
    // Same title -> same score; tie-break is date desc, same as pre-ranking order.
    assert_eq!(page.results[0].id, "newer");
    assert_eq!(page.results[1].id, "older");
}

// --- T8: existing paging semantics unchanged (spot check alongside tests.rs) ---

#[test]
fn t8_paging_counts_unchanged_by_tokenization() {
    let dir = tmp();
    let entries: Vec<_> = (0u8..5)
        .map(|i| {
            (
                Box::leak(i.to_string().into_boxed_str()) as &str,
                "Auth work session",
                "2026-04-15",
                "Claude Code",
            )
        })
        .collect();
    make_index(dir.path(), &entries);
    let page = search_sessions_page(
        dir.path(),
        &SearchParams {
            query: Some("auth work".into()),
            limit: Some(2),
            offset: Some(1),
            ..Default::default()
        },
        None,
    );
    assert_eq!(page.searched, 5);
    assert_eq!(page.total_matches, 5);
    assert_eq!(page.returned, 2);
    assert_eq!(page.offset, 1);
}

// --- T10: search_fulltext multi-token query incl. a project-only hit ------

#[test]
fn t10_fulltext_multitoken_matches_project_field() {
    let dir = tmp();
    make_index(
        dir.path(),
        &[("a", "Sweep run", "2026-04-15", "Claude Code")],
    );
    // make_index hardcodes project = "proj"; a token that only appears there
    // must still be found by search_fulltext (which, unlike matches_params,
    // does check the project field - see content::matches_fulltext).
    let results = search_fulltext(dir.path(), "proj sweep");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "a");
}

#[test]
fn t10_fulltext_quoted_phrase_still_exact_substring() {
    let dir = tmp();
    make_forge_entry(dir.path());
    let results = search_fulltext(dir.path(), "\"Forge MCP client bridge\"");
    assert!(results.is_empty());
}
