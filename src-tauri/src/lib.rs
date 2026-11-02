pub mod media;
pub mod system;

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
    let hit_state = Arc::new(system::HitState::default());
    let hit_state_setup = hit_state.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(hit_state)
        .invoke_handler(tauri::generate_handler![
            set_hit,
            system::quick_action,
            system::open_settings_cmd,
            system::open_island_cmd,
            system::get_autostart,
            system::set_autostart,
            media::media_toggle,
            media::media_next,
            media::media_prev,
            media::media_seek,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

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
                main_win.on_window_event(move |event| match event {
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
                });
            }

            // Start background native pollers
            media::start(handle.clone());
            system::start(handle, hit_state_setup);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
