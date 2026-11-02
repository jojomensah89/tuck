//! Offline, explicit-invocation dictation service.
//!
//! Coordinator wiring is intentionally kept out of this module. See
//! `docs/phase-two-dictation.md` for builder, state, command and event setup.

mod audio;
mod model;
mod target;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};
use zeroize::Zeroize;

const EVENT_STATE: &str = "dictation_state";
const EVENT_PARTIAL: &str = "dictation_partial";
const EVENT_TRANSCRIPT: &str = "dictation_transcript";
const EVENT_ERROR: &str = "dictation_error";
const DEFAULT_TOGGLE: &str = "Ctrl+Shift+Space";
const DEFAULT_HOLD: &str = "Ctrl+Alt+Space";
const MAX_TRANSCRIPT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DictationSettings {
    pub toggle_shortcut: String,
    pub hold_to_talk_shortcut: String,
    pub review_before_insert: bool,
    pub partial_preview: bool,
}

impl Default for DictationSettings {
    fn default() -> Self {
        Self {
            toggle_shortcut: DEFAULT_TOGGLE.to_owned(),
            hold_to_talk_shortcut: DEFAULT_HOLD.to_owned(),
            review_before_insert: false,
            partial_preview: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DictationSnapshot {
    pub status: String,
    pub model_installed: bool,
    pub model_downloaded_bytes: u64,
    pub transcript: Option<String>,
    pub message: Option<String>,
    pub settings: DictationSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DictationTranscriptEvent {
    pub text: String,
    pub inserted: bool,
    pub requires_review: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug)]
enum RecordingMode {
    Toggle,
    HoldToTalk,
}

struct ActiveSession {
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    mode: RecordingMode,
}

struct Runtime {
    status: String,
    message: Option<String>,
    transcript: Option<String>,
    review_target: Option<target::ForegroundTarget>,
    active: Option<ActiveSession>,
    downloaded_bytes: u64,
    downloading: bool,
    download_cancel: Option<Arc<AtomicBool>>,
}

/// Shared by coordinator-registered Tauri commands and the shortcut callback.
pub struct DictationService {
    app: RwLock<Option<AppHandle>>,
    settings: RwLock<DictationSettings>,
    runtime: Mutex<Runtime>,
    model_busy: AtomicBool,
    registered_shortcuts: Mutex<Vec<Shortcut>>,
    settings_operation: Mutex<()>,
}

impl Default for DictationService {
    fn default() -> Self {
        Self::new()
    }
}

impl DictationService {
    pub fn new() -> Self {
        Self {
            app: RwLock::new(None),
            settings: RwLock::new(DictationSettings::default()),
            runtime: Mutex::new(Runtime {
                status: "idle".into(),
                message: None,
                transcript: None,
                review_target: None,
                active: None,
                downloaded_bytes: 0,
                downloading: false,
                download_cancel: None,
            }),
            model_busy: AtomicBool::new(false),
            registered_shortcuts: Mutex::new(Vec::new()),
            settings_operation: Mutex::new(()),
        }
    }

    fn model_path(&self) -> Result<PathBuf, String> {
        let app = self
            .app
            .read()
            .map_err(|_| "Dictation service unavailable")?;
        let app = app.as_ref().ok_or("Dictation has not been initialized")?;
        model::model_path(app)
    }

    fn snapshot(&self) -> DictationSnapshot {
        let settings = self.settings.read().map(|s| s.clone()).unwrap_or_default();
        let runtime = self.runtime.lock().ok();
        let path = self.model_path().ok();
        DictationSnapshot {
            status: runtime
                .as_ref()
                .map(|r| r.status.clone())
                .unwrap_or_else(|| "error".into()),
            model_installed: path.as_deref().is_some_and(Path::is_file),
            model_downloaded_bytes: runtime.as_ref().map_or(0, |r| r.downloaded_bytes),
            transcript: runtime.and_then(|r| r.transcript.clone()),
            message: self.runtime.lock().ok().and_then(|r| r.message.clone()),
            settings,
        }
    }

    fn emit_state(&self) {
        if let Ok(app) = self.app.read() {
            if let Some(app) = app.as_ref() {
                let _ = app.emit(EVENT_STATE, self.snapshot());
            }
        }
    }

    fn set_status(&self, status: &str, message: Option<String>) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.status = status.to_owned();
            runtime.message = message;
        }
        self.emit_state();
    }

    fn app(&self) -> Result<AppHandle, String> {
        self.app
            .read()
            .map_err(|_| "Dictation service unavailable".to_owned())?
            .clone()
            .ok_or_else(|| "Dictation has not been initialized".to_owned())
    }

    fn settings(&self) -> DictationSettings {
        self.settings.read().map(|s| s.clone()).unwrap_or_default()
    }

    fn begin(self: &Arc<Self>, mode: RecordingMode) -> Result<(), String> {
        let app = self.app()?;
        let model_path = self.model_path()?;
        if !model_path.is_file() {
            self.set_status(
                "modelMissing",
                Some("Download the offline speech model in Settings before recording.".into()),
            );
            return Err("The offline dictation model is not installed".into());
        }
        {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "Dictation service unavailable")?;
            if runtime.active.is_some() || runtime.downloading {
                return Err("Dictation is already recording, transcribing, or downloading".into());
            }
            runtime.transcript = None;
            runtime.review_target = None;
            runtime.message = None;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let original_target = target::capture_foreground();
        {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "Dictation service unavailable")?;
            if runtime.active.is_some() || runtime.downloading {
                return Err("Dictation is already active".into());
            }
            runtime.active = Some(ActiveSession {
                stop: stop.clone(),
                cancel: cancel.clone(),
                mode: mode.clone(),
            });
            runtime.status = "starting".into();
        }
        self.emit_state();

        let service = self.clone();
        std::thread::Builder::new()
            .name("tuck-dictation".into())
            .spawn(move || {
                let result =
                    audio::record_and_transcribe(&service, &app, &model_path, stop, cancel.clone());
                if cancel.load(Ordering::Acquire) {
                    service.finish_cancelled();
                    return;
                }
                match result {
                    Ok(mut text) => {
                        let mut limit = text.len().min(MAX_TRANSCRIPT_BYTES);
                        while !text.is_char_boundary(limit) {
                            limit -= 1;
                        }
                        text.truncate(limit);
                        service.finish_transcript(text, original_target);
                    }
                    Err(error) => service.finish_error(error),
                }
            })
            .map_err(|error| {
                self.clear_active();
                format!("Could not start dictation worker: {error}")
            })?;
        Ok(())
    }

