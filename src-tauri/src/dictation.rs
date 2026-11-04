//! Offline, explicit-invocation dictation service.
//!
//! Coordinator wiring is intentionally kept out of this module. See
//! `docs/phase-two-dictation.md` for builder, state, command and event setup.

mod audio;
mod dual_shift;
mod model;
mod target;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};
use zeroize::{Zeroize, Zeroizing};

const EVENT_STATE: &str = "dictation_state";
const EVENT_PARTIAL: &str = "dictation_partial";
const EVENT_TRANSCRIPT: &str = "dictation_transcript";
const EVENT_ERROR: &str = "dictation_error";
const DEFAULT_TOGGLE: &str = "Ctrl+Shift+Space";
const DEFAULT_HOLD: &str = "Shift+Shift";
const MAX_TRANSCRIPT_BYTES: usize = 64 * 1024;

pub fn is_dual_shift_shortcut(shortcut: &str) -> bool {
    let s = shortcut.trim().to_lowercase().replace(' ', "");
    matches!(
        s.as_str(),
        "shift+shift"
            | "twoshifts"
            | "twoshift"
            | "dualshift"
            | "dualshifts"
            | "leftshift+rightshift"
            | "rightshift+leftshift"
            | "lshift+rshift"
            | "rshift+lshift"
    )
}

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
    pub can_insert: bool,
}

