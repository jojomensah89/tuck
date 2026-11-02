//! Small opt-in preferences; no audio, lyrics or clipboard contents are stored here.
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
static WRITE: Mutex<()> = Mutex::new(());
#[derive(Serialize, Deserialize)]
struct Preference {
    enabled: bool,
}
pub fn read(app: &AppHandle, name: &str) -> bool {
    use std::io::Read;
    let Ok(dir) = app.path().app_config_dir() else {
        return false;
    };
    let Ok(file) = std::fs::File::open(dir.join(format!("{name}.json"))) else {
        return false;
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
