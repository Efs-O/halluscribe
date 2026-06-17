use super::{
    build_session, parse_rfc3339, read_json, stable_hash, ChatProvider, MessageRole, ParsedMessage,
    ParsedSession, ReaderError,
};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct GeminiRecord {
    title: Option<String>,
    time: Option<String>,
    header: Option<String>,
    #[serde(default, rename = "safeHtmlItem")]
    safe_html_item: Vec<GeminiHtmlItem>,
}

#[derive(Deserialize)]
struct GeminiHtmlItem {
    html: Option<String>,
}

pub fn read(path: &Path) -> Result<Vec<ParsedSession>, ReaderError> {
    let content = read_json(path)?;
    let records: Vec<GeminiRecord> = serde_json::from_str(&content)?;
    let mut sessions = Vec::new();

    for record in records {
        let Some(time_raw) = record.time.as_deref() else {
            continue;
        };
        let Some(created_at) = parse_rfc3339(time_raw) else {
            continue;
        };

        let raw_title = record.title.unwrap_or_default();
        let prompt = raw_title
            .strip_prefix("Prompted ")
            .map(normalize_prompt_text)
            .unwrap_or_else(|| normalize_prompt_text(raw_title.trim()));

        let response = record
            .safe_html_item
            .iter()
            .filter_map(|item| item.html.as_deref())
            .map(html_to_text)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");

        let mut messages = Vec::new();
        if !prompt.is_empty() {
            messages.push(ParsedMessage {
                role: MessageRole::User,
                text: prompt.clone(),
                timestamp: Some(created_at),
            });
        }
        if !response.is_empty() {
            messages.push(ParsedMessage {
                role: MessageRole::Assistant,
                text: response,
                timestamp: Some(created_at),
            });
        }
        if messages.is_empty() {
            continue;
        }

        let title = if prompt.is_empty() {
            record
                .header
                .map(|header| collapse_inline_whitespace(&header))
                .filter(|header| !header.is_empty())
                .unwrap_or_else(|| "Gemini activity".to_string())
        } else {
            summarize_prompt_as_title(&prompt)
        };
        let hash = stable_hash(&title);
        let id = format!(
            "gemini-{}-{}",
            created_at.format("%Y-%m-%dT%H-%M-%S%.3fZ"),
            &hash[..8]
        );
        let total_chars: usize = messages.iter().map(|message| message.text.len()).sum();
        let fill_pct = ((total_chars as f64 / 3_000_000.0) * 100.0).clamp(0.0, 100.0);

        if let Some(session) = build_session(
            id,
            title,
            created_at,
            None,
            path.to_path_buf(),
            ChatProvider::Gemini,
            fill_pct,
            true,
            messages,
        ) {
            sessions.push(session);
        }
    }

    Ok(sessions)
}

fn normalize_prompt_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn summarize_prompt_as_title(prompt: &str) -> String {
    let first_line = prompt
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(prompt);
    let collapsed = collapse_inline_whitespace(first_line);
    if collapsed.chars().count() <= 120 {
        collapsed
    } else {
        let shortened = collapsed.chars().take(117).collect::<String>();
        format!("{shortened}...")
    }
}

fn collapse_inline_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn html_to_text(html: &str) -> String {
    let mut text = html
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</p>", "\n")
        .replace("</div>", "\n");

    let mut stripped = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => stripped.push(ch),
            _ => {}
        }
    }

    text = stripped
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
        .replace("&quot;", "\"");

    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::read;
    use crate::readers::{ChatProvider, MessageRole};
    use std::fs;

    fn write_fixture(content: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("My Activity.json"), content).expect("write fixture");
        dir
    }

    #[test]
    fn reads_prompt_and_html_response_into_two_turn_session() {
        let dir = write_fixture(
            r#"[
              {
                "header": "Gemini Apps",
                "title": "Prompted Best islands in Greece?",
                "time": "2026-04-20T18:24:31.187Z",
                "safeHtmlItem": [
                  { "html": "<p>Naxos &amp; Crete</p><div>Try shoulder season.<br />Less crowded.</div>" }
                ]
              }
            ]"#,
        );

        let sessions = read(&dir.path().join("My Activity.json")).expect("parse gemini export");
        assert_eq!(sessions.len(), 1);

        let session = &sessions[0];
        assert!(session.id.starts_with("gemini-2026-04-20T18-24-31.187Z-"));
        assert_eq!(session.title, "Best islands in Greece?");
        assert_eq!(session.provider, ChatProvider::Gemini);
        assert!(session.fill_estimated);
        assert_eq!(session.messages.len(), 2);
        assert!(matches!(session.messages[0].role, MessageRole::User));
        assert_eq!(session.messages[0].text, "Best islands in Greece?");
        assert!(matches!(session.messages[1].role, MessageRole::Assistant));
        assert_eq!(
            session.messages[1].text,
            "Naxos & Crete\nTry shoulder season.\nLess crowded."
        );
    }

    #[test]
    fn trims_noisy_multiline_titles_to_first_meaningful_line() {
        let dir = write_fixture(
            r#"[
              {
                "header": "Gemini Apps",
                "title": "Prompted WHICH DO I SELECT : \n\nAI Mode\n\nChrome\n\nDiscover",
                "time": "2026-04-20T18:24:31.187Z",
                "safeHtmlItem": [
                  { "html": "<p>Select My Activity, then only Gemini Apps.</p>" }
                ]
              }
            ]"#,
        );

        let sessions = read(&dir.path().join("My Activity.json")).expect("parse gemini export");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "WHICH DO I SELECT :");
        assert_eq!(
            sessions[0].messages[0].text,
            "WHICH DO I SELECT :\nAI Mode\nChrome\nDiscover"
        );
    }
}
