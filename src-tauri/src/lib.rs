mod antigravity_usage;
pub mod clipboard_history;
pub mod desktop_signals;
pub mod dictation;
pub mod extensions;
#[cfg(windows)]
mod file_drop;
mod idle_webview;
pub mod installed_usage;
mod legacy_credentials;
pub mod lifecycle;
pub mod lyrics;
mod lyrics_match;
pub mod media;
mod network_rates;
pub mod osd_flyout;
mod package;
mod sensors;
mod service_preferences;
mod shell_thumbs;
pub mod system;
pub mod telemetry;
mod update;
pub mod usage;
mod welcome;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WindowEvent,
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
                if let Err(error) = system::open_settings_cmd(handle, None).await {
                    eprintln!("Single-instance settings focus: {error}");
                }
            });
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    sensors::handle_shortcut(app, shortcut, event);
                    dictation::handle_shortcut(app, shortcut, event);
                })
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(hit_state)
        .manage(system::PendingUpdateState::default())
        .manage(update::UpdateService::default())
        .invoke_handler(tauri::generate_handler![
            sensors::sensor_shortcuts_pause,
            sensors::sensor_settings,
            sensors::sensor_configure,
            sensors::sensor_snapshot,
            package::is_store_build,
            package::autostart_status,
            welcome::maybe_show_welcome,
            welcome::open_about_link,
            set_hit,
            update::update_check,
            update::update_snapshot,
            update::update_action,
            system::get_pending_update,
            system::set_pending_update,
            system::check_update_allowed,
            system::prepare_update_install,
            system::restart_after_failed_update,
            idle_webview::island_activity,
            dictation::dictation_get_state,
            dictation::dictation_keep_warm,
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
            clipboard_history::clipboard_thumbnail,
            lyrics::lyrics_status,
            lyrics::lyrics_set_enabled,
            lyrics::lyrics_get,
            lyrics::lyrics_clear_cache,
            telemetry::telemetry_now,
            telemetry::telemetry_history,
            media::media_sessions,
            media::media_select_session,
            installed_usage::installed_usage_providers,
            installed_usage::set_usage_provider,
            installed_usage::refresh_installed_usage,
            extensions::extension_snapshot,
            extensions::extension_settings,
            extensions::shelf_add,
            extensions::shelf_pick,
            extensions::shelf_remove,
            extensions::shelf_reveal,
            extensions::shelf_open,
            extensions::shelf_thumbnail,
            extensions::shelf_drag,
            extensions::notification_access,
            usage::usage_now,
            usage::refresh_usage,
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
            app.manage(welcome::capture(app.handle()));
            let handle = app.handle().clone();
            legacy_credentials::run(&handle);
            // Missing shortcut availability must not prevent the island from launching.
            if let Err(error) = dictation::start(handle.clone(), &dictation_setup) {
                eprintln!("Dictation startup: {error}");
            }
            if let Err(error) = clipboard_history::start(handle.clone(), &clipboard_setup) {
                eprintln!("Clipboard startup: {error}");
            }

            // Setup System Tray Menu:
            // 1. Open island
            // 2. Settings...
            // 3. Separator
            // 4. Launch at sign-in (checkable, synchronized with autostart)
            // 5. Check for updates (reads 'Update available (vX.Y.Z)' when one is found)
            // 6. Separator
            // 7. About Tuck (shows the version)
            // 8. Quit Tuck
            let autostart_enabled =
                tauri::async_runtime::block_on(system::get_autostart(handle.clone()))
                    .unwrap_or(false);

            let open_island_item =
                MenuItem::with_id(app, "open_island", "Open island", true, None::<&str>)?;
            let settings_item =
                MenuItem::with_id(app, "settings", "Settings...", true, None::<&str>)?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let autostart_item = CheckMenuItem::with_id(
                app,
                "autostart",
                "Launch at sign-in",
                true,
                autostart_enabled,
                None::<&str>,
            )?;
            let check_updates_item = MenuItem::with_id(
                app,
                "check_updates",
                "Check for updates",
                true,
                None::<&str>,
            )?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let about_item = MenuItem::with_id(app, "about", "About Tuck", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Tuck", true, None::<&str>)?;

            let tray_menu = Menu::with_items(
                app,
                &[
                    &open_island_item,
                    &settings_item,
                    &sep1,
                    &autostart_item,
                    &check_updates_item,
                    &sep2,
                    &about_item,
                    &quit_item,
                ],
            )?;

            if package::has_package_identity() {
                tray_menu.remove(&check_updates_item)?;
            }

            app.manage(system::TrayItems {
                autostart: autostart_item.clone(),
                check_updates: check_updates_item.clone(),
            });

            let tray_builder = TrayIconBuilder::with_id("main")
                .tooltip("Tuck")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .icon(tauri::image::Image::new(
                    include_bytes!("../icons/tray.rgba"),
                    32,
                    32,
                ))
                .icon_as_template(true);

            let _tray = tray_builder
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.emit("tray-toggle-island", ());
                        }
                    }
                })
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
                    "autostart" => {
                        let a = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let current = system::get_autostart(a.clone()).await.unwrap_or(false);
                            let _ = system::set_autostart(a.clone(), !current).await;
                            // Windows may keep the old state (Task Manager or policy): show what it did
                            let actual = system::get_autostart(a.clone()).await.unwrap_or(current);
                            system::update_tray_autostart(&a, actual);
                            let _ = a.emit("autostart-changed", actual);
                        });
                    }
                    "check_updates" => {
                        let a = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = system::open_settings_view(&a, "about", true);
                        });
                    }
                    "about" => {
                        let a = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = system::open_settings_view(&a, "about", false);
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

                #[cfg(windows)]
                file_drop::start(&main_win);

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
                        WindowEvent::DragDrop(event) => {
                            #[cfg(debug_assertions)]
                            if std::env::var_os("TUCK_DROP_TRACE").is_some() {
                                eprintln!("drop-event {event:?}");
                            }
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
            sensors::start(handle.clone());
            desktop_signals::start(handle.clone());
            system::start(handle, hit_state_setup);

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                run_shutdown_cleanup(app);
            }
        });
}

/// Central shutdown cleanup: cancels signals, un-hides native OS flyout, and shuts down dictation & clipboard.
pub fn run_shutdown_cleanup(app: &AppHandle) {
    #[cfg(windows)]
    file_drop::stop();
    lifecycle::stop();
    media::stop();
    desktop_signals::stop();
    osd_flyout::shutdown();
    if let Some(dictation) = app.try_state::<Arc<dictation::DictationService>>() {
        dictation.shutdown();
    }
    if let Some(cb) = app.try_state::<Arc<clipboard_history::ClipboardHistoryService>>() {
        clipboard_history::stop(&cb);
    }
}
