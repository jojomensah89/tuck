use crate::system::{self, PendingUpdateInfo};
use std::{sync::Mutex, time::Duration};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

#[derive(Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    info: Option<PendingUpdateInfo>,
    phase: &'static str,
    progress: Option<f64>,
    error: Option<String>,
    blocker: Option<String>,
}
#[derive(Default)]
struct Inner {
    snapshot: Snapshot,
    update: Option<Update>,
    bytes: Option<Vec<u8>>,
}
#[derive(Default)]
pub(crate) struct UpdateService {
    inner: Mutex<Inner>,
    operation: tokio::sync::Mutex<()>,
}
fn publish(app: &AppHandle, inner: &Inner) {
    let _ = app.emit("update-state-changed", &inner.snapshot);
    if let Some(tray) = app.try_state::<system::TrayItems>() {
        let label = match inner.snapshot.phase {
            "downloading" => "Downloading update".into(),
            "restart" => "Restart to update".into(),
            "installing" => "Installing update".into(),
            _ => inner
                .snapshot
                .info
                .as_ref()
                .map(|i| format!("Update to v{}", i.version))
                .unwrap_or_else(|| "Check for updates".into()),
        };
        let _ = tray.check_updates.set_text(label);
    }
}
#[tauri::command]
pub(crate) fn update_snapshot(state: State<'_, UpdateService>) -> Snapshot {
    state.inner.lock().unwrap().snapshot.clone()
}
#[tauri::command]
pub(crate) async fn update_check(
    app: AppHandle,
    state: State<'_, UpdateService>,
) -> Result<Option<PendingUpdateInfo>, String> {
    if crate::package::has_package_identity() {
        return Ok(None);
    }
    let _operation = state.operation.lock().await;
    if state.inner.lock().unwrap().bytes.is_some() {
        return Ok(state.inner.lock().unwrap().snapshot.info.clone());
    }
    let found = app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let cached = app
        .path()
        .app_cache_dir()
        .ok()
        .map(|dir| dir.join("pending-update.bin"));
    let restored = cached
        .as_ref()
        .and_then(|path| std::fs::read(path).ok())
        .filter(|bytes| {
            found
                .as_ref()
                .is_some_and(|update| update.verify(bytes).is_ok())
        });
    if restored.is_none() {
        if let Some(path) = &cached {
            let _ = std::fs::remove_file(path);
        }
    }
    let info = found.as_ref().map(|u| PendingUpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        body: u.body.clone(),
        date: u.date.map(|d| d.to_string()),
    });
    let mut inner = state.inner.lock().unwrap();
    inner.update = found;
    inner.bytes = restored;
    inner.snapshot = Snapshot {
        info: info.clone(),
        phase: if inner.bytes.is_some() {
            "restart"
        } else {
            "available"
        },
        ..Default::default()
    };
    system::set_pending_update(app.clone(), app.state(), info.clone())?;
    publish(&app, &inner);
    Ok(info)
}
#[tauri::command]
pub(crate) async fn update_action(
    app: AppHandle,
    state: State<'_, UpdateService>,
) -> Result<(), String> {
    if crate::package::has_package_identity() {
        return Err("Updates come from the Microsoft Store".into());
    }
    let Ok(_operation) = state.operation.try_lock() else {
        return Ok(());
    };
    if let Err(reason) = system::ensure_update_allowed(&app) {
        let mut inner = state.inner.lock().unwrap();
        inner.snapshot.blocker = Some(reason.clone());
        publish(&app, &inner);
        return Err(reason);
    }
    let (mut update, bytes) = {
        let mut inner = state.inner.lock().unwrap();
        let update = inner.update.clone().ok_or("No update available")?;
        let bytes = inner.bytes.take();
        inner.snapshot.error = None;
        inner.snapshot.blocker = None;
        inner.snapshot.phase = action_phase(bytes.is_some());
        inner.snapshot.progress = None;
        publish(&app, &inner);
        (update, bytes)
    };
    if let Some(bytes) = bytes {
        if let Err(reason) = system::prepare_update_install(app.clone()) {
            let mut inner = state.inner.lock().unwrap();
            inner.bytes = Some(bytes);
            inner.snapshot.phase = "restart";
            inner.snapshot.blocker = Some(reason.clone());
            publish(&app, &inner);
            return Err(reason);
        }
        if let Err(error) = update.install(&bytes) {
            let mut inner = state.inner.lock().unwrap();
            inner.bytes = Some(bytes);
            inner.snapshot.phase = "available";
            inner.snapshot.error = Some(error.to_string());
            publish(&app, &inner);
            eprintln!("Update install failed: {error}");
            system::restart_after_failed_update(app);
            return Err(error.to_string());
        }
    } else {
        update.timeout = Some(Duration::from_secs(600));
        let mut downloaded = 0u64;
        let mut last_percent = None;
        let result = update
            .download(
                |chunk, total| {
                    downloaded += chunk as u64;
                    let percent = whole_percent(downloaded, total);
                    // Chunks arrive far faster than the ring can show them
                    if percent == last_percent {
                        return;
                    }
                    last_percent = percent;
                    let mut inner = state.inner.lock().unwrap();
                    inner.snapshot.progress = percent.map(f64::from);
                    publish(&app, &inner);
                },
                || {},
            )
            .await;
        let mut inner = state.inner.lock().unwrap();
        let result = result.map_err(|error| error.to_string()).and_then(|bytes| {
            let dir = app
                .path()
                .app_cache_dir()
                .map_err(|error| error.to_string())?;
            std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
            std::fs::write(dir.join("pending-update.bin"), &bytes)
                .map_err(|error| format!("Could not cache update: {error}"))?;
            Ok(bytes)
        });
        let result = inner.finish_download(result);
        publish(&app, &inner);
        result?;
    }
    Ok(())
}

