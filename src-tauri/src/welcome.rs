use std::{fs, path::Path, sync::Arc};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

pub struct FirstRun(pub bool);

// Directory entries only: never inspect another application's files or credentials.
fn pristine(path: &Path) -> bool {
    match fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_none(),
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}
pub fn capture(app: &AppHandle) -> FirstRun {
    FirstRun(app.path().app_config_dir().is_ok_and(|p| pristine(&p)))
}
fn claim(dir: &Path) -> bool {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join("welcome-seen"))
        .is_ok()
}
#[tauri::command]
pub fn maybe_show_welcome(
    app: AppHandle,
    first: State<'_, FirstRun>,
    saved_preferences: bool,
    service: State<'_, Arc<crate::dictation::DictationService>>,
) -> Result<(), String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|_| "Settings unavailable")?;
    fs::create_dir_all(&dir).map_err(|_| "Settings unavailable")?;
    // Claim before opening: failed writes suppress the screen, never a repeated popup.
    if !claim(&dir) || !first.0 || saved_preferences {
        return Ok(());
    }
    #[cfg(windows)]
    let fullscreen = app
        .get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .is_none_or(|raw| {
            crate::system::check_fullscreen(windows::Win32::Foundation::HWND(raw.0 as _), None)
        });
    #[cfg(not(windows))]
    let fullscreen = crate::system::fullscreen_now();
    if fullscreen || service.update_blocker().is_some() {
        return Ok(());
    }
    crate::system::open_settings_view(&app, "welcome", false)
}
#[tauri::command]
pub fn open_about_link(app: AppHandle, url: String) -> Result<(), String> {
    const URLS: &[&str] = &[
        "https://github.com/jojomensah89/tuck",
        "https://github.com/jojomensah89/tuck/issues/new",
        "https://github.com/jojomensah89/tuck/blob/main/LICENSE",
        "https://github.com/jojomensah89/tuck/blob/main/THIRD_PARTY_NOTICES.md",
    ];
    if !URLS.contains(&url.as_str()) {
        return Err("Link not allowed".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| "Browser unavailable".into())
}
#[cfg(test)]
mod tests {
    #[test]
    fn unreadable_or_existing_marker_stays_quiet() {
        let root = std::env::temp_dir().join(format!("tuck-marker-{}", uuid::Uuid::new_v4()));
        assert!(!super::claim(&root));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("welcome-seen")).unwrap();
        assert!(!super::claim(&root));
        std::fs::remove_dir(root.join("welcome-seen")).unwrap();
        assert!(super::claim(&root));
        assert!(!super::claim(&root));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn first_run_decision() {
        let root = std::env::temp_dir().join(format!("tuck-welcome-{}", uuid::Uuid::new_v4()));
        assert!(super::pristine(&root));
        std::fs::create_dir(&root).unwrap();
        assert!(super::pristine(&root));
        std::fs::write(root.join("dictation.json"), "{}").unwrap();
        assert!(!super::pristine(&root));
        assert!(!super::pristine(&root.join("dictation.json"))); // unreadable as a directory
        std::fs::remove_dir_all(root).unwrap();
    }
}
