// HalluScribe - MCP tool router: 4 read-only tools backed by the existing
// search and profile modules. No write/redact/delete surface is exposed here
// by design - see docs/internal/PERSONA_PROTOCOL_PLAN.md Phase 3.

use crate::profile::{self, ProfileScope};
use crate::search::{self, SearchParams};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::{schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use std::path::PathBuf;

/// Search filters mirrored 1:1 onto `search::SearchParams`. All fields are
/// optional; omitted fields match every session.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchSessionsRequest {
    /// Text matched (case-insensitive) against title, error tags, topic tags,
    /// and finally the session body if nothing in metadata matches.
    #[serde(default)]
    pub query: Option<String>,
    /// ISO date lower bound (YYYY-MM-DD, inclusive).
    #[serde(default)]
    pub date_from: Option<String>,
    /// ISO date upper bound (YYYY-MM-DD, inclusive).
    #[serde(default)]
    pub date_to: Option<String>,
    /// Every tag listed must appear in the session's error or topic tags.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Substring match (case-insensitive) against the project field.
    #[serde(default)]
    pub project: Option<String>,
    /// Tool filter, e.g. "claude_code", "codex", "continue", "forge".
    #[serde(default)]
    pub tool: Option<String>,
    /// Max results returned (default 20, hard-capped at 30).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Number of leading matches to skip before the returned page.
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadSessionRequest {
    /// The session `id` field from a `search_sessions` result.
    pub session_id: String,
}

/// Optional scope selector for the profile/digest tools.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ScopeRequest {
    /// Which profile to return: "work" (default) or "personal". Personal is
    /// a SUPERSET that also carries private-chat-derived (ChatGPT/Claude.ai/
    /// Gemini) life context, so request it only when that exposure is intended.
    #[serde(default)]
    pub scope: Option<String>,
}

/// Parameters for `get_digest`. Digests grow with the archive (a busy week can
/// exceed 100KB) and easily overflow an MCP client's per-result token cap, so
/// unlike the fixed-size profile the digest is paged.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DigestRequest {
    /// Which scope's digest to return: "work" (default) or "personal". Personal
    /// is a SUPERSET that also carries private-chat-derived (ChatGPT/Claude.ai/
    /// Gemini) life context, so request it only when that exposure is intended.
    #[serde(default)]
    pub scope: Option<String>,
    /// Byte offset to continue from (default 0). Use the `offset=N` value named
    /// in the previous slice's header line. Snapped down to a UTF-8 boundary.
    #[serde(default)]
    pub offset: Option<usize>,
    /// Max bytes returned per call (default 20000, capped at 50000).
    #[serde(default)]
    pub max_chars: Option<usize>,
}

/// Default / hard-cap slice sizes for `get_digest`. 20KB ≈ 5-7K tokens — safely
/// under common MCP client result caps (25K tokens) with headroom to spare.
const DIGEST_DEFAULT_MAX_CHARS: usize = 20_000;
const DIGEST_MAX_CHARS_CAP: usize = 50_000;

/// Largest byte index `<= i` that lands on a UTF-8 character boundary of `s`.
fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Maps the optional wire-format scope string to a `ProfileScope`, defaulting
/// to Work for back-compat (no `scope` arg) and falling back to Work on any
/// unrecognized value rather than erroring.
fn scope_or_work(scope: Option<String>) -> ProfileScope {
    scope
        .as_deref()
        .and_then(ProfileScope::from_key)
        .unwrap_or(ProfileScope::Work)
}

/// Stateless (beyond the archive path) MCP server: one HalluScribe archive,
/// four read-only tools. Constructed once in `main` with the resolved
/// archive directory and served over stdio for the lifetime of the process.
/// `#[tool_router]`/`#[tool_handler]` build the tool dispatch table fresh
/// from `Self::tool_router()` on each call rather than from a stored field
/// (the pattern used by the rust-sdk's own examples), so no router field is
/// kept on the struct.
#[derive(Clone)]
pub struct HalluscribeServer {
    archive_dir: PathBuf,
}

impl HalluscribeServer {
    pub fn new(archive_dir: PathBuf) -> Self {
        Self { archive_dir }
    }
}

impl HalluscribeServer {
    // Plain, macro-free handler bodies. Kept separate from the `#[tool]`
    // methods below so unit tests can exercise the actual logic (JSON
    // shaping, not-found mapping) without going through the MCP parameter
    // extraction / router machinery.
    fn do_search_sessions(&self, request: SearchSessionsRequest) -> Result<String, McpError> {
        let params = SearchParams {
            query: request.query,
            date_from: request.date_from,
            date_to: request.date_to,
            tags: request.tags,
            project: request.project,
            tool: request.tool,
            limit: request.limit,
            offset: request.offset,
        };
        let page = search::search_sessions_page(&self.archive_dir, &params, None);
        serde_json::to_string_pretty(&page)
            .map_err(|error| McpError::internal_error(error.to_string(), None))
    }

    fn do_read_session(&self, session_id: &str) -> Result<String, McpError> {
        search::read_session(&self.archive_dir, session_id)
            .map_err(|error| McpError::resource_not_found(error, None))
    }

    fn do_get_profile(&self, scope: ProfileScope) -> String {
        profile::read_profile_md(&self.archive_dir, scope)
            .unwrap_or_else(|| "No profile has been built yet.".to_string())
    }

