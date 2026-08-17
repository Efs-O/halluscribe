// HalluScribe - GGUF header reader tests.
// Headers are synthesised byte-for-byte rather than committed as fixtures: the
// real files are multi-gigabyte, and the point under test is the parser, not
// any one model. The byte layouts here mirror the real files read on this
// machine (Qwen3.8-27B -> `qwen35`, gemma-4-12b -> `gemma4`).

use super::*;
use std::fs;
use std::path::{Path, PathBuf};

const TYPE_U32: u32 = 4;
const TYPE_ARRAY_TAG: u32 = 9;

/// Builder for a GGUF header holding `keys` in order.
fn header(version: u32, keys: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = b"GGUF".to_vec();
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes()); // tensor count
    bytes.extend_from_slice(&(keys.len() as u64).to_le_bytes());
    for (key, encoded_value) in keys {
        bytes.extend_from_slice(&(key.len() as u64).to_le_bytes());
        bytes.extend_from_slice(key.as_bytes());
        bytes.extend_from_slice(encoded_value);
    }
    bytes
}

fn string_value(value: &str) -> Vec<u8> {
    let mut bytes = TYPE_STRING.to_le_bytes().to_vec();
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
    bytes
}

fn u32_value(value: u32) -> Vec<u8> {
    let mut bytes = TYPE_U32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes
}

/// A string array, the shape a tokenizer vocabulary uses - the case that must
/// be skipped element by element rather than in one jump.
fn string_array_value(values: &[&str]) -> Vec<u8> {
    let mut bytes = TYPE_ARRAY_TAG.to_le_bytes().to_vec();
    bytes.extend_from_slice(&TYPE_STRING.to_le_bytes());
    bytes.extend_from_slice(&(values.len() as u64).to_le_bytes());
    for value in values {
        bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    bytes
}

fn write_temp(tag: &str, bytes: &[u8]) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("halluscribe-gguf-{tag}-{unique}.gguf"));
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn reads_gemma_architecture_without_builtin_mtp() {
    let path = write_temp(
        "gemma",
        &header(
            3,
            &[
                ("general.architecture", string_value("gemma4")),
                ("general.name", string_value("Gemma-4-12B-It")),
                ("gemma4.block_count", u32_value(48)),
            ],
        ),
    );
    let identity = read_identity(&path).unwrap();
    assert_eq!(identity.architecture, "gemma4");
    assert!(
        !identity.has_builtin_mtp,
        "Gemma 4 carries its MTP head as a sidecar drafter, not in the model GGUF"
    );
    let _ = fs::remove_file(path);
}

#[test]
fn reads_qwen_architecture_and_detects_builtin_mtp() {
    let path = write_temp(
        "qwen",
        &header(
            3,
            &[
                ("general.architecture", string_value("qwen35")),
                ("general.name", string_value("Qwen3.8-27B")),
                ("qwen35.nextn_predict_layers", u32_value(1)),
            ],
        ),
    );
    let identity = read_identity(&path).unwrap();
    assert_eq!(identity.architecture, "qwen35");
    assert!(
        identity.has_builtin_mtp,
        "nextn_predict_layers means the MTP head ships inside this GGUF"
    );
    let _ = fs::remove_file(path);
}

#[test]
fn skips_over_string_arrays_to_reach_later_keys() {
    // A tokenizer vocabulary sits between the two keys of interest in real
    // files; mis-skipping it desynchronises every following read.
    let path = write_temp(
        "vocab",
        &header(
            3,
            &[
                ("general.architecture", string_value("qwen35")),
                (
                    "tokenizer.ggml.tokens",
                    string_array_value(&["a", "bb", ""]),
                ),
                ("qwen35.nextn_predict_layers", u32_value(1)),
            ],
        ),
    );
    let identity = read_identity(&path).unwrap();
    assert_eq!(identity.architecture, "qwen35");
    assert!(identity.has_builtin_mtp);
    let _ = fs::remove_file(path);
}

#[test]
fn rejects_a_file_that_is_not_gguf() {
    let path = write_temp("notgguf", b"This is a text file, not a model.");
    let error = read_identity(&path).unwrap_err();
    assert!(error.contains("not a GGUF file"), "got: {error}");
    let _ = fs::remove_file(path);
}

#[test]
fn rejects_a_truncated_header_rather_than_guessing() {
    // Declares one key but ends before the value - a half-finished download.
    let mut bytes = b"GGUF".to_vec();
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&1u64.to_le_bytes());
    bytes.extend_from_slice(&20u64.to_le_bytes());
    bytes.extend_from_slice(b"general.arch");
    let path = write_temp("truncated", &bytes);
    assert!(read_identity(&path).is_err());
    let _ = fs::remove_file(path);
}

#[test]
fn rejects_a_header_with_no_architecture_key() {
    let path = write_temp(
        "noarch",
        &header(3, &[("general.name", string_value("Mystery Model"))]),
    );
    let error = read_identity(&path).unwrap_err();
    assert!(
        error.contains("does not declare general.architecture"),
        "got: {error}"
    );
    let _ = fs::remove_file(path);
}

/// Read a real GGUF off this machine, when one is pointed at.
///
/// Synthesised headers prove the parser handles the layout it was written for;
/// only a real file proves that layout is the one shipping models actually use.
/// The path comes from `HALLUSCRIBE_TEST_GGUF` (semicolon-separated for several)
/// rather than a constant, because a hardcoded model path would be both an OS
/// assumption and unrunnable in CI. Silently skipped when unset.
#[test]
fn reads_a_real_gguf_when_one_is_pointed_at() {
    let Ok(raw) = std::env::var("HALLUSCRIBE_TEST_GGUF") else {
        return;
    };
    for entry in raw.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let identity = read_identity(Path::new(entry))
            .unwrap_or_else(|e| panic!("failed to read {entry}: {e}"));
        assert!(
            !identity.architecture.trim().is_empty(),
            "{entry} yielded an empty architecture"
        );
        println!(
            "{entry} -> architecture={} builtin_mtp={}",
            identity.architecture, identity.has_builtin_mtp
        );
    }
}
