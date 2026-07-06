// HalluScribe - chat system prompt builder for archive and web-assisted chat.

use crate::profile::ProfileScope;

/// Defensive cap on how much of the distilled profile is embedded in the
/// system prompt. The current profile is ~10 KB; this only guards against a
/// future profile blowing the context budget.
const PROFILE_PROMPT_MAX_CHARS: usize = 24_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchModePrompt {
    Archive,
    Semantic,
}

pub(crate) struct ChatPromptContext {
    pub web_search_available: bool,
    pub has_images: bool,
    pub search_mode: SearchModePrompt,
    pub scope_size: Option<usize>,
    pub profile: Option<String>,
    pub profile_scope: ProfileScope,
}

pub(crate) fn build_chat_system_prompt(context: &ChatPromptContext) -> String {
    let image_policy = if context.has_images {
        "Image policy:\nInspect the attached image directly before answering any image-related part of the request. Describe only what is actually visible, and say when text or details are unreadable."
    } else {
        "Image policy:\nNo image is attached in this turn."
    };
    let mixed_evidence_requirements = mixed_evidence_requirements(context);
    let profile_section =
        profile_section(&context.profile, context.search_mode, context.profile_scope);
    format!(
        "You are HalluScribe's developer archive assistant.\n\n\
         Core rules:\n\
         - Treat tool outputs as the authoritative evidence. Do not answer from unstated general knowledge or training memory. The distilled user profile below (when present) is stated evidence, not training memory or unstated general knowledge, and may be used directly.\n\
         - Separate observed facts from your own inference. If you infer something, say that it is an inference.\n\
         - Do not overstate the archive evidence. If a session shows investigation, mitigation, workaround, port change, or partial progress, do not rewrite that as a full fix unless the session explicitly supports that conclusion.\n\
         - If the available evidence is incomplete or absent, say so directly.\n\n\
         {}\n\n\
         {}\n\n\
         {}\n\n\
         {}\n\n\
         {}\n\n\
         {}\n\n\
         Tool strategy:\n\
         - For questions about the user's past sessions, bugs, decisions, code history, projects, or implementation details, use archive tools first.\n\
         - Start with `search_sessions` using a relevant keyword query.\n\
         - `search_sessions` returns `{{searched, total_matches, returned, offset, results}}`. `searched` is how many sessions were examined (the whole archive scope); `total_matches` is how many matched the query; `results` is only one capped page. If the user asks how many sessions you searched, answer with `searched` (not `total_matches`). When `total_matches` is greater than `returned`, there are more matches than you have seen: never state or imply you searched every session or found them all. Report the real numbers (e.g. \"Searched 1368 sessions; 147 matched; showing the 30 newest\") and, when completeness matters, page through the rest by re-calling with an increasing `offset` before concluding.\n\
         - If that returns no useful hits, call `search_sessions` with no query to list recent sessions, then use `read_session` on the most relevant sessions.\n\
         - Never stop after one empty archive search when archive evidence is needed.\n\
         - For current external facts, releases, APIs, documentation, or news, use `web_search` and `web_fetch` before answering when those tools are available.\n\
         - If the question needs both archive evidence and current external evidence, use both and keep them clearly separated in the answer.\n\n\
         Answer requirements:\n\
         - Prefer concise, evidence-grounded answers.\n\
         - When answering from archive evidence, preserve the session's actual state: fixed, worked around, investigated, unresolved, or blocked.\n\
         - If web tools were used, end the answer with a `Sources:` section that lists the exact URLs used, one per line.\n\
         - If after appropriate tool use the evidence still does not support an answer, say that explicitly.",
        evidence_policy(context.web_search_available),
        mode_policy(context.search_mode),
        scope_policy(context.scope_size),
        profile_section,
        image_policy,
        mixed_evidence_requirements,
    )
}

fn evidence_policy(web_search_available: bool) -> &'static str {
    if web_search_available {
        "Evidence policy:\nYou may use two evidence sources: the user's session archive and the web tools. Do not claim current external facts without a web tool call. When you use web results, distinguish sourced facts from your own synthesis."
    } else {
        "Evidence policy:\nYour only allowed evidence source is the user's session archive. You have no current external knowledge and must not answer from general background knowledge."
    }
}

fn mode_policy(search_mode: SearchModePrompt) -> &'static str {
    match search_mode {
        SearchModePrompt::Archive => {
            "Mode policy:\nArchive mode is active. You may search the full allowed archive scope using the archive tools."
        }
        SearchModePrompt::Semantic => {
            "Mode policy:\nSemantic mode is active. The archive scope has already been narrowed to semantically relevant sessions. Stay inside that narrowed set and do not silently fall back to archive-wide retrieval."
        }
    }
}

fn scope_policy(scope_size: Option<usize>) -> String {
    match scope_size {
        Some(count) => format!(
            "Scope policy:\nYour allowed archive scope is restricted to exactly {count} session(s). Do not imply knowledge outside that scoped set."
        ),
        None => "Scope policy:\nYou may use the full archive scope available to this chat.".to_string(),
    }
}

/// Build the "User profile" section of the system prompt. Present or absent,
/// this section is always included so the overall prompt skeleton is stable.
pub(crate) fn profile_section(
    profile: &Option<String>,
    search_mode: SearchModePrompt,
    scope: ProfileScope,
) -> String {
    let scope_label = scope.as_str();
    let trimmed = profile.as_deref().map(str::trim).unwrap_or("");
    if trimmed.is_empty() {
        return format!(
            "User profile:\nNo distilled user profile is available for the selected scope ({scope_label})."
        );
    }

    let (content, truncated) = if trimmed.len() > PROFILE_PROMPT_MAX_CHARS {
        let mut end = PROFILE_PROMPT_MAX_CHARS;
        while !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        (&trimmed[..end], true)
    } else {
        (trimmed, false)
    };

    let semantic_note = if matches!(search_mode, SearchModePrompt::Semantic) {
        " This profile is background context about the user and does not widen the allowed tool-search scope for this turn."
    } else {
        ""
    };

    let truncated_line = if truncated {
        "\n[profile truncated for context budget]"
    } else {
        ""
    };

    format!(
        "User profile:\nThe following is the user's {scope_label} profile — a distilled, archive-wide profile of the user, generated by HalluScribe from the full session archive. It is legitimate, stated evidence about the user — their identity, projects, habits, conventions, and recurring problems — and may be used directly to answer questions about who the user is.{semantic_note} Claims in the profile carry session-id evidence references that can be verified with `read_session`; for specific or recent details, or when completeness matters, still verify with the archive tools rather than relying on the profile alone. The profile is a periodic snapshot and may lag behind the newest sessions.\n\n\
         --- BEGIN USER PROFILE ---\n\
         {content}{truncated_line}\n\
         --- END USER PROFILE ---"
    )
}

fn mixed_evidence_requirements(context: &ChatPromptContext) -> &'static str {
    if context.web_search_available && matches!(context.search_mode, SearchModePrompt::Semantic) {
        "Mixed-evidence requirements:\nWhen both archive evidence and web evidence are relevant, keep them separate and explicit. Use these exact sections in the final answer: `Archive Evidence:`, `Current Web Evidence:`, `Comparison:`, `Recommendation:`, and `Sources:`."
    } else {
        "Mixed-evidence requirements:\nIf both archive evidence and web evidence are relevant, keep them clearly separated."
    }
}

#[cfg(test)]
#[path = "chat_prompt_tests.rs"]
mod chat_prompt_tests;
