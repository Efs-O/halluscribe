// HalluScribe - read a user-picked image into a chat attachment.
// The picker is the Tauri dialog plugin, which returns a path rather than a
// browser File, so the bytes are read here. Validating in Rust also fixes a
// weakness of the old `<input type="file">` path, which trusted the browser's
// `file.type`: Windows reports an empty MIME for files with no registered
// association, and a perfectly valid JPEG was rejected as "unsupported".

use serde::Serialize;
use std::path::Path;

/// Matches the frontend's previous limit. Large enough for a screenshot, small
/// enough that the base64 payload does not bloat the chat request.
const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

/// Camel-cased to match the frontend's existing `ChatAttachment` shape, so the
/// picker result drops straight into the chat state with no translation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImageAttachment {
    name: String,
    mime_type: String,
    base64: String,
}

/// Read the image at `path`, rejecting anything the vision models cannot take.
/// The MIME type comes from the file extension, not from the OS.
#[tauri::command]
pub(crate) fn read_image_attachment(path: String) -> Result<ImageAttachment, String> {
    let path = Path::new(&path);
    let mime_type = mime_for(path)?;
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("cannot read image: {error}"))?;
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "Image is {:.1} MB. Keep attachments under 5 MB.",
            metadata.len() as f64 / (1024.0 * 1024.0)
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| format!("cannot read image: {error}"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image")
        .to_string();
    Ok(ImageAttachment {
        name,
        mime_type,
        base64: encode_base64(&bytes),
    })
}

/// Map an extension to the MIME type the OpenAI-compatible chat endpoint wants.
fn mime_for(path: &Path) -> Result<String, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match extension.as_str() {
        "png" => Ok("image/png".to_string()),
        "jpg" | "jpeg" | "jfif" => Ok("image/jpeg".to_string()),
        "webp" => Ok("image/webp".to_string()),
        "gif" => Ok("image/gif".to_string()),
        "" => Err("That file has no extension, so its image type is unknown.".to_string()),
        other => Err(format!(
            "Unsupported image type .{other}. Use PNG, JPG, WEBP, or GIF."
        )),
    }
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding. Hand-rolled rather than pulling in a crate for
/// forty lines - adding a dependency is a decision for the repo owner.
fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map_or(0, u32::from);
        let b2 = chunk.get(2).copied().map_or(0, u32::from);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64_ALPHABET[(triple >> 18) as usize & 63] as char);
        out.push(BASE64_ALPHABET[(triple >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            BASE64_ALPHABET[(triple >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64_ALPHABET[triple as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        // RFC 4648 test vectors - these cover all three padding cases.
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foob"), "Zm9vYg==");
        assert_eq!(encode_base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_handles_high_bytes() {
        // PNG magic number - exercises bytes above 0x7f.
        assert_eq!(encode_base64(&[0x89, 0x50, 0x4E, 0x47]), "iVBORw==");
    }

    #[test]
    fn mime_is_derived_from_the_extension_case_insensitively() {
        assert_eq!(mime_for(Path::new("a.PNG")).unwrap(), "image/png");
        assert_eq!(mime_for(Path::new("a.jpeg")).unwrap(), "image/jpeg");
        assert_eq!(mime_for(Path::new("a.webp")).unwrap(), "image/webp");
        assert_eq!(mime_for(Path::new("a.gif")).unwrap(), "image/gif");
    }

    #[test]
    fn jfif_is_accepted_as_jpeg() {
        // The extension Windows hands out with no registered MIME type, which
        // the old browser-side check rejected outright.
        assert_eq!(mime_for(Path::new("photo.jfif")).unwrap(), "image/jpeg");
    }

    #[test]
    fn unsupported_and_missing_extensions_are_rejected() {
        assert!(mime_for(Path::new("a.bmp")).is_err());
        assert!(mime_for(Path::new("a.heic")).is_err());
        assert!(mime_for(Path::new("noextension")).is_err());
    }

    #[test]
    fn oversized_images_are_rejected_before_reading() {
        let dir = std::env::temp_dir().join("halluscribe-image-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.png");
        std::fs::write(&path, vec![0u8; (MAX_IMAGE_BYTES + 1) as usize]).unwrap();
        let error = read_image_attachment(path.to_string_lossy().to_string()).unwrap_err();
        assert!(error.contains("under 5 MB"), "{error}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_small_png_round_trips() {
        let dir = std::env::temp_dir().join("halluscribe-image-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("small.png");
        std::fs::write(&path, b"foobar").unwrap();
        let attachment = read_image_attachment(path.to_string_lossy().to_string()).unwrap();
        assert_eq!(attachment.mime_type, "image/png");
        assert_eq!(attachment.name, "small.png");
        assert_eq!(attachment.base64, "Zm9vYmFy");
        let _ = std::fs::remove_file(&path);
    }
}
