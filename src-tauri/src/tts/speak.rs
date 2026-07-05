// HalluScribe - Piper synthesis: spawn piper on the selected voice, capture
// raw PCM from stdout, and prepend a hand-rolled WAV header (44-byte
// RIFF/WAVE, mono 16-bit PCM - port of Gemma4kids' `buildWavHeader`). Piper
// is short-lived and `wait()`-ed on synchronously, so it is NOT registered in
// the OPS-1 `llama_pids` orphan reaper - there is nothing to orphan.

use super::discover;
use crate::llama_runtime::apply_no_window;
use crate::settings::HalluScribeSettings;
use std::io::Write;
use std::process::{Command, Stdio};

/// Build the 44-byte WAV header for headerless mono 16-bit PCM at
/// `sample_rate`, `pcm_len` bytes long. Byte-for-byte port of Gemma4kids'
/// `buildWavHeader`.
pub fn wav_header(pcm_len: usize, sample_rate: u32) -> [u8; 44] {
    let mut header = [0u8; 44];
    let pcm_len_u32 = pcm_len as u32;
    let byte_rate = sample_rate.wrapping_mul(2);

    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36u32 + pcm_len_u32).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    header[20..22].copy_from_slice(&1u16.to_le_bytes()); // audio format: PCM
    header[22..24].copy_from_slice(&1u16.to_le_bytes()); // channels: mono
    header[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&2u16.to_le_bytes()); // block align
    header[34..36].copy_from_slice(&16u16.to_le_bytes()); // bits per sample
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&pcm_len_u32.to_le_bytes());
    header
}

/// Synthesize `text` with the user-selected voice and return a complete WAV
/// file (header + PCM) ready for the webview's `AudioContext.decodeAudioData`.
pub fn synth_wav(
    app: &tauri::AppHandle,
    settings: &HalluScribeSettings,
    text: &str,
) -> Result<Vec<u8>, String> {
    let piper_bin = discover::find_piper_bin(app, &settings.tts_piper_bin)
        .ok_or_else(|| "Piper not installed. Add it under ~/.halluscribe/tts/piper.".to_string())?;

    if settings.tts_voice.is_empty() {
        return Err("No voice selected. Pick a voice in Settings.".to_string());
    }

    let voices_dir = discover::tts_root(app)?.join("voices");
    let voice_path = voices_dir.join(format!("{}.onnx", settings.tts_voice));
    if !voice_path.is_file() {
        return Err(format!(
            "Voice '{}' not found under ~/.halluscribe/tts/voices.",
            settings.tts_voice
        ));
    }

    let sample_rate = discover::scan_voices(app)
        .into_iter()
        .find(|voice| voice.name == settings.tts_voice)
        .map(|voice| voice.sample_rate)
        .unwrap_or(22_050);

    let mut command = Command::new(&piper_bin);
    command
        .arg("--model")
        .arg(&voice_path)
        .arg("--output-raw")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_no_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn piper: {error}"))?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "failed to open piper stdin".to_string())?;
        stdin
            .write_all(text.as_bytes())
            .map_err(|error| format!("failed to write text to piper: {error}"))?;
        // `stdin` is dropped here, closing the pipe so piper starts synthesis.
    }

    let output = child
        .wait_with_output()
        .map_err(|error| format!("failed to read piper output: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Piper exited with status {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    let pcm = output.stdout;
    let mut wav = wav_header(pcm.len(), sample_rate).to_vec();
    wav.extend_from_slice(&pcm);
    Ok(wav)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_has_expected_tags_and_sizes() {
        let header = wav_header(1000, 22_050);
        assert_eq!(&header[0..4], b"RIFF");
        assert_eq!(&header[8..12], b"WAVE");
        assert_eq!(&header[12..16], b"fmt ");
        assert_eq!(&header[36..40], b"data");

        assert_eq!(u32::from_le_bytes(header[4..8].try_into().unwrap()), 1036);
        assert_eq!(u32::from_le_bytes(header[16..20].try_into().unwrap()), 16);
        assert_eq!(u16::from_le_bytes(header[20..22].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(header[22..24].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_le_bytes(header[24..28].try_into().unwrap()),
            22_050
        );
        assert_eq!(
            u32::from_le_bytes(header[28..32].try_into().unwrap()),
            44_100
        );
        assert_eq!(u16::from_le_bytes(header[32..34].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(header[34..36].try_into().unwrap()), 16);
        assert_eq!(u32::from_le_bytes(header[40..44].try_into().unwrap()), 1000);
    }

    #[test]
    fn wav_header_scales_riff_and_byte_rate_with_sample_rate() {
        let header = wav_header(0, 48_000);
        assert_eq!(u32::from_le_bytes(header[4..8].try_into().unwrap()), 36);
        assert_eq!(
            u32::from_le_bytes(header[24..28].try_into().unwrap()),
            48_000
        );
        assert_eq!(
            u32::from_le_bytes(header[28..32].try_into().unwrap()),
            96_000
        );
    }
}
