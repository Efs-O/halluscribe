// HalluScribe - profile scope: WORK vs PERSONAL profile selection, each with
// its own on-disk layout, section skeleton, and source consent list. See
// docs/internal/PERSONA_PROTOCOL_PLAN.md Phase 2c.

use super::types::ProfileSection;
use serde::{Deserialize, Serialize};

/// Which of the two profiles (Work or Personal) an operation targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileScope {
    Work,
    Personal,
}

const WORK_SECTIONS: [ProfileSection; 6] = [
    ProfileSection::Identity,
    ProfileSection::Projects,
    ProfileSection::Conventions,
    ProfileSection::RecurringProblems,
    ProfileSection::CommunicationStyle,
    ProfileSection::Timeline,
];

const PERSONAL_SECTIONS: [ProfileSection; 4] = [
    ProfileSection::Identity,
    ProfileSection::PersonalContext,
    ProfileSection::CommunicationStyle,
    ProfileSection::Timeline,
];

/// Chat-export providers folded into the Personal scope's source list in
/// addition to the user's configured `profile_sources`. Work scope never
/// sees these unless the user explicitly added them to `profile_sources`.
const PERSONAL_EXTRA_SOURCES: [&str; 4] = ["chatgpt", "claude_ai", "gemini", "grok"];

impl ProfileScope {
    /// Stable machine-readable key: also the on-disk directory name
    /// (`profile/work/`, `profile/personal/`) and the Tauri command
    /// `scope` argument value.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Personal => "personal",
        }
    }

    /// On-disk directory name under `<archive_dir>/profile/`. Same string as
    /// `as_str()` today, kept as a distinct method so disk layout and wire
    /// format can diverge later without a signature change.
    pub fn dir_name(&self) -> &'static str {
        self.as_str()
    }

    pub fn from_key(value: &str) -> Option<Self> {
        match value {
            "work" => Some(Self::Work),
            "personal" => Some(Self::Personal),
            _ => None,
        }
    }

    /// The fixed section skeleton this scope's profile.md is built from.
    /// Work is the original six-section skeleton (unchanged order); Personal
    /// is a life/character skeleton — Identity, Interests & Life Context
    /// (`PersonalContext`), Communication Style, and Timeline — dropping the
    /// coding-only sections (Projects, Conventions, RecurringProblems).
    pub fn sections(&self) -> &'static [ProfileSection] {
        match self {
            Self::Work => &WORK_SECTIONS,
            Self::Personal => &PERSONAL_SECTIONS,
        }
    }
}

/// Sources consented for `scope`, given the user's configured
/// `profile_sources` setting. Work is exactly `profile_sources`; Personal is
/// the union of `profile_sources` and the personal-chat-export providers
/// (deduplicated, `profile_sources`' order preserved, extras appended).
pub fn sources_for_scope(profile_sources: &[String], scope: ProfileScope) -> Vec<String> {
    let mut sources: Vec<String> = profile_sources.to_vec();
    if scope == ProfileScope::Personal {
        for extra in PERSONAL_EXTRA_SOURCES {
            if !sources.iter().any(|s| s == extra) {
                sources.push(extra.to_string());
            }
        }
    }
    sources
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_key_round_trips() {
        for scope in [ProfileScope::Work, ProfileScope::Personal] {
            assert_eq!(ProfileScope::from_key(scope.as_str()), Some(scope));
        }
    }

    #[test]
    fn scope_from_key_rejects_unknown() {
        assert_eq!(ProfileScope::from_key("nonsense"), None);
    }

    #[test]
    fn work_sections_exclude_personal_context() {
        let sections = ProfileScope::Work.sections();
        assert_eq!(sections.len(), 6);
        assert!(!sections.contains(&ProfileSection::PersonalContext));
    }

    #[test]
    fn personal_sections_are_life_focused() {
        let sections = ProfileScope::Personal.sections();
        assert_eq!(sections.len(), 4);
        assert_eq!(sections[0], ProfileSection::Identity);
        assert_eq!(sections[1], ProfileSection::PersonalContext);
        assert!(!sections.contains(&ProfileSection::Projects));
        assert!(!sections.contains(&ProfileSection::Conventions));
        assert!(!sections.contains(&ProfileSection::RecurringProblems));
    }

    #[test]
    fn sources_for_scope_work_is_unchanged() {
        let configured = vec!["claude_code".to_string(), "codex".to_string()];
        let sources = sources_for_scope(&configured, ProfileScope::Work);
        assert_eq!(sources, configured);
    }

    #[test]
    fn sources_for_scope_personal_adds_chat_exports() {
        let configured = vec!["claude_code".to_string()];
        let sources = sources_for_scope(&configured, ProfileScope::Personal);
        assert!(sources.contains(&"claude_code".to_string()));
        assert!(sources.contains(&"chatgpt".to_string()));
        assert!(sources.contains(&"claude_ai".to_string()));
        assert!(sources.contains(&"gemini".to_string()));
        assert!(sources.contains(&"grok".to_string()));
        assert_eq!(sources.len(), 5);
    }

    #[test]
    fn sources_for_scope_personal_dedups_already_configured_extras() {
        let configured = vec!["claude_code".to_string(), "chatgpt".to_string()];
        let sources = sources_for_scope(&configured, ProfileScope::Personal);
        assert_eq!(
            sources.iter().filter(|s| s.as_str() == "chatgpt").count(),
            1
        );
        assert_eq!(sources.len(), 5);
    }
}
