pub mod clipboard_history;
pub mod desktop_signals;
pub mod dictation;
pub mod extensions;
mod idle_webview;
pub mod installed_usage;
pub mod lifecycle;
pub mod lyrics;
mod lyrics_match;
pub mod media;
pub mod osd_flyout;
mod service_preferences;
pub mod system;
pub mod telemetry;
pub mod usage;
pub mod visualizer;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, State, WindowEvent,
};

#[cfg(windows)]
use windows::Win32::Foundation::HWND;

/// Frontend command to notify Rust of the island's current CSS dimensions.
#[tauri::command]
fn set_hit(hit_state: State<'_, Arc<system::HitState>>, w: u32, h: u32) {
    let (clamped_w, clamped_h) = system::clamp_hit_size(w, h);
    hit_state.width.store(clamped_w, Ordering::Relaxed);
    hit_state.height.store(clamped_h, Ordering::Relaxed);
}

pub fn run() {
    let dictation_service = Arc::new(dictation::DictationService::new());
    let clipboard_service = Arc::new(clipboard_history::ClipboardHistoryService::new());
    let dictation_setup = dictation_service.clone();
    let clipboard_setup = clipboard_service.clone();
    let hit_state = Arc::new(system::HitState::default());
    let hit_state_setup = hit_state.clone();

    tauri::Builder::default()
        .manage(dictation_service)
        .manage(clipboard_service)
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = system::open_settings(&handle) {
                    eprintln!("Single-instance settings focus: {error}");
                }
            });
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(dictation::handle_shortcut)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(hit_state)
        .invoke_handler(tauri::generate_handler![
            set_hit,
            idle_webview::island_activity,
            dictation::dictation_get_state,
            dictation::dictation_set_settings,
            dictation::dictation_model_download,
            dictation::dictation_model_download_cancel,
            dictation::dictation_model_remove,
            dictation::dictation_model_use,
            dictation::dictation_models_scan,
            dictation::dictation_shortcuts_pause,
            dictation::dictation_shortcuts_retry,
            dictation::dictation_toggle,
            dictation::dictation_hold_start,
            dictation::dictation_hold_stop,
            dictation::dictation_cancel,
            dictation::dictation_review_insert,
            dictation::dictation_review_discard,
            clipboard_history::clipboard_get_state,
            clipboard_history::clipboard_configure,
            clipboard_history::clipboard_set_enabled,
            clipboard_history::clipboard_set_persistence,
            clipboard_history::clipboard_delete,
            clipboard_history::clipboard_clear,
            clipboard_history::clipboard_pin,
            clipboard_history::clipboard_copy,
            lyrics::lyrics_status,
            lyrics::lyrics_set_enabled,
            lyrics::lyrics_get,
            lyrics::lyrics_clear_cache,
            telemetry::telemetry_now,
            telemetry::telemetry_history,
            visualizer::visualizer_status,
            visualizer::visualizer_set_enabled,
            visualizer::visualizer_set_reduced_motion,
            visualizer::visualizer_spectrum,
            media::media_sessions,
            media::media_select_session,
            installed_usage::installed_usage_enabled,
            installed_usage::set_installed_usage_enabled,
            installed_usage::refresh_installed_usage,
            extensions::extension_snapshot,
            extensions::extension_settings,
            extensions::call_set_active,
            extensions::calendar_import,
            extensions::calendar_join,
            extensions::shelf_add,
            extensions::shelf_pick,
            extensions::shelf_remove,
            extensions::shelf_reveal,
            extensions::shelf_drag,
            extensions::radio_set,
            extensions::notification_access,
            usage::usage_now,
            usage::set_secret,
            desktop_signals::volume_step,
            desktop_signals::volume_toggle_mute,
            osd_flyout::get_hide_system_flyout,
            osd_flyout::set_hide_system_flyout,
            osd_flyout::set_flyout_island_pause,
            system::quick_action,
            system::open_settings_cmd,
            system::open_island_cmd,
            system::deactivate_island,
            system::fullscreen_now,
            system::get_fullscreen_auto_hide,
            system::set_fullscreen_auto_hide,
            system::get_autostart,
            system::set_autostart,
            media::media_now,
            media::media_toggle,
            media::media_next,
            media::media_prev,
            media::media_seek,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            // Missing shortcut availability must not prevent the island from launching.
            if let Err(error) = dictation::start(handle.clone(), &dictation_setup) {
                eprintln!("Dictation startup: {error}");
            }
            if let Err(error) = clipboard_history::start(handle.clone(), &clipboard_setup) {
                eprintln!("Clipboard startup: {error}");
            }

            // Setup System Tray Menu (Open island / Settings... / Quit Tuck)
            let open_island_item =
                MenuItem::with_id(app, "open_island", "Open island", true, None::<&str>)?;
            let settings_item =
                MenuItem::with_id(app, "settings", "Settings...", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Tuck", true, None::<&str>)?;
            let tray_menu =
                Menu::with_items(app, &[&open_island_item, &settings_item, &quit_item])?;

            let mut tray_builder = TrayIconBuilder::new()
                .tooltip("Tuck")
                .menu(&tray_menu)
                .show_menu_on_left_click(false);

            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }

            let _tray = tray_builder
                .on_menu_event(|app: &AppHandle, event| match event.id.as_ref() {
                    "open_island" => {
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = system::open_island(&win);
                        }
                    }
                    "settings" => {
                        let a = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = system::open_settings(&a);
                        });
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // Setup Main Island Window
            if let Some(main_win) = app.get_webview_window("main") {
                // Initial center on primary monitor physical origin
                let _ = system::center_on_primary_monitor(&main_win);

                #[cfg(windows)]
                if let Ok(hwnd) = main_win.hwnd() {
                    system::apply_island_styles(HWND(hwnd.0 as _));
                }

                // Initial pass-through until cursor enters island
                let _ = main_win.set_ignore_cursor_events(true);
                let _ = main_win.show();

                // Listen for scale and display changes to keep centered, and handle keyboard focus restore
                let win_clone = main_win.clone();
                main_win.on_window_event(move |event| {
                    if matches!(
                        event,
                        WindowEvent::Moved(_)
                            | WindowEvent::Resized(_)
                            | WindowEvent::ScaleFactorChanged { .. }
                    ) {
                        system::invalidate_hit_geometry();
                    }
                    match event {
                        WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Moved(_) => {
                            let _ = system::center_on_primary_monitor(&win_clone);
                        }
                        WindowEvent::Focused(false) =>
                        {
                            #[cfg(windows)]
                            if let Ok(hwnd) = win_clone.hwnd() {
                                system::set_island_focusable(HWND(hwnd.0 as _), false);
                            }
                        }
                        _ => {}
                    }
                });
            }

            // Start background native pollers
            installed_usage::start(handle.clone());
            extensions::start(handle.clone());
            usage::start(handle.clone());
            media::start(handle.clone());
            lyrics::start(&handle);
            telemetry::start(handle.clone());
            osd_flyout::start(handle.clone());
            desktop_signals::start(handle.clone());
            visualizer::start(handle.clone());
            system::start(handle, hit_state_setup);

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                lifecycle::stop();
                media::stop();
                desktop_signals::stop();
                osd_flyout::shutdown();
                app.state::<Arc<dictation::DictationService>>().shutdown();
                clipboard_history::stop(
                    &app.state::<Arc<clipboard_history::ClipboardHistoryService>>(),
                );
            }
        });
}