impl Drop for DictationSnapshot {
    fn drop(&mut self) {
        if let Some(transcript) = self.transcript.as_mut() {
            transcript.zeroize();
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DictationTranscriptEvent {
    pub text: String,
    pub inserted: bool,
    pub requires_review: bool,
    pub reason: Option<String>,
}

impl Drop for DictationTranscriptEvent {
    fn drop(&mut self) {
        self.text.zeroize();
    }
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
    removing_model: bool,
    download_cancel: Option<Arc<AtomicBool>>,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(transcript) = self.transcript.as_mut() {
            transcript.zeroize();
        }
    }
}

/// Shared by coordinator-registered Tauri commands and the shortcut callback.
pub struct DictationService {
    app: RwLock<Option<AppHandle>>,
    settings: RwLock<DictationSettings>,
    runtime: Mutex<Runtime>,
    model_busy: AtomicBool,
    registered_shortcuts: Mutex<Vec<Shortcut>>,
    settings_operation: Mutex<()>,
    is_shutdown: AtomicBool,
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
                removing_model: false,
                download_cancel: None,
            }),
            model_busy: AtomicBool::new(false),
            registered_shortcuts: Mutex::new(Vec::new()),
            settings_operation: Mutex::new(()),
            is_shutdown: AtomicBool::new(false),
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
        let (status, model_downloaded_bytes, can_insert, transcript, message) = self
            .runtime
            .lock()
            .map(|runtime| {
                (
                    runtime.status.clone(),
                    runtime.downloaded_bytes,
                    runtime.status == "review" && runtime.review_target.is_some(),
                    runtime.transcript.clone(),
                    runtime.message.clone(),
                )
            })
            .unwrap_or_else(|_| ("error".into(), 0, false, None, None));
        let path = self.model_path().ok();
        DictationSnapshot {
            status,
            model_installed: path.as_deref().is_some_and(Path::is_file),
            model_downloaded_bytes,
            can_insert,
            transcript,
            message,
            settings,
        }
    }

    fn emit_state(&self) {
        let app = self.app.read().ok().and_then(|slot| slot.clone());
        if let Some(app) = app {
            let _ = app.emit(EVENT_STATE, self.snapshot());
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
            if runtime.active.is_some()
                || runtime.downloading
                || runtime.removing_model
                || runtime.status == "inserting"
            {
                return Err("Dictation is already recording, transcribing, or downloading".into());
            }
            if let Some(mut text) = runtime.transcript.take() {
                text.zeroize();
            }
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
            if runtime.active.is_some()
                || runtime.downloading
                || runtime.removing_model
                || runtime.status == "inserting"
            {
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

        if let Err(error) = model::verify_installed(&model_path) {
            self.clear_active();
            self.set_status("error", Some(error.clone()));
            return Err(error);
        }
        if stop.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
            self.finish_cancelled();
            return Ok(());
        }

        let service = self.clone();
        std::thread::Builder::new()
            .name("tuck-dictation".into())
            .spawn(move || {
                let result =
                    audio::record_and_transcribe(&service, &app, &model_path, stop, cancel.clone());
                if cancel.load(Ordering::Acquire) {
                    if let Ok(mut text) = result {
                        text.zeroize();
                    }
                    service.finish_cancelled();
                    return;
                }
                match result {
                    Ok(mut text) => {
                        truncate_utf8(&mut text, MAX_TRANSCRIPT_BYTES);
                        service.finish_transcript(text, original_target);
                    }
                    Err(error) if error == "Dictation stopped before microphone capture began" => {
                        service.finish_cancelled();
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

    fn finish_transcript(&self, text: String, original_target: Option<target::ForegroundTarget>) {
        let source = Zeroizing::new(text);
        let text = Zeroizing::new(source.trim().to_owned());
        if text.is_empty() {
            self.clear_active();
            self.set_status("idle", Some("No speech was recognized.".into()));
            return;
        }
        let settings = self.settings();
        let safety = original_target
            .as_ref()
            .ok_or_else(|| "The original foreground window could not be captured.".to_owned())
            .and_then(target::verify_foreground_target)
            .and_then(|_| match original_target.as_ref() {
                Some(target) => target::is_editable_target(target),
                None => Err("The original foreground window could not be captured.".into()),
            });
        let mut inserted = false;
        let mut partial_insertion = false;
        let mut reason = safety.err();
        if !settings.review_before_insert && reason.is_none() {
            if let Some(target) = original_target.as_ref() {
                match target::insert_unicode(target, text.as_str()) {
                    Ok(()) => inserted = true,
                    Err(error) => {
                        partial_insertion = error.may_have_inserted;
                        reason = Some(error.message);
                    }
                }
            }
        }
        let review = settings.review_before_insert || !inserted;
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.active = None;
            runtime.status = if review { "review" } else { "idle" }.into();
            runtime.message = reason.clone();
            // Keep text only during review. Immediate successful insertion drops it.
            runtime.transcript = review.then(|| text.as_str().to_owned());
            runtime.review_target = if review && !partial_insertion {
                original_target
            } else {
                None
            };
        }
        self.emit_state();
        if let Ok(app) = self.app() {
            let _ = app.emit(
                EVENT_TRANSCRIPT,
                DictationTranscriptEvent {
                    text: text.as_str().to_owned(),
                    inserted,
                    requires_review: review,
                    reason,
                },
            );
        }
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
            if let Some(mut text) = runtime.transcript.take() {
                text.zeroize();
            }
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

    /// Ends the recording only if a hold started it; a toggle recording keeps running.
    fn stop_hold(self: &Arc<Self>) {
        let holding = self.runtime.lock().is_ok_and(|runtime| {
            runtime
                .active
                .as_ref()
                .is_some_and(|session| matches!(session.mode, RecordingMode::HoldToTalk))
        });
        if holding {
            let _ = self.stop();
        }
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
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "Dictation service unavailable")?;
            if runtime.status != "review" {
                return Err("There is no transcript awaiting review".into());
            }
            let target = runtime.review_target.clone().ok_or("Automatic insertion is unavailable. Reconcile any existing text manually before copying the transcript.")?;
            let text = runtime
                .transcript
                .take()
                .ok_or("There is no transcript awaiting review")?;
            runtime.status = "inserting".into();
            (Zeroizing::new(text), target)
        };
        self.emit_state();
        let result = target::insert_unicode(&original_target, &text);
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "Dictation service unavailable")?;
        let result = match result {
            Ok(()) => {
                runtime.review_target = None;
                runtime.message = None;
                runtime.status = "idle".into();
                Ok(())
            }
            Err(error) => {
                runtime.transcript = Some(text.to_string());
                runtime.status = "review".into();
                if error.may_have_inserted {
                    runtime.review_target = None;
                }
                runtime.message = Some(error.message.clone());
                Err(error.message)
            }
        };
        drop(runtime);
        self.emit_state();
        result
    }

    fn discard_review(&self) -> Result<(), String> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "Dictation service unavailable")?;
        if runtime.status != "review" {
            return Err("There is no transcript awaiting review".into());
        }
        if let Some(mut text) = runtime.transcript.take() {
            text.zeroize();
        }
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
            if runtime.active.is_some()
                || runtime.downloading
                || runtime.removing_model
                || runtime.status == "review"
                || runtime.status == "inserting"
            {
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
                let mut last_progress_event = std::time::Instant::now()
                    .checked_sub(std::time::Duration::from_secs(1))
                    .unwrap_or_else(std::time::Instant::now);
                let result = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(model::download(&app, &cancel, |received| {
                        if let Ok(mut runtime) = service.runtime.lock() {
                            runtime.downloaded_bytes = received;
                        }
                        if last_progress_event.elapsed() >= std::time::Duration::from_millis(250) {
                            last_progress_event = std::time::Instant::now();
                            service.emit_state();
                        }
                    })),
                    Err(error) => Err(format!("Could not start model download runtime: {error}")),
                };
                let cancelled = cancel.load(Ordering::Acquire);
                let error = result.err().filter(|_| !cancelled);
                if let Ok(mut runtime) = service.runtime.lock() {
                    runtime.downloading = false;
                    runtime.download_cancel = None;
                    runtime.status = if error.is_none() { "idle" } else { "error" }.into();
                    runtime.message = error.clone();
                }
                service.emit_state();
                if let Some(error) = error {
                    if let Ok(app) = service.app() {
                        let _ = app.emit(EVENT_ERROR, error);
                    }
                }
            })
            .map_err(|e| {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.downloading = false;
                    runtime.download_cancel = None;
                    runtime.status = "error".into();
                    runtime.message = Some(format!("Could not start model download: {e}"));
                }
                self.emit_state();
                format!("Could not start model download: {e}")
            })?;
        Ok(())
    }

    fn remove_model(&self) -> Result<(), String> {
        {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "Dictation service unavailable")?;
            if runtime.active.is_some()
                || runtime.downloading
                || runtime.removing_model
                || runtime.status == "review"
                || runtime.status == "inserting"
                || self.model_busy.load(Ordering::Acquire)
            {
                return Err("The model is in use; stop dictation before removing it".into());
            }
            runtime.removing_model = true;
            runtime.status = "removingModel".into();
            runtime.message = None;
        }
        self.emit_state();
        let path = match self.model_path() {
            Ok(path) => path,
            Err(error) => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.removing_model = false;
                    runtime.status = "error".into();
                    runtime.message = Some(error.clone());
                }
                self.emit_state();
                return Err(error);
            }
        };
        let result = if path.exists() {
            std::fs::remove_file(path).map_err(|e| format!("Could not remove the model: {e}"))
        } else {
            Ok(())
        };
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.removing_model = false;
            runtime.status = if result.is_ok() {
                "modelMissing"
            } else {
                "error"
            }
            .into();
            runtime.message = result.as_ref().err().cloned();
        }
        self.emit_state();
        result
    }

    pub fn shutdown(&self) {
        self.is_shutdown.store(true, Ordering::Release);
        dual_shift::stop();
        let _ = self.cancel();
    }

    pub fn is_shutdown(&self) -> bool {
        self.is_shutdown.load(Ordering::Acquire)
    }
}

