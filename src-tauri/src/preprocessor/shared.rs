// HalluScribe - shared preprocessing helpers.

use serde_json::Value;

pub(super) fn extract_content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .map(|b| {
                b.get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub(super) fn head_tail_5(text: &str) -> String {
    head_tail(text, 5)
}

pub(super) fn head_tail(text: &str, side_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let keep = side_lines.saturating_mul(2);
    if lines.len() <= keep {
        return text.to_string();
    }
    let omitted = lines.len() - keep;
    format!(
        "{}\n... ({omitted} lines omitted) ...\n{}",
        lines[..side_lines].join("\n"),
        lines[lines.len() - side_lines..].join("\n")
    )
}

pub(super) fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{out}...[truncated]")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_tail_short_passthrough() {
        let t = "a\nb\nc";
        assert_eq!(head_tail_5(t), t);
    }

    #[test]
    fn head_tail_exactly_10_passthrough() {
        let t = (1..=10)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(head_tail_5(&t), t);
    }

    #[test]
    fn head_tail_long_omits_middle() {
        let t = (1..=20)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let r = head_tail_5(&t);
        assert!(r.contains("omitted"));
        assert!(r.starts_with("1\n"));
        assert!(r.ends_with("20"));
        assert!(!r.contains("\n6\n"), "line 6 must be omitted");
    }

    #[test]
    fn head_tail_with_custom_window_keeps_more_lines() {
        let t = (1..=30)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let r = head_tail(&t, 10);
        assert!(r.contains("omitted"));
        assert!(r.contains("\n10\n"));
        assert!(r.contains("\n21\n"));
        assert!(!r.contains("\n11\n"), "middle lines must still be omitted");
    }

    #[test]
    fn truncate_short_passthrough() {
        assert_eq!(truncate_text("hello", 100), "hello");
    }

    #[test]
    fn truncate_long_marks_truncation() {
        let r = truncate_text(&"a".repeat(200), 50);
        assert!(r.contains("truncated"));
        assert!(r.starts_with(&"a".repeat(50)));
    }

    #[test]
    fn truncate_unicode_safe() {
        let r = truncate_text(&"cafΓ©".repeat(100), 7);
        assert!(r.contains("truncated"));
    }

    #[test]
    fn extract_content_text_string_value() {
        let v = Value::String("hello".into());
        assert_eq!(extract_content_text(Some(&v)), "hello");
    }

    #[test]
    fn extract_content_text_array_keeps_text_blocks_only() {
        let v: Value =
            serde_json::from_str(r#"[{"type":"text","text":"hi"},{"type":"image","data":"..."}]"#)
                .unwrap();
        assert_eq!(extract_content_text(Some(&v)), "hi");
    }
}
