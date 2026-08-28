// HalluScribe - per-provider project attribution for an archived session.
// Each coding tool stores sessions under a different layout, so the project a
// session belongs to has to be recovered differently for each one. Every
// resolver falls back rather than guessing: a wrong project label is worse than
// the tool name, because it silently merges unrelated work in the index.

use std::path::Path;

/// Claude Code writes `projects/<slug>/*.jsonl`, and subagent transcripts one
/// level deeper at `projects/<slug>/<parent-session-id>/subagents/*.jsonl`.
/// Walking up to whichever ancestor sits directly under `projects` handles both
/// depths without special-casing the `subagents` folder name.
pub(super) fn claude_code(source_path: &Path) -> Option<String> {
    let mut dir = source_path.parent()?;
    loop {
        let parent = dir.parent()?;
        if parent.file_name().and_then(|name| name.to_str()) == Some("projects") {
            return dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(ToOwned::to_owned);
        }
        dir = parent;
    }
}

/// Codex writes `sessions/YYYY/MM/DD/rollout-*.jsonl`, so the path carries a
/// date and nothing else. The real workspace is in the header line instead:
/// `{"type":"session_meta","payload":{"cwd":"n:\\vs code apps\\Forge",...}}`.
pub(super) fn codex(source_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(source_path).ok()?;
    let cwd = content
        .lines()
        .take(SCANNED_HEADER_LINES)
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|value| value.get("type").and_then(serde_json::Value::as_str) == Some("session_meta"))
        .and_then(|value| {
            value
                .get("payload")?
                .get("cwd")?
                .as_str()
                .map(ToOwned::to_owned)
        })?;
    last_path_component(&cwd)
}

/// Forge writes every session into one flat `~/.forge/sessions` folder, so the
/// path holds nothing. Forge >= 0.13.17 records the workspace in its
/// `session_start` header; sessions written before that have no project.
pub(super) fn forge(source_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(source_path).ok()?;
    content
        .lines()
        .take(SCANNED_HEADER_LINES)
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|value| {
            value.get("type").and_then(serde_json::Value::as_str) == Some("session_start")
        })
        .and_then(|value| {
            let name = value.get("workspace_name")?.as_str()?.trim().to_string();
            (!name.is_empty()).then_some(name)
        })
}

/// Both headers are the first line of the file. A handful of lines of slack
/// covers a logger that ever prepends to it, without reading a 12 MB session
/// into memory looking for a field that is not there.
const SCANNED_HEADER_LINES: usize = 8;

/// The recorded cwd is whatever the tool ran on — typically a Windows path,
/// which `Path` will not segment when this runs on Linux. Splitting on both
/// separators keeps the label correct across the archive's origin machines.
fn last_path_component(cwd: &str) -> Option<String> {
    cwd.trim()
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn claude_code_takes_the_slug_directly_under_projects() {
        let path = PathBuf::from("/home/u/.claude/projects/n--vs-code-apps-Forge/abc.jsonl");
        assert_eq!(claude_code(&path).as_deref(), Some("n--vs-code-apps-Forge"));
    }

    #[test]
    fn claude_code_walks_past_subagents_to_the_slug() {
        let path = PathBuf::from(
            "/home/u/.claude/projects/n--vs-code-apps-Forge/6a40c131/subagents/abc.jsonl",
        );
        assert_eq!(claude_code(&path).as_deref(), Some("n--vs-code-apps-Forge"));
    }

    #[test]
    fn claude_code_returns_none_outside_a_projects_tree() {
        let path = PathBuf::from("/tmp/loose/abc.jsonl");
        assert_eq!(claude_code(&path), None);
    }

    #[test]
    fn last_component_splits_windows_and_posix_paths() {
        assert_eq!(
            last_path_component(r"n:\vs code apps\Forge").as_deref(),
            Some("Forge")
        );
        assert_eq!(
            last_path_component("/home/u/projects/halluscribe/").as_deref(),
            Some("halluscribe")
        );
        assert_eq!(last_path_component("   "), None);
    }

    #[test]
    fn codex_reads_the_cwd_from_session_meta() {
        let dir = std::env::temp_dir().join("hs-codex-project-label");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-test.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"session_meta","payload":{"cwd":"n:\\vs code apps\\Forge"}}"#,
        )
        .unwrap();
        assert_eq!(codex(&path).as_deref(), Some("Forge"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn codex_returns_none_when_the_header_carries_no_cwd() {
        let dir = std::env::temp_dir().join("hs-codex-project-label");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-nocwd.jsonl");
        std::fs::write(&path, "{\"type\":\"session_meta\",\"payload\":{}}\n").unwrap();
        assert_eq!(codex(&path), None);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn forge_reads_the_workspace_name_from_session_start() {
        let dir = std::env::temp_dir().join("hs-forge-project-label");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"session_start\",\"workspace_name\":\"Halluscribe\"}\n",
        )
        .unwrap();
        assert_eq!(forge(&path).as_deref(), Some("Halluscribe"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn forge_returns_none_for_a_pre_0_13_17_session() {
        let dir = std::env::temp_dir().join("hs-forge-project-label");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("legacy.jsonl");
        std::fs::write(&path, "{\"type\":\"session_start\",\"title\":\"x\"}\n").unwrap();
        assert_eq!(forge(&path), None);
        std::fs::remove_file(&path).ok();
    }
}