/// Arms the dual-Shift monitor only while it is the configured hold shortcut.
fn sync_dual_shift(service: &Arc<DictationService>) {
    let enabled = is_dual_shift_shortcut(&service.settings().hold_to_talk_shortcut);
    let service = service.clone();
    dual_shift::set_enabled(enabled, move || {
        Box::new(move |change| {
            if service.is_shutdown() {
                return;
            }
            match change {
                dual_shift::HoldChange::Pressed => {
                    let _ = service.begin(RecordingMode::HoldToTalk);
                }
                dual_shift::HoldChange::Released => service.stop_hold(),
                dual_shift::HoldChange::Unavailable(message) => {
                    service.set_status("error", Some(message));
                }
            }
        })
    });
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
    if let Err(error) = register_shortcuts(&app, service) {
        eprintln!("Dictation shortcut registration warning: {error}");
    }
    sync_dual_shift(service);
    service.emit_state();
    Ok(())
}

pub fn register_shortcuts(app: &AppHandle, service: &Arc<DictationService>) -> Result<(), String> {
    let settings = service.settings();
    let shortcuts = app.global_shortcut();
    let toggle = Shortcut::try_from(settings.toggle_shortcut.as_str())
        .map_err(|e| format!("Invalid toggle shortcut: {e}"))?;
    let hold = if is_dual_shift_shortcut(&settings.hold_to_talk_shortcut) {
        None
    } else {
        Some(
            Shortcut::try_from(settings.hold_to_talk_shortcut.as_str())
                .map_err(|e| format!("Invalid hold-to-talk shortcut: {e}"))?,
        )
    };
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
    if let Some(hold_shortcut) = hold {
        if let Err(error) = shortcuts.register(hold_shortcut) {
            let _ = shortcuts.unregister(toggle);
            for prior in &previous {
                let _ = shortcuts.register(*prior);
            }
            return Err(format!("Could not register hold-to-talk shortcut: {error}"));
        }
        if let Ok(mut registered) = service.registered_shortcuts.lock() {
            *registered = vec![toggle, hold_shortcut];
        }
    } else if let Ok(mut registered) = service.registered_shortcuts.lock() {
        *registered = vec![toggle];
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
        let status = service.snapshot().status.clone();
        if status == "recording" || status == "starting" {
            let _ = service.stop();
        } else if status == "idle" || status == "review" || status == "error" {
            let _ = service.begin(RecordingMode::Toggle);
        }
    } else if hold.as_ref() == Some(shortcut) {
        match event.state {
            ShortcutState::Pressed => {
                let _ = service.begin(RecordingMode::HoldToTalk);
            }
            ShortcutState::Released => service.stop_hold(),
        }
    }
}

