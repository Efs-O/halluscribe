// HalluScribe - stable facade for domain-specific Tauri command handlers.

mod archive;
mod briefing;
mod chat;
mod image;
mod search;
mod settings;
mod sweep;

const BUSY_MESSAGE: &str =
    "Another job (a sweep, briefing, or chat) is using the model. Try again once it finishes.";

pub(crate) use archive::{
    apply_redaction, delete_sessions, get_raw_session_total, get_recent_sessions, get_stats,
    preview_redaction, read_session,
};
pub(crate) use briefing::{cancel_briefing, run_briefing};
pub(crate) use chat::{cancel_chat, save_recorded_chat_session, send_chat_message};
pub(crate) use image::read_image_attachment;
pub(crate) use search::{
    rebuild_session_embeddings, search_raw_transcripts, search_sessions, search_sessions_fulltext,
    search_sessions_semantic,
};
pub(crate) use settings::{
    chat_import_status, get_settings, save_settings, validate_ollama_api_key,
};
pub(crate) use sweep::{cancel_sweep, trigger_sweep};
