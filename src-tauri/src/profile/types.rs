// HalluScribe - profile domain types: facts, sections, and provenance metadata.

use serde::{Deserialize, Serialize};

/// The fixed section skeleton profile.md uses, per
/// docs/internal/PERSONA_PROTOCOL_PLAN.md Phase 2 (and Phase 2c for
/// `PersonalContext`, which only the Personal scope's skeleton includes —
/// see `ProfileScope::sections()` in `profile/scope.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSection {
    Identity,
    Projects,
    Conventions,
    RecurringProblems,
    CommunicationStyle,
    Timeline,
    PersonalContext,
}

impl ProfileSection {
    /// Every section across both scopes, for serde/round-trip purposes.
    /// Pipeline code that must respect a scope's skeleton uses
    /// `ProfileScope::sections()` instead.
    pub const ALL: [ProfileSection; 7] = [
        Self::Identity,
        Self::Projects,
        Self::Conventions,
        Self::RecurringProblems,
        Self::CommunicationStyle,
        Self::Timeline,
        Self::PersonalContext,
    ];

    /// Stable machine-readable key, also used as the tool-call enum value and
    /// as the `save_user_profile` object property name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Projects => "projects",
            Self::Conventions => "conventions",
            Self::RecurringProblems => "recurring_problems",
            Self::CommunicationStyle => "communication_style",
            Self::Timeline => "timeline",
            Self::PersonalContext => "personal_context",
        }
    }

    /// Human-readable markdown heading text (without the `##` prefix).
    pub fn heading(&self) -> &'static str {
        match self {
            Self::Identity => "Identity & Context",
            Self::Projects => "Active Projects",
            Self::Conventions => "Conventions & Preferences",
            Self::RecurringProblems => "Recurring Problems",
            Self::CommunicationStyle => "Communication Style",
            Self::Timeline => "Timeline Highlights",
            Self::PersonalContext => "Interests & Life Context",
        }
    }

    pub fn from_key(value: &str) -> Option<Self> {
        match value {
            "identity" => Some(Self::Identity),
            "projects" => Some(Self::Projects),
            "conventions" => Some(Self::Conventions),
            "recurring_problems" => Some(Self::RecurringProblems),
            "communication_style" => Some(Self::CommunicationStyle),
            "timeline" => Some(Self::Timeline),
            "personal_context" => Some(Self::PersonalContext),
            _ => None,
        }
    }
}

/// One candidate fact extracted by the map step, evidenced by session ids.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileFact {
    pub section: ProfileSection,
    pub fact: String,
    /// Session ids this fact is grounded in (e.g. `["abc-123"]`).
    pub evidence: Vec<String>,
    /// Date associated with the fact (usually the evidencing session's date).
    pub date: String,
}

/// The merged reduce-step output: one block of prose per section, ready to
/// drop into the profile.md skeleton.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileSections {
    pub identity: String,
    pub projects: String,
    pub conventions: String,
    pub recurring_problems: String,
    pub communication_style: String,
    pub timeline: String,
    pub personal_context: String,
}

impl ProfileSections {
    pub fn get(&self, section: ProfileSection) -> &str {
        match section {
            ProfileSection::Identity => &self.identity,
            ProfileSection::Projects => &self.projects,
            ProfileSection::Conventions => &self.conventions,
            ProfileSection::RecurringProblems => &self.recurring_problems,
            ProfileSection::CommunicationStyle => &self.communication_style,
            ProfileSection::Timeline => &self.timeline,
            ProfileSection::PersonalContext => &self.personal_context,
        }
    }

    pub fn set(&mut self, section: ProfileSection, value: String) {
        match section {
            ProfileSection::Identity => self.identity = value,
            ProfileSection::Projects => self.projects = value,
            ProfileSection::Conventions => self.conventions = value,
            ProfileSection::RecurringProblems => self.recurring_problems = value,
            ProfileSection::CommunicationStyle => self.communication_style = value,
            ProfileSection::Timeline => self.timeline = value,
            ProfileSection::PersonalContext => self.personal_context = value,
        }
    }
}

/// Watermark + provenance persisted alongside profile.md so incremental
/// refreshes know what has already been distilled.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub generated_at: String,
    /// RFC3339 timestamp (or empty) of the newest session folded into the
    /// profile so far. Incremental refreshes only map sessions newer than this.
    #[serde(default)]
    pub last_distilled_ts: String,
    pub session_count: usize,
    pub sources: Vec<String>,
    pub facts_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_str_round_trips_for_all_variants() {
        for section in ProfileSection::ALL {
            let key = section.as_str();
            assert_eq!(ProfileSection::from_key(key), Some(section));
        }
    }

    #[test]
    fn section_from_key_rejects_unknown() {
        assert_eq!(ProfileSection::from_key("nonsense"), None);
    }

    #[test]
    fn section_headings_are_distinct() {
        let mut headings: Vec<&str> = ProfileSection::ALL.iter().map(|s| s.heading()).collect();
        let before = headings.len();
        headings.sort_unstable();
        headings.dedup();
        assert_eq!(headings.len(), before);
    }

    #[test]
    fn profile_sections_get_set_round_trip() {
        let mut sections = ProfileSections::default();
        for section in ProfileSection::ALL {
            sections.set(section, format!("text-{}", section.as_str()));
        }
        for section in ProfileSection::ALL {
            assert_eq!(sections.get(section), format!("text-{}", section.as_str()));
        }
    }

    #[test]
    fn profile_fact_serde_roundtrip() {
        let fact = ProfileFact {
            section: ProfileSection::RecurringProblems,
            fact: "Hits exit code 127 across shells".to_string(),
            evidence: vec!["abc-123".to_string(), "def-456".to_string()],
            date: "2026-06-01".to_string(),
        };
        let json = serde_json::to_string(&fact).unwrap();
        assert!(json.contains("\"recurring_problems\""));
        let back: ProfileFact = serde_json::from_str(&json).unwrap();
        assert_eq!(back.fact, fact.fact);
        assert_eq!(back.evidence, fact.evidence);
        assert_eq!(back.section, ProfileSection::RecurringProblems);
    }

    #[test]
    fn profile_meta_serde_roundtrip() {
        let meta = ProfileMeta {
            generated_at: "2026-07-03T00:00:00Z".to_string(),
            last_distilled_ts: "2026-06-30T00:00:00Z".to_string(),
            session_count: 42,
            sources: vec!["claude_code".to_string(), "codex".to_string()],
            facts_count: 17,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: ProfileMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.session_count, 42);
        assert_eq!(back.facts_count, 17);
        assert_eq!(back.sources, meta.sources);
    }

    #[test]
    fn profile_meta_last_distilled_ts_defaults_when_absent() {
        let meta: ProfileMeta = serde_json::from_str(
            r#"{"generated_at":"2026-07-03T00:00:00Z","session_count":1,"sources":[],"facts_count":0}"#,
        )
        .unwrap();
        assert_eq!(meta.last_distilled_ts, "");
    }
}
