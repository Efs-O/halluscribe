// HalluScribe - application library root.
// Wires Tauri commands and the nightly background sweep timer.

mod app_state;
mod app_support;
mod atomic_file;
mod chat_prompt;
mod commands;
mod commands_capture;
mod commands_profile;
mod commands_tts;
mod commands_workspace;
mod infer_lock;
pub mod llama_gpu;
mod llama_mtp;
mod llama_pids;
mod llama_runtime;
pub mod llama_tuning;
mod recorded_sessions;
mod scheduled_sweep;
mod tts;
mod window_size;

pub mod apple_backup;
pub mod archive;
pub mod briefing;
pub mod core;
pub mod gemma;
pub mod mcp;
pub mod pack;
pub mod preprocessor;
pub mod profile;
pub mod readers;
pub mod retrieval;
pub mod scanner;
pub mod scheduler;
pub mod search;
pub mod settings;
pub mod tokens;
pub mod workspace;

use app_state::{
    BriefingCancel, CancelState, CaptureCancel, CaptureStatusState, ChatCancel, ProfileCancel,
    SweepCancel,
};
use app_support::{archive_dir, default_archive_dir};
use commands::{
    apply_redaction, cancel_briefing, cancel_chat, cancel_sweep, chat_import_status,
    delete_sessions, get_raw_session_total, get_recent_sessions, get_settings, get_stats,
    preview_redaction, read_image_attachment, read_session, rebuild_session_embeddings,
    run_briefing, save_recorded_chat_session, save_settings, search_raw_transcripts,
    search_sessions, search_sessions_fulltext, search_sessions_semantic, send_chat_message,
    trigger_business_import, trigger_sweep, validate_ollama_api_key,
};
use commands_capture::{cancel_capture, get_capture_status};
use commands_profile::{
    backfill_raw, business_profile_status, cancel_profile_refresh, count_available_raw,
    export_persona_pack, get_latest_digest, get_profile, get_profile_refresh_status,
    run_profile_refresh,
};
use commands_tts::{tts_list_voices, tts_speak, tts_status};
use commands_workspace::{
    create_workspace, delete_workspace, list_workspaces, move_workspace, rename_default_workspace,
    rename_workspace, set_workspace_import_only, suggest_workspace_path, switch_workspace,
};
use std::sync::Arc;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Emitter;
use tauri::Manager;