fn validate_settings(settings: &DictationSettings) -> Result<(), String> {
    if settings.toggle_shortcut.len() > 64 || settings.hold_to_talk_shortcut.len() > 64 {
        return Err("Shortcut strings must be at most 64 bytes".into());
    }
    let toggle = Shortcut::try_from(settings.toggle_shortcut.as_str())
        .map_err(|e| format!("Invalid toggle shortcut: {e}"))?;
    if is_dual_shift_shortcut(&settings.hold_to_talk_shortcut) {
        return Ok(());
    }
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
    let mut settings: DictationSettings = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Dictation settings are invalid: {e}"))?;
    if settings.hold_to_talk_shortcut == "Ctrl+Alt+Space" {
        settings.hold_to_talk_shortcut = DEFAULT_HOLD.to_owned();
    }
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
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid dictation settings file path")?;
    let tmp = path.with_file_name(format!("{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let _cleanup = TemporaryConfigFile(tmp.clone());
    std::fs::write(&tmp, bytes).map_err(|e| format!("Could not write settings: {e}"))?;
    replace_file(&tmp, &path).map_err(|e| format!("Could not commit settings: {e}"))
}

struct TemporaryConfigFile(PathBuf);

impl Drop for TemporaryConfigFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        },
    };
    let from: Vec<u16> = from
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let to: Vec<u16> = to
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| std::io::Error::other(error.to_string()))
}

#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

fn truncate_utf8(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
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
    sync_dual_shift(&service);
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
    if matches!(service.snapshot().status.as_str(), "recording" | "starting") {
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
    fn dual_shift_shortcut_recognition() {
        assert!(is_dual_shift_shortcut("Shift+Shift"));
        assert!(is_dual_shift_shortcut("shift + shift"));
        assert!(is_dual_shift_shortcut("TwoShifts"));
        assert!(is_dual_shift_shortcut("two shifts"));
        assert!(is_dual_shift_shortcut("LeftShift+RightShift"));
        assert!(is_dual_shift_shortcut("dualshift"));
        assert!(!is_dual_shift_shortcut("Ctrl+Shift+Space"));
        assert!(!is_dual_shift_shortcut("Ctrl+Alt+Space"));
    }
    #[test]
    fn model_removal_is_rejected_during_startup_and_download() {
        let service = DictationService::new();
        service.runtime.lock().unwrap().downloading = true;
        assert!(service.remove_model().unwrap_err().contains("in use"));
        let mut runtime = service.runtime.lock().unwrap();
        runtime.downloading = false;
        runtime.removing_model = true;
        drop(runtime);
        assert!(service.remove_model().unwrap_err().contains("in use"));
        let mut runtime = service.runtime.lock().unwrap();
        runtime.removing_model = false;
        runtime.active = Some(ActiveSession {
            stop: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            mode: RecordingMode::Toggle,
        });
        drop(runtime);
        assert!(service.remove_model().unwrap_err().contains("in use"));
    }

    #[test]
    fn model_removal_preserves_review_until_explicit_discard() {
        let service = DictationService::new();
        {
            let mut runtime = service.runtime.lock().unwrap();
            runtime.status = "review".into();
            runtime.transcript = Some("pending transcript".into());
        }
        assert!(service.remove_model().unwrap_err().contains("in use"));
        {
            let runtime = service.runtime.lock().unwrap();
            assert_eq!(runtime.status, "review");
            assert_eq!(runtime.transcript.as_deref(), Some("pending transcript"));
        }
        service.discard_review().unwrap();
        let runtime = service.runtime.lock().unwrap();
        assert!(runtime.transcript.is_none());
        assert_eq!(runtime.status, "idle");
    }

    #[test]
    fn transcript_byte_cap_does_not_create_an_unbounded_payload() {
        let mut text = "a".repeat(MAX_TRANSCRIPT_BYTES + 10);
        truncate_utf8(&mut text, MAX_TRANSCRIPT_BYTES);
        assert_eq!(text.len(), MAX_TRANSCRIPT_BYTES);
    }

    #[test]
    fn transcript_truncation_preserves_unicode_boundaries() {
        let mut text = "😀".repeat(128);
        truncate_utf8(&mut text, 127);
        assert!(text.len() <= 127);
        assert!(text.is_char_boundary(text.len()));
    }

    #[test]
    fn settings_persistence_replaces_an_existing_file() {
        let dir =
            std::env::temp_dir().join(format!("tuck-dictation-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let temporary = dir.join("settings.json.tmp");
        std::fs::write(&path, b"old").unwrap();
        std::fs::write(
            &temporary,
            serde_json::to_vec(&DictationSettings::default()).unwrap(),
        )
        .unwrap();
        replace_file(&temporary, &path).unwrap();
        let restored: DictationSettings =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored, DictationSettings::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
