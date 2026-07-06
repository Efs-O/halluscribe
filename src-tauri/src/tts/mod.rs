// HalluScribe - offline text-to-speech (Piper) module root.
// Piper is a spawned binary (same subprocess pattern as llama-server, see
// gemma/llamacpp.rs) - no linkage, no `unsafe`. Voice discovery lives in
// `discover.rs`, synthesis in `speak.rs`. Piper install + voices are
// host-global under `~/.halluscribe/tts/`, shared across workspaces (see
// docs/internal/TTS_SPEAK_PLAN.md).

mod discover;
mod speak;

pub use discover::{find_piper_bin, scan_voices, VoiceInfo};
pub use speak::synth_wav;
