/// All fields are optional - omitted fields match every entry.
#[derive(Debug, Default)]
pub struct SearchParams {
    /// Text matched (case-insensitive) against title, error_tags, and topic_tags.
    pub query: Option<String>,
    /// ISO date lower bound (YYYY-MM-DD, inclusive).
    pub date_from: Option<String>,
    /// ISO date upper bound (YYYY-MM-DD, inclusive).
    pub date_to: Option<String>,
    /// Every tag listed must appear in error_tags or topic_tags.
    pub tags: Option<Vec<String>>,
    /// Substring match (case-insensitive) against the project field.
    pub project: Option<String>,
    /// Tool filter: "claude_code", "codex", "forge", or any substring.
    pub tool: Option<String>,
    /// Max results returned (default 20, hard-capped at 30).
    pub limit: Option<usize>,
    /// Number of leading matches to skip before the returned page (default 0).
    /// Used with `limit` to paginate through matches larger than one page.
    pub offset: Option<usize>,
}