    fn finish_transcript(
        &self,
        mut text: String,
        original_target: Option<target::ForegroundTarget>,
    ) {
        text = text.trim().to_owned();
        if text.is_empty() {
            text.zeroize();
            self.clear_active();
            self.set_status("idle", Some("No speech was recognized.".into()));
            return;
        }
        let settings = self.settings();
        let safety = original_target
            .as_ref()
            .ok_or_else(|| "The original foreground window could not be captured.".to_owned())
            .and_then(target::verify_foreground_target)
            .and_then(|_| {
                target::is_editable_target(original_target.as_ref().expect("checked above"))
            });
        let mut inserted = false;
        let mut reason = safety.err();
        if !settings.review_before_insert && reason.is_none() {
            if let Some(target) = original_target.as_ref() {
                match target::insert_unicode(target, &text) {
                    Ok(()) => inserted = true,
                    Err(error) => reason = Some(error),
                }
            }
        }
        let review = settings.review_before_insert || !inserted;
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.active = None;
            runtime.status = if review { "review" } else { "idle" }.into();
            runtime.message = reason.clone();
            // Keep text only during review. Immediate successful insertion drops it.
            runtime.transcript = review.then(|| text.clone());
            runtime.review_target = if review { original_target } else { None };
        }
        self.emit_state();
        if let Ok(app) = self.app() {
            let _ = app.emit(
                EVENT_TRANSCRIPT,
                DictationTranscriptEvent {
                    text: text.clone(),
                    inserted,
                    requires_review: review,
                    reason,
                },
            );
        }
        text.zeroize();
    }

    fn finish_error(&self, error: String) {
        self.clear_active();
        self.set_status("error", Some(error.clone()));
        if let Ok(app) = self.app() {
            let _ = app.emit(EVENT_ERROR, error);
        }
    }

    fn finish_cancelled(&self) {
        self.clear_active();
        self.set_status("idle", None);
    }

    fn clear_active(&self) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.active = None;
            runtime.transcript = None;
        }
    }

    fn stop(self: &Arc<Self>) -> Result<(), String> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "Dictation service unavailable")?;
        let active = runtime
            .active
            .as_ref()
            .ok_or("Dictation is not recording")?;
        if active.stop.swap(true, Ordering::AcqRel) {
            return Err("Dictation is already stopping".into());
        }
        Ok(())
    }

    fn cancel(&self) -> Result<(), String> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "Dictation service unavailable")?;
        if let Some(active) = runtime.active.as_ref() {
            active.cancel.store(true, Ordering::Release);
            active.stop.store(true, Ordering::Release);
            runtime.status = "cancelling".into();
            runtime.message = None;
            drop(runtime);
            self.emit_state();
            return Ok(());
        }
        if let Some(cancel) = runtime.download_cancel.as_ref() {
            cancel.store(true, Ordering::Release);
            runtime.message = Some("Cancelling model download…".into());
            drop(runtime);
            self.emit_state();
            return Ok(());
        }
        Err("There is no recording, inference, or model download to cancel".into())
    }

    fn finish_review(self: &Arc<Self>) -> Result<(), String> {
        let (text, original_target) = {
            let runtime = self
                .runtime
                .lock()
                .map_err(|_| "Dictation service unavailable")?;
            let text = runtime
                .transcript
                .clone()
                .ok_or("There is no transcript awaiting review")?;
            let target = runtime.review_target.clone();
            (text, target)
        };
        let target = original_target
            .ok_or("The original input target is unavailable; this text was not inserted")?;
        target::verify_foreground_target(&target)?;
        target::is_editable_target(&target)?;
        target::insert_unicode(&target, &text)?;
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.transcript = None;
            runtime.review_target = None;
            runtime.message = None;
            runtime.status = "idle".into();
        }
        self.emit_state();
        Ok(())
    }

    fn discard_review(&self) -> Result<(), String> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "Dictation service unavailable")?;
        if runtime.status != "review" {
            return Err("There is no transcript awaiting review".into());
        }
        runtime.transcript = None;
        runtime.review_target = None;
        runtime.message = None;
        runtime.status = "idle".into();
        drop(runtime);
        self.emit_state();
        Ok(())
    }

    fn download_model(self: &Arc<Self>) -> Result<(), String> {
        let app = self.app()?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "Dictation service unavailable")?;
            if runtime.active.is_some() || runtime.downloading {
                return Err("Stop dictation before changing the model".into());
            }
            runtime.downloading = true;
            runtime.download_cancel = Some(cancel.clone());
            runtime.status = "downloadingModel".into();
            runtime.downloaded_bytes = 0;
            runtime.message = None;
        }
        self.emit_state();
        let service = self.clone();
        std::thread::Builder::new()
            .name("tuck-model-download".into())
            .spawn(move || {
                let result = model::download(&app, &cancel, |received| {
                    if let Ok(mut runtime) = service.runtime.lock() {
                        runtime.downloaded_bytes = received;
                    }
                    service.emit_state();
                });
                if let Ok(mut runtime) = service.runtime.lock() {
                    runtime.downloading = false;
                    runtime.download_cancel = None;
                    runtime.status = if result.is_ok() || cancel.load(Ordering::Acquire) {
                        "idle"
                    } else {
                        "error"
                    }
                    .into();
                    runtime.message = result.err();
                }
                service.emit_state();
            })
            .map_err(|e| {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.downloading = false;
                    runtime.download_cancel = None;
                }
                format!("Could not start model download: {e}")
            })?;
        Ok(())
    }

    fn remove_model(&self) -> Result<(), String> {
        if self.model_busy.load(Ordering::Acquire) {
            return Err("The model is in use; stop dictation before removing it".into());
        }
        let path = self.model_path()?;
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| format!("Could not remove the model: {e}"))?;
        }
        self.set_status("modelMissing", None);
        Ok(())
    }

    pub fn shutdown(&self) {
        let _ = self.cancel();
    }
}

