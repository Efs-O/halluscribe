// HalluScribe - shape checks for the executable and model paths in settings.
// The binary field and the model fields both take a filesystem path, so pasting
// one into the other saves cleanly and only fails much later, deep inside a
// spawn: pointing the binary field at a GGUF surfaces as the bare OS message
// "%1 is not a valid Win32 application (os error 193)", which names neither the
// field nor the mistake. These checks reject the swap at save time instead.
//
// Deliberately shape-only, never existence: a path on a removable or network
// drive is legitimately absent at save time, and refusing to save it would be
// worse than the error it prevents.

use super::HalluScribeSettings;
use std::path::Path;

/// Extensions Windows will actually execute. Elsewhere the executable bit —
/// not the name — decides, so any extension is allowed.
#[cfg(target_os = "windows")]
const EXECUTABLE_EXTENSIONS: &[&str] = &["exe", "bat", "cmd", "com"];

/// Reject a settings payload whose binary field holds a model, or whose model
/// fields hold something that is not a GGUF. `Ok(())` when every populated path
/// has a plausible shape; empty fields mean "not configured" and always pass.
pub fn validate_paths(settings: &HalluScribeSettings) -> Result<(), String> {
    check_binary("llama.cpp server binary", &settings.llama_server_bin)?;
    check_model("Gemma model", &settings.gemma_model_path)?;
    check_model("embedding model", &settings.embedding_model_path)?;
    Ok(())
}

fn check_binary(label: &str, value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let extension = extension_of(trimmed);
    if extension.as_deref() == Some("gguf") {
        return Err(format!(
            "The {label} field is set to a GGUF model ({trimmed}). That field wants llama-server itself; the model belongs in the model path field."
        ));
    }
    #[cfg(target_os = "windows")]
    if let Some(extension) = extension {
        if !EXECUTABLE_EXTENSIONS.contains(&extension.as_str()) {
            return Err(format!(
                "The {label} field is set to a .{extension} file ({trimmed}), which Windows cannot execute. Point it at llama-server.exe."
            ));
        }
    }
    Ok(())
}

fn check_model(label: &str, value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    match extension_of(trimmed) {
        Some(extension) if extension == "gguf" => Ok(()),
        Some(extension) => Err(format!(
            "The {label} field is set to a .{extension} file ({trimmed}). It needs a .gguf model file."
        )),
        None => Err(format!(
            "The {label} field ({trimmed}) has no file extension. It needs a .gguf model file."
        )),
    }
}

fn extension_of(value: &str) -> Option<String> {
    Path::new(value)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(bin: &str, model: &str) -> HalluScribeSettings {
        HalluScribeSettings {
            llama_server_bin: bin.to_string(),
            gemma_model_path: model.to_string(),
            ..HalluScribeSettings::default()
        }
    }

    #[test]
    fn accepts_a_correctly_filled_pair() {
        let ok = settings(r"C:\llama\llama-server.exe", r"N:\models\gemma.gguf");
        assert!(validate_paths(&ok).is_ok());
    }

    #[test]
    fn accepts_empty_fields_as_unconfigured() {
        assert!(validate_paths(&settings("", "")).is_ok());
    }

    #[test]
    fn rejects_a_gguf_in_the_binary_field() {
        // The exact mistake that produced os error 193.
        let bad = settings(r"N:\models\gemma-4-12b.gguf", r"N:\models\gemma.gguf");
        let error = validate_paths(&bad).unwrap_err();
        assert!(error.contains("llama.cpp server binary"), "{error}");
        assert!(error.contains("GGUF"), "{error}");
    }

    #[test]
    fn rejects_a_non_gguf_in_the_model_field() {
        let bad = settings(r"C:\llama\llama-server.exe", r"C:\llama\llama-server.exe");
        let error = validate_paths(&bad).unwrap_err();
        assert!(error.contains("Gemma model"), "{error}");
    }

    #[test]
    fn rejects_an_extensionless_model_path() {
        let bad = settings(r"C:\llama\llama-server.exe", r"N:\models\gemma");
        assert!(validate_paths(&bad).is_err());
    }

    #[test]
    fn accepts_uppercase_gguf() {
        let ok = settings(r"C:\llama\llama-server.exe", r"N:\models\GEMMA.GGUF");
        assert!(validate_paths(&ok).is_ok());
    }

    #[test]
    fn validates_the_embedding_model_too() {
        let bad = HalluScribeSettings {
            embedding_model_path: r"C:\llama\llama-server.exe".to_string(),
            ..HalluScribeSettings::default()
        };
        let error = validate_paths(&bad).unwrap_err();
        assert!(error.contains("embedding model"), "{error}");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn rejects_a_non_executable_extension_in_the_binary_field() {
        let bad = settings(r"C:\llama\notes.txt", r"N:\models\gemma.gguf");
        assert!(validate_paths(&bad).is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn accepts_an_extensionless_binary_path() {
        // A bare `llama-server` resolves via PATH / the .exe fallback.
        let ok = settings("llama-server", r"N:\models\gemma.gguf");
        assert!(validate_paths(&ok).is_ok());
    }
}
