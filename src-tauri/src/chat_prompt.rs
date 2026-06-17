// HalluScribe - chat system prompt builder for archive and web-assisted chat.

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
}

pub(crate) fn build_chat_system_prompt(context: &ChatPromptContext) -> String {
    let image_policy = if context.has_images {
        "Image policy:\nInspect the attached image directly before answering any image-related part of the request. Describe only what is actually visible, and say when text or details are unreadable."
    } else {
        "Image policy:\nNo image is attached in this turn."
    };
    let mixed_evidence_requirements = mixed_evidence_requirements(context);
    format!(
        "You are HalluScribe's developer archive assistant.\n\n\
         Core rules:\n\
         - Treat tool outputs as the authoritative evidence. Do not answer from unstated general knowledge or training memory.\n\
         - Separate observed facts from your own inference. If you infer something, say that it is an inference.\n\
         - Do not overstate the archive evidence. If a session shows investigation, mitigation, workaround, port change, or partial progress, do not rewrite that as a full fix unless the session explicitly supports that conclusion.\n\
         - If the available evidence is incomplete or absent, say so directly.\n\n\
         {}\n\n\
         {}\n\n\
         {}\n\n\
         {}\n\n\
         {}\n\n\
         Tool strategy:\n\
         - For questions about the user's past sessions, bugs, decisions, code history, projects, or implementation details, use archive tools first.\n\
         - Start with `search_sessions` using a relevant keyword query.\n\
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

fn mixed_evidence_requirements(context: &ChatPromptContext) -> &'static str {
    if context.web_search_available && matches!(context.search_mode, SearchModePrompt::Semantic) {
        "Mixed-evidence requirements:\nWhen both archive evidence and web evidence are relevant, keep them separate and explicit. Use these exact sections in the final answer: `Archive Evidence:`, `Current Web Evidence:`, `Comparison:`, `Recommendation:`, and `Sources:`."
    } else {
        "Mixed-evidence requirements:\nIf both archive evidence and web evidence are relevant, keep them clearly separated."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_only_prompt_excludes_web_evidence() {
        let prompt = build_chat_system_prompt(&ChatPromptContext {
            web_search_available: false,
            has_images: false,
            search_mode: SearchModePrompt::Archive,
            scope_size: None,
        });
        assert!(prompt.contains("only allowed evidence source is the user's session archive"));
        assert!(!prompt.contains("two evidence sources"));
    }

    #[test]
    fn semantic_prompt_preserves_precision_rule() {
        let prompt = build_chat_system_prompt(&ChatPromptContext {
            web_search_available: true,
            has_images: false,
            search_mode: SearchModePrompt::Semantic,
            scope_size: Some(5),
        });
        assert!(prompt.contains("do not rewrite that as a full fix"));
        assert!(prompt.contains("semantically relevant sessions"));
        assert!(prompt.contains("exactly 5 session(s)"));
        assert!(prompt.contains("Archive Evidence:"));
        assert!(prompt.contains("Current Web Evidence:"));
    }
}