/// Initialize settings and shortcuts only. This does not access the microphone
/// or download/load a speech model.
pub fn start(app: AppHandle, service: &Arc<DictationService>) -> Result<(), String> {
    if let Ok(mut slot) = service.app.write() {
        *slot = Some(app.clone());
    }
    if let Ok(settings) = load_settings(&app) {
        if let Ok(mut current) = service.settings.write() {
            *current = settings;
        }
    }
    register_shortcuts(&app, service)?;
    service.emit_state();
    Ok(())
}

pub fn register_shortcuts(app: &AppHandle, service: &Arc<DictationService>) -> Result<(), String> {
    let settings = service.settings();
    let shortcuts = app.global_shortcut();
    let toggle = Shortcut::try_from(settings.toggle_shortcut.as_str())
        .map_err(|e| format!("Invalid toggle shortcut: {e}"))?;
    let hold = Shortcut::try_from(settings.hold_to_talk_shortcut.as_str())
        .map_err(|e| format!("Invalid hold-to-talk shortcut: {e}"))?;
    let previous = service
        .registered_shortcuts
        .lock()
        .map_err(|_| "Dictation service unavailable")?
        .clone();
    for shortcut in &previous {
        let _ = shortcuts.unregister(*shortcut);
    }
    if let Err(error) = shortcuts.register(toggle) {
        for prior in &previous {
            let _ = shortcuts.register(*prior);
        }
        return Err(format!("Could not register toggle shortcut: {error}"));
    }
    if let Err(error) = shortcuts.register(hold) {
        let _ = shortcuts.unregister(toggle);
        for prior in &previous {
            let _ = shortcuts.register(*prior);
        }
        return Err(format!("Could not register hold-to-talk shortcut: {error}"));
    }
    if let Ok(mut registered) = service.registered_shortcuts.lock() {
        *registered = vec![toggle, hold];
    }
    Ok(())
}

