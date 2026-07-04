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

    fn do_get_profile(&self) -> String {
        profile::read_profile_md(&self.archive_dir, ProfileScope::Work)
            .unwrap_or_else(|| "No profile has been built yet.".to_string())
    }

    fn do_get_digest(&self) -> String {
        profile::latest_digest(&self.archive_dir, ProfileScope::Work)
            .unwrap_or_else(|| "No digest has been generated yet.".to_string())
    }
}

#[tool_router]
impl HalluscribeServer {
    #[tool(
        description = "Search archived AI coding sessions (Claude Code, Codex, Continue, Forge, and imported chat exports) by query text, date range, tags, project, or source tool. Returns a JSON page: {searched, total_matches, returned, offset, results}. `results` are session INDEX ENTRIES (title, tags, tool, date, id) - not full transcripts; use read_session for the body. When reporting how many sessions were examined, always report `searched`, never `total_matches` or `returned`. Never claim the results are complete when `total_matches` exceeds `returned` - request further pages with `offset` instead. `limit` defaults to 20 and is capped at 30."
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
        description = "Get the distilled Work profile (profile.md): identity, projects, conventions, recurring problems, communication style, and timeline distilled from the user's archived sessions. Work scope only - the Personal profile is never exposed over MCP by design (v1 sharing rule). Returns a placeholder message if no profile has been built yet."
    )]
    fn get_profile(&self) -> Result<String, McpError> {
        Ok(self.do_get_profile())
    }

    #[tool(
        description = "Get the most recent weekly digest for the Work profile scope: sessions distilled, breakdowns by project/type, top new error tags, and new facts folded into the profile that week. Returns a placeholder message if no digest has been generated yet."
    )]
    fn get_digest(&self) -> Result<String, McpError> {
        Ok(self.do_get_digest())
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
                 AI coding sessions; get_profile and get_digest return the distilled Work profile. \
                 No write, redact, or delete tools are exposed."
                    .to_string(),
            )
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod server_tests;
