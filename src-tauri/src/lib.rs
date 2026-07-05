// HalluScribe - application library root.
// Wires Tauri commands and the nightly background sweep timer.

mod app_state;
mod app_support;
mod chat_prompt;
mod commands;
mod commands_profile;
mod commands_tts;
mod commands_workspace;
mod infer_lock;
mod llama_pids;
mod llama_runtime;
mod recorded_sessions;
mod tts;

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
pub mod workspace;

use app_state::{BriefingCancel, ChatCancel, SweepCancel};
use app_support::{archive_dir, clear_first_run, record_sweep_date, sweep_config};
use chrono::{Datelike, Local, Timelike};
use commands::{
    apply_redaction, cancel_briefing, cancel_chat, cancel_sweep, delete_sessions,
    get_raw_session_total, get_recent_sessions, get_settings, get_stats, preview_redaction,
    read_session, rebuild_session_embeddings, run_briefing, save_recorded_chat_session,
    save_settings, search_sessions, search_sessions_fulltext, search_sessions_semantic,
    send_chat_message, trigger_sweep, validate_ollama_api_key,
};
use commands_profile::{
    backfill_raw, count_available_raw, export_persona_pack, get_latest_digest, get_profile,
    get_profile_refresh_status, run_profile_refresh,
};
use commands_tts::{tts_list_voices, tts_speak, tts_status};
use commands_workspace::{
    create_workspace, delete_workspace, list_workspaces, rename_default_workspace,
    rename_workspace, set_workspace_import_only, switch_workspace,
};
use std::sync::{atomic::AtomicBool, Arc};
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
    let mut settings = settings::load_settings(&dir);
    settings.always_on_top = always_on_top;
    settings::save_settings(&dir, &settings).map_err(|error| error.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .manage(BriefingCancel(Arc::new(AtomicBool::new(false))))
        .manage(ChatCancel(Arc::new(AtomicBool::new(false))))
        .manage(SweepCancel(Arc::new(AtomicBool::new(false))))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let saved_settings = archive_dir(app.handle())
                .ok()
                .map(|dir| settings::load_settings(&dir))
                .unwrap_or_default();
            if let Err(error) = apply_always_on_top(app.handle(), saved_settings.always_on_top) {
                eprintln!("[window] failed to apply always-on-top at startup: {error}");
            }

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

            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut last_triggered_minute: Option<(i32, u32, u32, u32, u32)> = None;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    if let Some(config) = sweep_config(&handle, false) {
                        let now = Local::now();
                        let current_minute =
                            (now.year(), now.month(), now.day(), now.hour(), now.minute());
                        if last_triggered_minute == Some(current_minute) {
                            continue;
                        }
                        // Use the shared cancel flag so the Cancel button can stop
                        // a nightly auto-sweep, not just a manual "Run Now".
                        let cancel = handle.state::<SweepCancel>().0.clone();
                        cancel.store(false, std::sync::atomic::Ordering::Relaxed);
                        let result = scheduler::run_sweep(&handle, &config, cancel);
                        if result.ran {
                            last_triggered_minute = Some(current_minute);
                            clear_first_run(&handle);
                            record_sweep_date(&handle);
                            let mut message = format!(
                                "Sweep complete - processed: {}, skipped: {}, deferred: {}",
                                result.processed, result.skipped, result.deferred,
                            );
                            if !result.errors.is_empty() {
                                message.push_str(&format!(", errors: {}", result.errors.len()));
                            }
                            if result.flagged > 0 {
                                message.push_str(&format!(
                                    ", possible secrets flagged: {}",
                                    result.flagged
                                ));
                            }
                            let _ = handle.emit("sweep-done", message);
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_recent_sessions,
            get_stats,
            get_raw_session_total,
            trigger_sweep,
            get_settings,
            save_settings,
            validate_ollama_api_key,
            search_sessions,
            read_session,
            search_sessions_semantic,
            run_briefing,
            cancel_briefing,
            cancel_chat,
            save_recorded_chat_session,
            send_chat_message,
            search_sessions_fulltext,
            delete_sessions,
            cancel_sweep,
            rebuild_session_embeddings,
            preview_redaction,
            apply_redaction,
            run_profile_refresh,
            get_profile,
            get_latest_digest,
            get_profile_refresh_status,
            export_persona_pack,
            count_available_raw,
            backfill_raw,
            list_workspaces,
            create_workspace,
            switch_workspace,
            rename_workspace,
            rename_default_workspace,
            delete_workspace,
            set_workspace_import_only,
            tts_list_voices,
            tts_status,
            tts_speak,
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