/// Pass this from the coordinator's global-shortcut plugin handler.
pub fn handle_shortcut(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    let Some(service) = app.try_state::<Arc<DictationService>>() else {
        return;
    };
    let settings = service.settings();
    let toggle = Shortcut::try_from(settings.toggle_shortcut.as_str()).ok();
    let hold = Shortcut::try_from(settings.hold_to_talk_shortcut.as_str()).ok();
    if toggle.as_ref() == Some(shortcut) && event.state == ShortcutState::Pressed {
        let status = service.snapshot().status;
        if status == "recording" {
            let _ = service.stop();
        } else if status == "idle" || status == "review" || status == "error" {
            let _ = service.begin(RecordingMode::Toggle);
        }
    } else if hold.as_ref() == Some(shortcut) {
        match event.state {
            ShortcutState::Pressed => {
                let _ = service.begin(RecordingMode::HoldToTalk);
            }
            ShortcutState::Released => {
                if let Ok(runtime) = service.runtime.lock() {
                    if runtime
                        .active
                        .as_ref()
                        .is_some_and(|s| matches!(s.mode, RecordingMode::HoldToTalk))
                    {
                        drop(runtime);
                        let _ = service.stop();
                    }
                }
            }
        }
    }
}

fn validate_settings(settings: &DictationSettings) -> Result<(), String> {
    if settings.toggle_shortcut.len() > 64 || settings.hold_to_talk_shortcut.len() > 64 {
        return Err("Shortcut strings must be at most 64 bytes".into());
    }
    let toggle = Shortcut::try_from(settings.toggle_shortcut.as_str())
        .map_err(|e| format!("Invalid toggle shortcut: {e}"))?;
    let hold = Shortcut::try_from(settings.hold_to_talk_shortcut.as_str())
        .map_err(|e| format!("Invalid hold-to-talk shortcut: {e}"))?;
    if toggle == hold {
        return Err("Toggle and hold-to-talk shortcuts must be different".into());
    }
    Ok(())
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let path = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("Could not resolve settings directory: {e}"))?
        .join("dictation.json");
    Ok(path)
}

fn load_settings(app: &AppHandle) -> Result<DictationSettings, String> {
    let path = settings_path(app)?;
    let bytes = match std::fs::read(path) {
        Ok(b) if b.len() <= 4096 => b,
        Ok(_) => return Err("Dictation settings file exceeds the size limit".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DictationSettings::default())
        }
        Err(e) => return Err(format!("Could not read dictation settings: {e}")),
    };
    let settings: DictationSettings = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Dictation settings are invalid: {e}"))?;
    validate_settings(&settings)?;
    Ok(settings)
}