/// Draw a dark-blue square frame icon at runtime and apply it to both the tray
/// and the taskbar window button. Same pixel-math as HalluMeter's set_tray_color
/// so the two apps look like siblings.
fn apply_ring_icon(app: &tauri::AppHandle) {
    const SZ: u32 = 32;
    const R: u8 = 29;
    const G: u8 = 78;
    const B: u8 = 216;
    const BORDER: u32 = 5;

    let mut rgba: Vec<u8> = Vec::with_capacity((SZ * SZ * 4) as usize);
    for y in 0..SZ {
        for x in 0..SZ {
            let on_frame = x < BORDER || y < BORDER || x >= SZ - BORDER || y >= SZ - BORDER;
            if on_frame {
                rgba.extend_from_slice(&[R, G, B, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }

    let icon = Image::new_owned(rgba, SZ, SZ);
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_icon(Some(icon.clone()));
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_icon(icon);
    }
}

fn apply_always_on_top(app: &tauri::AppHandle, always_on_top: bool) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("main") {
        window
            .set_always_on_top(always_on_top)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn persist_always_on_top(app: &tauri::AppHandle, always_on_top: bool) -> Result<(), String> {
    let dir = archive_dir(app)?;
    let mut settings = settings::load_settings(&dir).map_err(|error| error.to_string())?;
    settings.always_on_top = always_on_top;
    settings::save_settings(&dir, &settings).map_err(|error| error.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .manage(BriefingCancel(CancelState::new()))
        .manage(ChatCancel(CancelState::new()))
        .manage(SweepCancel(CancelState::new()))
        .manage(ProfileCancel(CancelState::new()))
        .manage(CaptureCancel(CancelState::new()))
        .manage(CaptureStatusState(Arc::new(std::sync::Mutex::new(
            archive::CaptureStatus::default(),
        ))))
        // MUST stay first in the plugin chain (Tauri requirement): a second
        // launch is intercepted here and hands its argv to the running
        // instance instead of booting a rival one. The window may be hidden
        // in the tray rather than merely unfocused, so show() before
        // set_focus(), exactly as the tray's "show" item does.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Release builds send llama-server's stderr here, so an overnight
            // sweep failure leaves an explanation behind. Resolved via the
            // Tauri path API rather than assumed.
            if let Ok(dir) = archive_dir(app.handle()) {
                llama_runtime::set_log_dir(dir.join("logs"));
            }
            // llama-tuning.yaml describes this machine's cards and cores, so it
            // lives at the DEFAULT root and is shared by every workspace.
            // The machine's inference config lives at the DEFAULT root too, and
            // guest workspaces read it through from there rather than holding a
            // copy that drifts. Both roots are resolved from one lookup.
            match default_archive_dir(app.handle()) {
                Ok(dir) => {
                    settings::set_host_root(dir.clone());
                    llama_tuning::set_host_root(dir);
                }
                Err(error) => eprintln!("[llama] tuning root unavailable: {error}"),
            }
            let saved_settings = match archive_dir(app.handle())
                .map_err(|error| format!("archive path: {error}"))
                .and_then(|dir| settings::load_settings(&dir).map_err(|error| error.to_string()))
            {
                Ok(settings) => settings,
                Err(error) => {
                    eprintln!("[settings] failed to load at startup: {error}");
                    settings::HalluScribeSettings::default()
                }
            };
            if let Err(error) = apply_always_on_top(app.handle(), saved_settings.always_on_top) {
                eprintln!("[window] failed to apply always-on-top at startup: {error}");
            }
            window_size::restore(app.handle(), &saved_settings);
            window_size::watch(app.handle());

            let show_item = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let hide_item = MenuItem::with_id(app, "hide", "Hide", true, None::<&str>)?;
            let always_on_top_item = CheckMenuItem::with_id(
                app,
                "always_on_top",
                "Always on top",
                true,
                saved_settings.always_on_top,
                None::<&str>,
            )?;
            let always_on_top_toggle = always_on_top_item.clone();
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[&show_item, &hide_item, &always_on_top_item, &quit_item],
            )?;

            let tray = TrayIconBuilder::with_id("main")
                .menu(&menu)
                .tooltip("Halluscribe")
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                            apply_ring_icon(app);
                        }
                    }
                })
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "hide" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                    "always_on_top" => {
                        let new_value = always_on_top_toggle
                            .is_checked()
                            .unwrap_or(saved_settings.always_on_top);
                        let _ = always_on_top_toggle.set_checked(new_value);
                        if let Err(error) = apply_always_on_top(app, new_value) {
                            eprintln!("[window] failed to toggle always-on-top: {error}");
                        }
                        let _ = persist_always_on_top(app, new_value);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });

            tray.build(app)?;
            apply_ring_icon(app.handle());

            scheduled_sweep::start(app.handle().clone());

            // Startup raw capture (coding sources only, no settings gate): a
            // background pass so a coding tool that prunes its own JSONL logs
            // before its first sweep never loses that session's raw detail.
            // Never blocks app startup - spawned and forgotten; progress is
            // observable via managed state + the "raw-capture-progress" event.
            let capture_handle = app.handle().clone();
            std::thread::spawn(move || {
                let Ok(dir) = archive_dir(&capture_handle) else {
                    return;
                };
                let import_only =
                    match default_archive_dir(&capture_handle).and_then(|default_root| {
                        crate::workspace::is_active_import_only(&default_root, &dir)
                    }) {
                        Ok(import_only) => import_only,
                        Err(error) => {
                            eprintln!("[capture] could not load workspace registry: {error}");
                            return;
                        }
                    };
                let loaded_settings = match settings::load_settings(&dir) {
                    Ok(settings) => settings,
                    Err(error) => {
                        eprintln!("[capture] could not load settings: {error}");
                        return;
                    }
                };
                let Some(cancel) = capture_handle.state::<CaptureCancel>().0.try_begin_run() else {
                    return;
                };
                let status_state = capture_handle.state::<CaptureStatusState>().0.clone();
                let progress_handle = capture_handle.clone();
                let publish = move |status: &archive::CaptureStatus| {
                    *status_state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = status.clone();
                    let _ = progress_handle.emit("raw-capture-progress", status.clone());
                };
                // `run_capture` calls this once per file plus once more with the
                // final Done/Cancelled status, so managed state and the event
                // are always in sync - no separate "after it returns" emit needed.
                let pass = crate::app_state::catch_job_panic(|| {
                    archive::run_capture(&dir, &loaded_settings, import_only, &cancel, &publish)
                });
                if let Err(message) = pass {
                    eprintln!("[capture] the capture pass crashed: {message}");
                    publish(&archive::CaptureStatus::Failed {
                        done: 0,
                        total: 0,
                        captured: 0,
                        errors: vec![format!("the capture pass crashed: {message}")],
                    });
                }
                capture_handle
                    .state::<CaptureCancel>()
                    .0
                    .finish_run(&cancel);
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_recent_sessions,
            get_stats,
            get_raw_session_total,
            trigger_business_import,
            trigger_sweep,
            get_settings,
            save_settings,
            chat_import_status,
            validate_ollama_api_key,
            search_sessions,
            read_session,
            search_sessions_semantic,
            run_briefing,
            cancel_briefing,
            cancel_chat,
            save_recorded_chat_session,
            send_chat_message,
            search_raw_transcripts,
            search_sessions_fulltext,
            delete_sessions,
            cancel_sweep,
            rebuild_session_embeddings,
            preview_redaction,
            apply_redaction,
            run_profile_refresh,
            cancel_profile_refresh,
            get_profile,
            business_profile_status,
            get_latest_digest,
            get_profile_refresh_status,
            export_persona_pack,
            count_available_raw,
            backfill_raw,
            list_workspaces,
            suggest_workspace_path,
            create_workspace,
            move_workspace,
            switch_workspace,
            rename_workspace,
            rename_default_workspace,
            delete_workspace,
            set_workspace_import_only,
            tts_list_voices,
            tts_status,
            tts_speak,
            get_capture_status,
            cancel_capture,
            read_image_attachment,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                briefing::kill_server();
                retrieval::kill_embedding_server();
            }
        });
}
