// HalluScribe - remember the window size the user resized to.
// Restored at startup and re-saved as the window is dragged, so the app opens
// where it was left instead of snapping back to tauri.conf.json's dimensions.

use crate::settings;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{LogicalSize, Manager, PhysicalSize};

/// Minimum gap between writes while a drag is in flight. A resize emits an
/// event per frame; without this, one drag would rewrite settings.json dozens
/// of times a second.
const SAVE_THROTTLE: Duration = Duration::from_millis(750);

/// Sizes below tauri.conf.json's `minWidth`/`minHeight` are never persisted -
/// restoring one would open a window the user cannot use.
const MIN_WIDTH: u32 = 860;
const MIN_HEIGHT: u32 = 620;

fn last_save() -> &'static Mutex<Option<Instant>> {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    &LAST
}

/// Apply the saved size to the main window at startup. No saved size leaves the
/// window exactly as tauri.conf.json declared it.
pub fn restore(app: &tauri::AppHandle, saved: &settings::HalluScribeSettings) {
    let (Some(width), Some(height)) = (saved.window_width, saved.window_height) else {
        return;
    };
    if !is_usable(width, height) {
        return;
    }
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window.set_size(PhysicalSize::new(width, height)) {
            eprintln!("[window] failed to restore saved size: {error}");
        }
    }
}

/// Save the window's size whenever the user finishes nudging it. Tauri emits a
/// `Resized` event per frame of a drag, so writes are throttled; the `Moved`
/// and close events force a final write, catching the last size of a drag that
/// the throttle would otherwise swallow.
pub fn watch(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let handle = app.clone();
    let watched = window.clone();
    window.on_window_event(move |event| {
        let force = matches!(event, tauri::WindowEvent::CloseRequested { .. });
        if !force && !matches!(event, tauri::WindowEvent::Resized(_)) {
            return;
        }
        let Some(size) = current_size(&watched) else {
            return;
        };
        if let Ok(dir) = crate::app_support::archive_dir(&handle) {
            remember(&dir, size, force);
        }
    });
}

/// Persist `size` as the remembered window size, at most once per
/// [`SAVE_THROTTLE`]. `force` bypasses the throttle - used on exit so the final
/// size of a drag is never the one that got throttled away.
pub fn remember(archive_dir: &Path, size: PhysicalSize<u32>, force: bool) {
    if !is_usable(size.width, size.height) {
        return;
    }
    if !force && !throttle_allows() {
        return;
    }
    // Window geometry describes the machine's screen, not the archive, so it is
    // host-owned: the write goes to the root that owns it. Targeting the active
    // workspace instead would land in a guest file, where the host-owned fields
    // are stripped on save and the size would never stick.
    let target = settings::host_settings_dir(archive_dir);
    let mut current = match settings::load_settings(&target) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("[window] could not load settings to save window size: {error}");
            return;
        }
    };
    if current.window_width == Some(size.width) && current.window_height == Some(size.height) {
        return;
    }
    current.window_width = Some(size.width);
    current.window_height = Some(size.height);
    if let Err(error) = settings::save_settings(&target, &current) {
        eprintln!("[window] failed to save window size: {error}");
    }
}

/// True when enough time has passed since the last write. Records the new
/// instant as a side effect, so callers need not.
fn throttle_allows() -> bool {
    let Ok(mut guard) = last_save().lock() else {
        return false;
    };
    let now = Instant::now();
    let allowed = guard.is_none_or(|previous| now.duration_since(previous) >= SAVE_THROTTLE);
    if allowed {
        *guard = Some(now);
    }
    allowed
}

/// Reject degenerate sizes: a minimised window reports 0x0 on Windows, and
/// anything under the configured minimum would restore unusable.
fn is_usable(width: u32, height: u32) -> bool {
    width >= MIN_WIDTH && height >= MIN_HEIGHT
}

/// The window's current size, or `None` when it is minimised or maximised.
/// Maximised is excluded deliberately: persisting the full-screen dimensions
/// would restore an un-maximised window filling the whole display.
pub fn current_size(window: &tauri::WebviewWindow) -> Option<PhysicalSize<u32>> {
    if window.is_minimized().unwrap_or(false) || window.is_maximized().unwrap_or(false) {
        return None;
    }
    window.inner_size().ok()
}

/// Convert a logical size to physical using the window's scale factor, for
/// callers holding logical dimensions.
#[allow(dead_code)]
pub fn to_physical(size: LogicalSize<u32>, scale_factor: f64) -> PhysicalSize<u32> {
    PhysicalSize::new(
        (f64::from(size.width) * scale_factor).round() as u32,
        (f64::from(size.height) * scale_factor).round() as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_sizes_below_the_configured_minimum() {
        assert!(!is_usable(0, 0));
        assert!(!is_usable(MIN_WIDTH - 1, MIN_HEIGHT));
        assert!(!is_usable(MIN_WIDTH, MIN_HEIGHT - 1));
    }

    #[test]
    fn accepts_the_minimum_and_above() {
        assert!(is_usable(MIN_WIDTH, MIN_HEIGHT));
        assert!(is_usable(1920, 1080));
    }

    #[test]
    fn throttle_blocks_a_second_immediate_write() {
        *last_save().lock().unwrap() = None;
        assert!(throttle_allows(), "first write should pass");
        assert!(
            !throttle_allows(),
            "immediate second write should be blocked"
        );
    }

    #[test]
    fn to_physical_applies_the_scale_factor() {
        let scaled = to_physical(LogicalSize::new(1100, 780), 1.5);
        assert_eq!(scaled.width, 1650);
        assert_eq!(scaled.height, 1170);
    }
}
