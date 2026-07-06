// HalluScribe - previous-profile section parser: split an existing profile.md
// back into its `ProfileSections` so the per-section reduce can hand each
// model call only that section's previous content. Round-trips the exact
// format `writer::build_profile_markdown` emits.

use super::types::{ProfileSection, ProfileSections};
use super::writer::EMPTY_SECTION_PLACEHOLDER;

/// Parse a profile.md body into per-section text. Splits on `## ` heading
/// lines, matches the heading text against `ProfileSection::heading()` for
/// every known section, and collects each section's body (trimmed). Unknown
/// headings are ignored; the writer's empty-section placeholder parses back
/// to an empty string so writer → parser round-trips.
pub(super) fn parse_profile_sections(md: &str) -> ProfileSections {
    let mut sections = ProfileSections::default();
    let mut current: Option<ProfileSection> = None;
    let mut body = String::new();

    let mut flush = |section: Option<ProfileSection>, body: &mut String| {
        if let Some(section) = section {
            let trimmed = body.trim();
            let text = if trimmed == EMPTY_SECTION_PLACEHOLDER {
                ""
            } else {
                trimmed
            };
            sections.set(section, text.to_string());
        }
        body.clear();
    };

    for line in md.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            flush(current, &mut body);
            let heading = heading.trim();
            current = ProfileSection::ALL
                .iter()
                .copied()
                .find(|section| section.heading() == heading);
        } else if current.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    flush(current, &mut body);
    sections
}

#[cfg(test)]
mod tests {
    use super::super::scope::ProfileScope;
    use super::super::types::{ProfileMeta, ProfileSections};
    use super::super::writer;
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("halluscribe_profile_parse_md_{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn round_trips_writer_output_for_both_scopes() {
        for scope in [ProfileScope::Work, ProfileScope::Personal] {
            let dir = tmp_dir(scope.as_str());
            let mut sections = ProfileSections::default();
            for section in scope.sections() {
                sections.set(
                    *section,
                    format!("Prose for {} [abc-123]", section.as_str()),
                );
            }
            let meta = ProfileMeta {
                generated_at: "2026-07-04T00:00:00Z".to_string(),
                session_count: 3,
                ..Default::default()
            };
            let path = writer::write_profile(&dir, scope, &sections, &meta).unwrap();
            let md = fs::read_to_string(path).unwrap();
            let parsed = parse_profile_sections(&md);
            for section in scope.sections() {
                assert_eq!(parsed.get(*section), sections.get(*section));
            }
        }
    }

    #[test]
    fn empty_section_placeholder_parses_back_to_empty_string() {
        let dir = tmp_dir("placeholder");
        let mut sections = ProfileSections::default();
        sections.set(ProfileSection::Identity, "Only identity set.".to_string());
        let meta = ProfileMeta {
            generated_at: "2026-07-04T00:00:00Z".to_string(),
            ..Default::default()
        };
        let path = writer::write_profile(&dir, ProfileScope::Work, &sections, &meta).unwrap();
        let md = fs::read_to_string(path).unwrap();
        let parsed = parse_profile_sections(&md);
        assert_eq!(parsed.get(ProfileSection::Identity), "Only identity set.");
        assert_eq!(parsed.get(ProfileSection::Projects), "");
        assert_eq!(parsed.get(ProfileSection::Timeline), "");
    }

    #[test]
    fn unknown_headings_are_ignored() {
        let md = "# User Profile — header\n\n## Not A Real Section\n\nignored text\n\n\
                  ## Identity & Context\n\nreal text\n";
        let parsed = parse_profile_sections(md);
        assert_eq!(parsed.get(ProfileSection::Identity), "real text");
        assert_eq!(parsed.get(ProfileSection::Projects), "");
    }

    #[test]
    fn multi_line_bodies_are_kept_and_trimmed() {
        let md = "## Recurring Problems\n\n- line one\n- line two\n\n";
        let parsed = parse_profile_sections(md);
        assert_eq!(
            parsed.get(ProfileSection::RecurringProblems),
            "- line one\n- line two"
        );
    }

    #[test]
    fn empty_input_yields_default_sections() {
        let parsed = parse_profile_sections("");
        for section in ProfileSection::ALL {
            assert_eq!(parsed.get(section), "");
        }
    }
}
