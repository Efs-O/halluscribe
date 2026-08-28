// HalluScribe - JSONL preprocessor.
// Strips noise and extracts signal from raw session files before Gemma.

mod claude;
mod codex;
mod forge;
mod shared;
#[cfg(test)]
mod token_tests;

use crate::scanner::ToolSource;
use crate::tokens::TokenCount;
use std::{fmt, fs, path::Path};

/// Chronological, whole-turn fragments produced before transcript rendering.
/// Large-session chunking consumes these units directly so it never has to
/// recover conversation boundaries by parsing formatted transcript text.
#[derive(Debug, Clone, Default)]
pub struct PreprocessedSession {
    pub units: Vec<String>,
    /// Tokens the model generated. Read here rather than by a second pass over
    /// the file because these parsers already visit every line.
    pub tokens: TokenCount,
}

impl PreprocessedSession {
    pub fn render(&self) -> String {
        self.units.join("\n")
    }
}

#[derive(Debug)]
pub enum PreprocessError {
    Io(std::io::Error),
    UnsupportedTool,
}

impl fmt::Display for PreprocessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::UnsupportedTool => write!(f, "tool not yet supported for preprocessing"),
        }
    }
}

impl From<std::io::Error> for PreprocessError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

pub fn preprocess_session(path: &Path, tool: &ToolSource) -> Result<String, PreprocessError> {
    Ok(preprocess_session_units(path, tool)?.render())
}

pub fn preprocess_session_units(
    path: &Path,
    tool: &ToolSource,
) -> Result<PreprocessedSession, PreprocessError> {
    let content = fs::read_to_string(path)?;
    match tool {
        ToolSource::ClaudeCode => Ok(PreprocessedSession {
            units: claude::preprocess_units(&content),
            tokens: claude::token_count(&content),
        }),
        ToolSource::Codex => Ok(PreprocessedSession {
            units: codex::preprocess_units(&content),
            tokens: codex::token_count(&content),
        }),
        ToolSource::Forge => Ok(PreprocessedSession {
            units: forge::preprocess_units(&content),
            tokens: forge::token_count(&content),
        }),
    }
}
