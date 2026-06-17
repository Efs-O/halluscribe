use crate::readers::ChatProvider;
use std::path::PathBuf;

/// The AI tool that produced a session file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum ToolSource {
    ClaudeCode,
    Codex,
    Continue,
    Forge,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum ScanTargetKind {
    Coding(ToolSource),
    Import(ChatProvider),
}

/// A source file that should be parsed into one or more HalluScribe sessions.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScanTarget {
    pub path: PathBuf,
    pub kind: ScanTargetKind,
    pub fill_pct: Option<f64>,
    pub mtime_secs: i64,
}
