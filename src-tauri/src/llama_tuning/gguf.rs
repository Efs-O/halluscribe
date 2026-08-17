// HalluScribe - minimal GGUF header reader.
// Answers one question: what kind of model is this file? The answer keys the
// per-architecture llama-server tuning, so it must come from the file itself
// rather than its name - a renamed or re-quantised GGUF is still the same
// architecture, and llama.cpp dispatches on this same field.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// The identity of a model as its own header declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelIdentity {
    /// `general.architecture`, e.g. `gemma4` or `qwen35`.
    pub architecture: String,
    /// True when the header declares `<arch>.nextn_predict_layers`, meaning the
    /// multi-token-prediction head ships inside this GGUF instead of as a
    /// sidecar drafter. Qwen 3.8 is built this way; Gemma 4 is not.
    pub has_builtin_mtp: bool,
}

/// Stop scanning after this many key/value pairs. `general.architecture` is
/// conventionally the first key; a model that has not declared it within the
/// first few dozen is not one we can tune, and scanning a whole tokenizer
/// vocabulary to find that out would read hundreds of megabytes.
const MAX_KEYS_SCANNED: u64 = 64;

/// Refuse absurd lengths rather than trying to allocate them. A corrupt or
/// truncated header otherwise asks for gigabytes on the first bad read.
const MAX_STRING_LEN: u64 = 64 * 1024;

/// Read the architecture a GGUF file declares.
///
/// `Err` means the file is not a readable GGUF - a wrong path, a truncated
/// download, or a format newer than this reader. Callers surface that rather
/// than guessing an architecture, because guessing wrong picks the wrong KV
/// quantisation and speculative-decoding flags.
pub fn read_identity(model: &Path) -> Result<ModelIdentity, String> {
    let file = File::open(model)
        .map_err(|e| format!("failed to open model file {}: {e}", model.display()))?;
    let mut reader = BufReader::new(file);

    let mut magic = [0u8; 4];
    read_exact(&mut reader, &mut magic)?;
    if &magic != b"GGUF" {
        return Err(format!(
            "{} is not a GGUF file (bad magic bytes)",
            model.display()
        ));
    }
    let version = read_u32(&mut reader)?;
    if !(2..=3).contains(&version) {
        return Err(format!(
            "{} declares unsupported GGUF version {version}",
            model.display()
        ));
    }
    // Tensor count - present in the header but irrelevant here.
    let _ = read_u64(&mut reader)?;
    let kv_count = read_u64(&mut reader)?;

    let mut architecture: Option<String> = None;
    let mut has_builtin_mtp = false;
    for _ in 0..kv_count.min(MAX_KEYS_SCANNED) {
        let key = read_string(&mut reader)?;
        let value_type = read_u32(&mut reader)?;
        if key == "general.architecture" {
            if value_type != TYPE_STRING {
                return Err(format!(
                    "{} declares general.architecture as a non-string value",
                    model.display()
                ));
            }
            architecture = Some(read_string(&mut reader)?);
        } else {
            if key.ends_with(".nextn_predict_layers") {
                has_builtin_mtp = true;
            }
            skip_value(&mut reader, value_type)?;
        }
        // Everything needed is known once the architecture is in hand; the MTP
        // key sorts after it within the same `general`/`<arch>` block.
        if architecture.is_some() && has_builtin_mtp {
            break;
        }
    }

    let architecture = architecture.ok_or_else(|| {
        format!(
            "{} does not declare general.architecture in its first {MAX_KEYS_SCANNED} metadata keys",
            model.display()
        )
    })?;
    Ok(ModelIdentity {
        architecture,
        has_builtin_mtp,
    })
}

const TYPE_STRING: u32 = 8;
const TYPE_ARRAY: u32 = 9;

/// Byte width of every fixed-size GGUF value type. `None` for the two
/// variable-length types, which have their own readers.
fn scalar_width(value_type: u32) -> Option<u64> {
    match value_type {
        0 | 1 | 7 => Some(1), // u8, i8, bool
        2 | 3 => Some(2),     // u16, i16
        4..=6 => Some(4),     // u32, i32, f32
        10..=12 => Some(8),   // u64, i64, f64
        _ => None,
    }
}

/// Advance past a value without materialising it. Arrays recurse once per
/// element for strings, and are skipped in one jump when fixed-width.
fn skip_value<R: Read>(reader: &mut R, value_type: u32) -> Result<(), String> {
    if let Some(width) = scalar_width(value_type) {
        return skip_bytes(reader, width);
    }
    match value_type {
        TYPE_STRING => {
            let len = read_u64(reader)?;
            skip_bytes(reader, len)
        }
        TYPE_ARRAY => {
            let element_type = read_u32(reader)?;
            let count = read_u64(reader)?;
            if let Some(width) = scalar_width(element_type) {
                return skip_bytes(reader, count.saturating_mul(width));
            }
            for _ in 0..count {
                skip_value(reader, element_type)?;
            }
            Ok(())
        }
        other => Err(format!("unknown GGUF value type {other}")),
    }
}

fn skip_bytes<R: Read>(reader: &mut R, count: u64) -> Result<(), String> {
    let copied = std::io::copy(&mut reader.take(count), &mut std::io::sink())
        .map_err(|e| format!("failed reading GGUF header: {e}"))?;
    if copied != count {
        return Err("GGUF header ended mid-value".to_string());
    }
    Ok(())
}

fn read_exact<R: Read>(reader: &mut R, buffer: &mut [u8]) -> Result<(), String> {
    reader
        .read_exact(buffer)
        .map_err(|e| format!("failed reading GGUF header: {e}"))
}

fn read_u32<R: Read>(reader: &mut R) -> Result<u32, String> {
    let mut buffer = [0u8; 4];
    read_exact(reader, &mut buffer)?;
    Ok(u32::from_le_bytes(buffer))
}

fn read_u64<R: Read>(reader: &mut R) -> Result<u64, String> {
    let mut buffer = [0u8; 8];
    read_exact(reader, &mut buffer)?;
    Ok(u64::from_le_bytes(buffer))
}

fn read_string<R: Read>(reader: &mut R) -> Result<String, String> {
    let len = read_u64(reader)?;
    if len > MAX_STRING_LEN {
        return Err(format!(
            "GGUF header declares an implausible {len}-byte string"
        ));
    }
    let mut buffer = vec![0u8; len as usize];
    read_exact(reader, &mut buffer)?;
    String::from_utf8(buffer).map_err(|e| format!("GGUF header holds invalid UTF-8: {e}"))
}

#[cfg(test)]
mod tests;
