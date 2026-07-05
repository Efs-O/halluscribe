// HalluScribe - Tauri command handlers for offline text-to-speech (Piper):
// list installed voices, report setup status, and synthesize a reply.

use crate::app_support::archive_dir;
use crate::settings;
use crate::tts::{self, VoiceInfo};
use serde::Serialize;

/// Setup status for the Speak button + Settings Voice section: whether piper
/// resolves, how many voices are installed, and which one is selected.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TtsStatus {
    pub piper_installed: bool,
    pub voice_count: usize,
    pub selected_voice: String,
}

/// List installed Piper voices for the Settings voice picker.
#[tauri::command]
pub(crate) fn tts_list_voices(app: tauri::AppHandle) -> Vec<VoiceInfo> {
    tts::scan_voices(&app)
}

/// Report whether piper resolves and how many voices are installed, driving
/// the Speak button's enabled state and the Settings setup hints.
#[tauri::command]
pub(crate) fn tts_status(app: tauri::AppHandle) -> Result<TtsStatus, String> {
    let dir = archive_dir(&app)?;
    let loaded_settings = settings::load_settings(&dir);
    let piper_installed = tts::find_piper_bin(&app, &loaded_settings.tts_piper_bin).is_some();
    let voice_count = tts::scan_voices(&app).len();
    Ok(TtsStatus {
        piper_installed,
        voice_count,
        selected_voice: loaded_settings.tts_voice,
    })
}

/// Synthesize `text` with the user-selected voice and return a complete WAV
/// file as raw bytes, so the webview receives an `ArrayBuffer` instead of a
/// JSON number array.
#[tauri::command]
pub(crate) fn tts_speak(
    app: tauri::AppHandle,
    text: String,
) -> Result<tauri::ipc::Response, String> {
    let dir = archive_dir(&app)?;
    let loaded_settings = settings::load_settings(&dir);
    let wav = tts::synth_wav(&app, &loaded_settings, &text)?;
    Ok(tauri::ipc::Response::new(wav))
}
