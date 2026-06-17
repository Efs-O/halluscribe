// HalluScribe - JSONL preprocessor.
// Strips noise and extracts signal from raw session files before Gemma.

mod claude;
mod codex;
mod cont;
mod forge;
mod shared;

use crate::scanner::ToolSource;
use std::{fmt, fs, path::Path};

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
    match tool {
        ToolSource::Continue => Ok(cont::preprocess(path)),
        _ => {
            let content = fs::read_to_string(path)?;
            match tool {
                ToolSource::ClaudeCode => Ok(claude::preprocess(&content)),
                ToolSource::Codex => Ok(codex::preprocess(&content)),
                ToolSource::Forge => Ok(forge::preprocess(&content)),
                ToolSource::Continue => unreachable!(),
            }
        }
    }
}
