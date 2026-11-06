//! Small service preferences; no audio, lyrics or clipboard contents are stored here.
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
static WRITE: Mutex<()> = Mutex::new(());
#[derive(Serialize, Deserialize)]
struct Preference {
    enabled: bool,
}
pub fn read(app: &AppHandle, name: &str) -> bool {
    read_with_default(app, name, false)
}
/// Use the default only for an absent file; invalid or unreadable preferences stay off.
pub fn read_with_default(app: &AppHandle, name: &str, default: bool) -> bool {
    let Ok(dir) = app.path().app_config_dir() else {
        return false;
    };
    read_file(&dir.join(format!("{name}.json")), default)
}
fn read_file(path: &std::path::Path, default: bool) -> bool {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return default,
        Err(_) => return false,
    };
    let mut bytes = Vec::new();
    if file.take(1025).read_to_end(&mut bytes).is_err() || bytes.len() > 1024 {
        return false;
    }
    serde_json::from_slice::<Preference>(&bytes).is_ok_and(|value| value.enabled)
}
pub fn write(
    app: &AppHandle,
    name: &str,
    enabled: bool,
) -> Result<std::sync::MutexGuard<'static, ()>, String> {
    let _write = WRITE.lock().map_err(|_| "Service preference unavailable")?;
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|_| "Settings directory unavailable")?;
    std::fs::create_dir_all(&dir).map_err(|_| "Could not create settings directory")?;
    let path = dir.join(format!("{name}.json"));
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(&Preference { enabled })
        .map_err(|_| "Could not encode service preference")?;
    std::fs::write(&temporary, bytes)
        .and_then(|_| std::fs::rename(&temporary, &path))
        .map_err(|_| "Could not save service preference")?;
    Ok(_write)
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_on_applies_only_to_missing_preferences() {
        let dir = std::env::temp_dir().join(format!("tuck-default-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("flyout.json");
        assert!(super::read_file(&path, true));
        assert!(!super::read_file(&path, false));
        std::fs::write(&path, br#"{"enabled":false}"#).unwrap();
        assert!(!super::read_file(&path, true));
        std::fs::write(&path, br#"{"enabled":true}"#).unwrap();
        assert!(super::read_file(&path, true));
        std::fs::write(&path, b"invalid").unwrap();
        assert!(!super::read_file(&path, true));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn windows_rename_replaces_an_existing_preference_file() {
        let dir = std::env::temp_dir().join(format!("tuck-pref-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("preference.json");
        let temporary = dir.join("preference.tmp");
        for enabled in [false, true, false] {
            let bytes = serde_json::to_vec(&super::Preference { enabled }).unwrap();
            std::fs::write(&temporary, bytes).unwrap();
            std::fs::rename(&temporary, &path).unwrap();
            let saved: super::Preference =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved.enabled, enabled);
        }
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