fn whole_percent(downloaded: u64, total: Option<u64>) -> Option<u8> {
    let total = total.filter(|total| *total > 0)?;
    Some((downloaded.saturating_mul(100) / total).min(100) as u8)
}

fn action_phase(downloaded: bool) -> &'static str {
    if downloaded {
        "installing"
    } else {
        "downloading"
    }
}
impl Inner {
    fn finish_download(&mut self, result: Result<Vec<u8>, String>) -> Result<(), String> {
        match result {
            Ok(bytes) => {
                self.bytes = Some(bytes);
                self.snapshot.phase = "restart";
                self.snapshot.error = None;
                Ok(())
            }
            Err(error) => {
                self.snapshot.phase = "available";
                self.snapshot.error = Some(error.clone());
                eprintln!("Update download failed: {error}");
                Err(error)
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_is_reported_once_per_whole_percent_and_never_invented() {
        assert_eq!(whole_percent(1, None), None);
        assert_eq!(whole_percent(1, Some(0)), None);
        assert_eq!(whole_percent(0, Some(1000)), Some(0));
        assert_eq!(whole_percent(9, Some(1000)), Some(0));
        assert_eq!(whole_percent(420, Some(1000)), Some(42));
        assert_eq!(whole_percent(2000, Some(1000)), Some(100));
    }
    #[test]
    fn download_completion_requires_a_second_click_and_keeps_bytes_for_retries() {
        let mut inner = Inner::default();
        assert_eq!(action_phase(inner.bytes.is_some()), "downloading");
        inner.finish_download(Ok(vec![1, 2, 3])).unwrap();
        assert_eq!(inner.snapshot.phase, "restart");
        assert_eq!(inner.bytes, Some(vec![1, 2, 3]));
        assert_eq!(action_phase(inner.bytes.is_some()), "installing");
        // A refusal or installer error preserves the verified download.
        inner.snapshot.phase = "available";
        inner.snapshot.error = Some("Installer failed".into());
        assert_eq!(action_phase(inner.bytes.is_some()), "installing");
    }
    #[test]
    fn download_failure_returns_to_available_and_success_clears_error() {
        let mut inner = Inner::default();
        inner.snapshot.phase = "downloading";
        assert!(inner
            .finish_download(Err("network failure".into()))
            .is_err());
        assert_eq!(inner.snapshot.phase, "available");
        assert!(inner.bytes.is_none());
        inner.finish_download(Ok(vec![42])).unwrap();
        assert_eq!(inner.snapshot.phase, "restart");
        assert!(inner.snapshot.error.is_none());
    }
    #[test]
    fn shared_operation_lock_rejects_a_second_window_action() {
        let service = UpdateService::default();
        let first_window = service.operation.try_lock().unwrap();
        assert!(service.operation.try_lock().is_err());
        drop(first_window);
        assert!(service.operation.try_lock().is_ok());
    }
}