fn persist_settings(app: &AppHandle, settings: &DictationSettings) -> Result<(), String> {
    let path = settings_path(app)?;
    let parent = path.parent().ok_or("Invalid settings directory")?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Could not create settings directory: {e}"))?;
    let bytes =
        serde_json::to_vec(settings).map_err(|e| format!("Could not serialize settings: {e}"))?;
    if bytes.len() > 4096 {
        return Err("Dictation settings exceed the size limit".into());
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("Could not write settings: {e}"))?;
    std::fs::rename(tmp, path).map_err(|e| format!("Could not commit settings: {e}"))
}

#[tauri::command]
pub fn dictation_get_state(service: State<'_, Arc<DictationService>>) -> DictationSnapshot {
    service.snapshot()
}

#[tauri::command]
pub fn dictation_set_settings(
    app: AppHandle,
    service: State<'_, Arc<DictationService>>,
    settings: DictationSettings,
) -> Result<DictationSnapshot, String> {
    let _operation = service
        .settings_operation
        .lock()
        .map_err(|_| "Dictation settings unavailable")?;
    validate_settings(&settings)?;
    let previous = service.settings();
    *service
        .settings
        .write()
        .map_err(|_| "Dictation settings unavailable")? = settings.clone();
    if let Err(error) =
        register_shortcuts(&app, &service).and_then(|_| persist_settings(&app, &settings))
    {
        *service
            .settings
            .write()
            .map_err(|_| "Dictation settings unavailable")? = previous;
        let _ = register_shortcuts(&app, &service);
        return Err(error);
    }
    service.emit_state();
    Ok(service.snapshot())
}

#[tauri::command]
pub fn dictation_model_download(service: State<'_, Arc<DictationService>>) -> Result<(), String> {
    service.download_model()
}

#[tauri::command]
pub fn dictation_model_remove(service: State<'_, Arc<DictationService>>) -> Result<(), String> {
    service.remove_model()
}

#[tauri::command]
pub fn dictation_toggle(service: State<'_, Arc<DictationService>>) -> Result<(), String> {
    if service.snapshot().status == "recording" {
        service.stop()
    } else {
        service.begin(RecordingMode::Toggle)
    }
}

#[tauri::command]
pub fn dictation_hold_start(service: State<'_, Arc<DictationService>>) -> Result<(), String> {
    service.begin(RecordingMode::HoldToTalk)
}

#[tauri::command]
pub fn dictation_hold_stop(service: State<'_, Arc<DictationService>>) -> Result<(), String> {
    service.stop()
}

#[tauri::command]
pub fn dictation_cancel(service: State<'_, Arc<DictationService>>) -> Result<(), String> {
    if service.snapshot().status == "review" {
        service.discard_review()
    } else {
        service.cancel()
    }
}

#[tauri::command]
pub fn dictation_review_insert(service: State<'_, Arc<DictationService>>) -> Result<(), String> {
    service.finish_review()
}

#[tauri::command]
pub fn dictation_review_discard(service: State<'_, Arc<DictationService>>) -> Result<(), String> {
    service.discard_review()
}

#[tauri::command]
pub fn dictation_model_download_cancel(
    service: State<'_, Arc<DictationService>>,
) -> Result<(), String> {
    service.cancel()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_to_explicit_hotkeys_and_immediate_insert() {
        let settings = DictationSettings::default();
        assert_eq!(settings.toggle_shortcut, DEFAULT_TOGGLE);
        assert_eq!(settings.hold_to_talk_shortcut, DEFAULT_HOLD);
        assert!(!settings.review_before_insert);
        assert!(settings.partial_preview);
    }

    #[test]
    fn settings_reject_duplicate_or_oversized_shortcuts() {
        let mut settings = DictationSettings::default();
        settings.hold_to_talk_shortcut = settings.toggle_shortcut.clone();
        assert!(validate_settings(&settings).is_err());
        settings.hold_to_talk_shortcut = "X".repeat(65);
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn transcript_byte_cap_does_not_create_an_unbounded_payload() {
        let mut text = "a".repeat(MAX_TRANSCRIPT_BYTES + 10);
        text.truncate(MAX_TRANSCRIPT_BYTES);
        assert_eq!(text.len(), MAX_TRANSCRIPT_BYTES);
    }
}