    fn do_get_digest(
        &self,
        scope: ProfileScope,
        offset: usize,
        max_chars: Option<usize>,
    ) -> String {
        let Some(full) = profile::latest_digest(&self.archive_dir, scope) else {
            return "No digest has been generated yet.".to_string();
        };
        let max = max_chars
            .unwrap_or(DIGEST_DEFAULT_MAX_CHARS)
            .clamp(1, DIGEST_MAX_CHARS_CAP);
        let total = full.len();
        // Whole digest fits in one default-shaped call: return it verbatim so
        // small archives keep the original (headerless) behaviour.
        if offset == 0 && total <= max {
            return full;
        }
        let start = floor_char_boundary(&full, offset);
        let end = floor_char_boundary(&full, (start.saturating_add(max)).min(total));
        let continuation = if end < total {
            format!("continue with offset={end}")
        } else {
            "end of digest".to_string()
        };
        format!(
            "[digest slice bytes {start}..{end} of {total}; {continuation}]\n{}",
            &full[start..end]
        )
    }
}

#[tool_router]
impl HalluscribeServer {
    #[tool(
        description = "Search archived AI coding sessions (Claude Code, Codex, Continue, Forge, and imported chat exports) by query text, date range, tags, project, or source tool. Multi-word `query` values are AND-of-keywords: every term must appear somewhere in the session (any field, any position) - it does NOT have to be a contiguous phrase, so \"has the Forge bridge been implemented\" and \"Forge MCP client bridge\" both match a session titled about the Forge MCP bridge even though neither is a literal substring of the title. Wrap the query in `\"double quotes\"` to force exact-phrase (contiguous substring) matching instead. Results are relevance-ranked: sessions with query terms in the title or tags rank above sessions where the terms only appear in the body. 1-3 short, distinctive keywords work best - long natural-language questions still work but add noise. Returns a JSON page: {searched, total_matches, returned, offset, results}. `results` are session INDEX ENTRIES (title, tags, tool, date, id) - not full transcripts; use read_session for the body. When reporting how many sessions were examined, always report `searched`, never `total_matches` or `returned`. Never claim the results are complete when `total_matches` exceeds `returned` - request further pages with `offset` instead. `limit` defaults to 20 and is capped at 30."
    )]
    fn search_sessions(
        &self,
        Parameters(request): Parameters<SearchSessionsRequest>,
    ) -> Result<String, McpError> {
        self.do_search_sessions(request)
    }

    #[tool(
        description = "Read the full redaction-applied Markdown content of one archived session, given its session_id (the `id` field from a search_sessions result)."
    )]
    fn read_session(
        &self,
        Parameters(request): Parameters<ReadSessionRequest>,
    ) -> Result<String, McpError> {
        self.do_read_session(&request.session_id)
    }

    #[tool(
        description = "Get the distilled profile.md for the requested scope: identity, projects, conventions, recurring problems, communication style, and timeline distilled from the user's archived sessions. Optional `scope` argument: \"work\" (default) or \"personal\". Personal is a SUPERSET that also carries private-chat-derived (ChatGPT/Claude.ai/Gemini) life context - request it only when that exposure is intended. Returns a placeholder message if no profile has been built yet for the requested scope."
    )]
    fn get_profile(
        &self,
        Parameters(request): Parameters<ScopeRequest>,
    ) -> Result<String, McpError> {
        Ok(self.do_get_profile(scope_or_work(request.scope)))
    }

    #[tool(
        description = "Get the most recent weekly digest for the requested profile scope: sessions distilled, breakdowns by project/type, top new error tags, and new facts folded into the profile that week. Optional `scope` argument: \"work\" (default) or \"personal\" (a superset also carrying private-chat-derived life context). Large digests are PAGED: at most `max_chars` bytes (default 20000, cap 50000) are returned per call, prefixed by a header line naming the byte range and the `offset` to continue from; the statistics live at the top, so the first page alone usually suffices. Returns a placeholder message if no digest has been generated yet for the requested scope."
    )]
    fn get_digest(
        &self,
        Parameters(request): Parameters<DigestRequest>,
    ) -> Result<String, McpError> {
        Ok(self.do_get_digest(
            scope_or_work(request.scope),
            request.offset.unwrap_or(0),
            request.max_chars,
        ))
    }
}

#[tool_handler]
impl ServerHandler for HalluscribeServer {
    fn get_info(&self) -> ServerInfo {
        // Not `Implementation::from_build_env()`: that macro expands inside
        // the rmcp crate and reports "rmcp"/"2.1.0" as the server identity.
        let mut implementation = Implementation::from_build_env();
        implementation.name = "halluscribe-mcp".to_string();
        implementation.version = env!("CARGO_PKG_VERSION").to_string();
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(implementation)
            .with_instructions(
                "HalluScribe read-only archive server. Gives every future agent session memory \
                 of all previous ones: search_sessions and read_session query the user's archived \
                 AI coding sessions; get_profile and get_digest return the distilled profile for \
                 the requested scope (work by default, or personal - a superset that also carries \
                 private-chat-derived life context). No write, redact, or delete tools are exposed."
                    .to_string(),
            )
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod server_tests;
