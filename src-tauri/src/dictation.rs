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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
const MAX_SETTINGS_BYTES: usize = 64 * 1024;
const SHORTCUT_PAUSE_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

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
#[serde(rename_all = "camelCase", default)]
pub struct DictationSettings {
    pub toggle_shortcut: String,
    pub hold_to_talk_shortcut: String,
    pub review_before_insert: bool,
    pub partial_preview: bool,
    /// Catalog id of the chosen model. Absent in files written before the picker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,
    /// Another app's file used in place. Set only by the backend, from its own scan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_model_path: Option<PathBuf>,
}

impl Default for DictationSettings {
    fn default() -> Self {
        Self {
            toggle_shortcut: DEFAULT_TOGGLE.to_owned(),
            hold_to_talk_shortcut: DEFAULT_HOLD.to_owned(),
            review_before_insert: false,
            partial_preview: true,
            active_model: None,
            active_model_path: None,
        }
    }
}

/// Per-shortcut reasons, shown next to the field they belong to.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutProblems {
    pub toggle: Option<String>,
    pub hold: Option<String>,
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
    pub models: Vec<model::ModelRow>,
    pub active_model_id: Option<String>,
    pub active_model_label: Option<String>,
    /// Why the chosen model is not the one in use, or why a model action failed.
    pub model_notice: Option<String>,
    pub scanning_models: bool,
    pub shortcut_problems: ShortcutProblems,
    /// Set when `dictation.json` could not be used as written.
    pub settings_notice: Option<String>,
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
    downloading_model: Option<&'static str>,
    removing_model: bool,
    download_cancel: Option<Arc<AtomicBool>>,
    checking_models: Vec<String>,
    scanning_models: bool,
    found_models: Vec<model::Found>,
    model_error: Option<String>,
    shortcut_problems: ShortcutProblems,
    settings_notice: Option<String>,
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
    verified: Mutex<model::VerifyCache>,
    locations: Vec<model::Location>,
    shortcuts_paused: AtomicBool,
    pause_generation: AtomicU64,
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
                downloading_model: None,
                removing_model: false,
                download_cancel: None,
                checking_models: Vec::new(),
                scanning_models: false,
                found_models: Vec::new(),
                model_error: None,
                shortcut_problems: ShortcutProblems::default(),
                settings_notice: None,
            }),
            model_busy: AtomicBool::new(false),
            registered_shortcuts: Mutex::new(Vec::new()),
            settings_operation: Mutex::new(()),
            is_shutdown: AtomicBool::new(false),
            verified: Mutex::new(model::VerifyCache::empty()),
            locations: model::system_locations(),
            shortcuts_paused: AtomicBool::new(false),
            pause_generation: AtomicU64::new(0),
        }
    }

    fn models_dir(&self) -> Result<PathBuf, String> {
        let app = self
            .app
            .read()
            .map_err(|_| "Dictation service unavailable")?;
        let app = app.as_ref().ok_or("Dictation has not been initialized")?;
        Ok(app
            .path()
            .app_data_dir()
            .map_err(|e| format!("Could not resolve model directory: {e}"))?
            .join("dictation"))
    }

    fn resolve_model(&self) -> model::Resolution {
        let Ok(dir) = self.models_dir() else {
            return model::Resolution::default();
        };
        let settings = self.settings();
        model::resolve_active(
            settings.active_model.as_deref(),
            settings.active_model_path.as_deref(),
            &dir,
            Path::is_file,
        )
    }

    fn snapshot(&self) -> DictationSnapshot {
        let settings = self.settings.read().map(|s| s.clone()).unwrap_or_default();
        let resolution = self.resolve_model();
        let dir = self.models_dir().ok();
        let locked = self.runtime.lock();
        let (status, downloaded, can_insert, transcript, message, models, notice, scanning) =
            match locked.as_ref() {
                Ok(runtime) => (
                    runtime.status.clone(),
                    runtime.downloaded_bytes,
                    runtime.status == "review" && runtime.review_target.is_some(),
                    runtime.transcript.clone(),
                    runtime.message.clone(),
                    model::build_rows(
                        &model::RowInputs {
                            models_dir: dir.as_deref(),
                            active: resolution.active.as_ref(),
                            found: &runtime.found_models,
                            downloading: runtime.downloading_model,
                            checking: &runtime.checking_models,
                            locations: &self.locations,
                        },
                        Path::is_file,
                    ),
                    runtime
                        .model_error
                        .clone()
                        .or_else(|| resolution.notice.clone()),
                    runtime.scanning_models,
                ),
                Err(_) => (
                    "error".into(),
                    0,
                    false,
                    None,
                    None,
                    Vec::new(),
                    None,
                    false,
                ),
            };
        let (problems, settings_notice) = locked
            .as_ref()
            .map(|runtime| {
                (
                    runtime.shortcut_problems.clone(),
                    runtime.settings_notice.clone(),
                )
            })
            .unwrap_or_default();
        drop(locked);
        let active = resolution.active.as_ref();
        DictationSnapshot {
            status,
            model_installed: active.is_some(),
            model_downloaded_bytes: downloaded,
            can_insert,
            transcript,
            message,
            settings,
            models,
            active_model_id: active.map(|active| active.entry.id.to_owned()),
            active_model_label: active.map(|active| active.entry.label.to_owned()),
            model_notice: notice,
            scanning_models: scanning,
            shortcut_problems: problems,
            settings_notice,
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
        let resolution = self.resolve_model();
        let Some(active) = resolution.active else {
            self.set_status(
                "modelMissing",
                Some(
                    resolution
                        .notice
                        .unwrap_or_else(|| "Choose a speech model in Settings.".into()),
                ),
            );
            return Err("No speech model is ready".into());
        };
        {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "Dictation service unavailable")?;
            if runtime.active.is_some() || runtime.removing_model || runtime.status == "inserting" {
                return Err("Dictation is already recording or transcribing".into());
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
            if runtime.active.is_some() || runtime.removing_model || runtime.status == "inserting" {
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
                // Off the caller's thread: hashing a model not yet verified can take seconds.
                if let Err(error) =
                    model::verify(&active.path, active.entry, &service.verified, false)
                {
                    service.finish_error(error);
                    return;
                }
                if stop.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
                    service.finish_cancelled();
                    return;
                }
                let result = audio::record_and_transcribe(
                    &service,
                    &app,
                    &active.path,
                    stop,
                    cancel.clone(),
                );
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
        Err("There is no recording or transcription to cancel".into())
    }

    /// Separate from `cancel`, so cancelling a download never ends a recording.
    fn cancel_download(&self) -> Result<(), String> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "Dictation service unavailable")?;
        let cancel = runtime
            .download_cancel
            .as_ref()
            .ok_or("There is no model download to cancel")?;
        cancel.store(true, Ordering::Release);
        Ok(())
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

    fn download_model(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let entry = model::entry(id)
            .filter(|entry| entry.offered)
            .ok_or("That model is not available to download")?;
        let dir = self.models_dir()?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "Dictation service unavailable")?;
            if runtime.downloading_model.is_some() {
                return Err("Another model is downloading. Wait for it or cancel it.".into());
            }
            if runtime.removing_model {
                return Err("A model is being removed. Try again in a moment.".into());
            }
            runtime.downloading_model = Some(entry.id);
            runtime.download_cancel = Some(cancel.clone());
            runtime.downloaded_bytes = 0;
            runtime.model_error = None;
        }
        self.emit_state();
        let active_before = self.resolve_model().active.map(|active| active.entry.id);
        let selection_before = self.settings().active_model;
        let service = self.clone();
        let spawned = std::thread::Builder::new()
            .name("tuck-model-download".into())
            .spawn(move || {
                let mut last_event = std::time::Instant::now();
                let result = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(model::download(
                        &dir,
                        entry,
                        &service.verified,
                        &cancel,
                        |received| {
                            if let Ok(mut runtime) = service.runtime.lock() {
                                runtime.downloaded_bytes = received;
                            }
                            if last_event.elapsed() >= std::time::Duration::from_millis(250) {
                                last_event = std::time::Instant::now();
                                service.emit_state();
                            }
                        },
                    )),
                    Err(error) => Err(format!("Could not start model download runtime: {error}")),
                };
                let error = result.as_ref().err().cloned();
                let reported = error.clone().filter(|_| !cancel.load(Ordering::Acquire));
                if let Ok(mut runtime) = service.runtime.lock() {
                    runtime.downloading_model = None;
                    runtime.download_cancel = None;
                    runtime.downloaded_bytes = 0;
                    runtime.model_error = reported.clone();
                }
                // Only when the owner did not choose a model during the download.
                if error.is_none() && service.settings().active_model == selection_before {
                    let selection = selection_after_download(
                        active_before,
                        selection_before.is_some(),
                        entry.id,
                    );
                    if let Some(id) = selection {
                        if let Err(error) = service.commit_selection(Some(id.to_owned()), None) {
                            service.set_model_error(error);
                        }
                    }
                }
                service.clear_missing_status();
                service.emit_state();
                if let Some(error) = reported {
                    if let Ok(app) = service.app() {
                        let _ = app.emit(EVENT_ERROR, error);
                    }
                }
            });
        if let Err(error) = spawned {
            let message = format!("Could not start model download: {error}");
            if let Ok(mut runtime) = self.runtime.lock() {
                runtime.downloading_model = None;
                runtime.download_cancel = None;
                runtime.model_error = Some(message.clone());
            }
            self.emit_state();
            return Err(message);
        }
        Ok(())
    }

    fn set_model_error(&self, error: String) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.model_error = Some(error);
        }
    }

    /// A model just became usable, so stop saying one is missing.
    fn clear_missing_status(&self) {
        if self.resolve_model().active.is_none() {
            return;
        }
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.status == "modelMissing" {
                runtime.status = "idle".into();
                runtime.message = None;
            }
        }
    }

    /// Stores the selection in `dictation.json`, undoing it if the file cannot be written.
    fn commit_selection(&self, id: Option<String>, path: Option<PathBuf>) -> Result<(), String> {
        let app = self.app()?;
        let _operation = self
            .settings_operation
            .lock()
            .map_err(|_| "Dictation settings unavailable")?;
        let mut next = self.settings();
        next.active_model = id;
        next.active_model_path = path;
        persist_settings(&app, &next)?;
        *self
            .settings
            .write()
            .map_err(|_| "Dictation settings unavailable")? = next;
        Ok(())
    }

    /// Verifies off the UI thread, then selects. Other apps' files are used where they are.
    fn use_model(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let entry = model::entry(id).ok_or("That model is not in Tuck's catalog")?;
        let own = self.models_dir()?.join(entry.id);
        let external = {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "Dictation service unavailable")?;
            if runtime.active.is_some() {
                return Err("Stop dictation before changing the model".into());
            }
            let external = runtime
                .found_models
                .iter()
                .find(|found| found.id == entry.id)
                .map(|found| found.path.clone());
            if !own.is_file() && external.is_none() {
                return Err(format!(
                    "{} is not on this computer. Download it first.",
                    entry.label
                ));
            }
            runtime.checking_models.push(entry.id.to_owned());
            runtime.model_error = None;
            external
        };
        self.emit_state();
        let service = self.clone();
        let spawned = std::thread::Builder::new()
            .name("tuck-model-check".into())
            .spawn(move || {
                let own_installed = own.is_file();
                let path = if own_installed {
                    own
                } else {
                    external.clone().unwrap_or(own)
                };
                let result = model::verify(&path, entry, &service.verified, true).and_then(|_| {
                    service.commit_selection(
                        Some(entry.id.to_owned()),
                        (!own_installed).then_some(path),
                    )
                });
                if let Ok(mut runtime) = service.runtime.lock() {
                    runtime.checking_models.retain(|item| item != entry.id);
                    runtime.model_error = result.err();
                }
                service.clear_missing_status();
                service.emit_state();
            });
        if let Err(error) = spawned {
            if let Ok(mut runtime) = self.runtime.lock() {
                runtime.checking_models.retain(|item| item != entry.id);
            }
            self.emit_state();
            return Err(format!("Could not check the model: {error}"));
        }
        Ok(())
    }

    /// Deletes a model Tuck downloaded. For another app's file it only stops using it.
    fn remove_model(&self, id: &str) -> Result<(), String> {
        let entry = model::entry(id).ok_or("That model is not in Tuck's catalog")?;
        {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "Dictation service unavailable")?;
            if runtime.active.is_some()
                || runtime.downloading_model == Some(entry.id)
                || runtime.removing_model
                || runtime.status == "review"
                || runtime.status == "inserting"
                || self.model_busy.load(Ordering::Acquire)
            {
                return Err("The model is in use; stop dictation before removing it".into());
            }
            runtime.removing_model = true;
            runtime.model_error = None;
        }
        let result = self.remove_model_files(entry);
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.removing_model = false;
            runtime.model_error = result.as_ref().err().cloned();
        }
        self.emit_state();
        result
    }

    fn remove_model_files(&self, entry: &'static model::CatalogEntry) -> Result<(), String> {
        let settings = self.settings();
        let selected = settings.active_model.as_deref() == Some(entry.id);
        if selected && settings.active_model_path.is_some() {
            return self.commit_selection(None, None);
        }
        // The path comes from the catalog id inside Tuck's own folder, so this can only
        // ever delete a file Tuck put there.
        let path = self.models_dir()?.join(entry.id);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("Could not remove the model: {e}"))?;
        }
        model::forget(&path, &self.verified);
        if selected {
            self.commit_selection(None, None)?;
        }
        Ok(())
    }

    /// On demand only. Lists by name and size, then hashes only what could be a match,
    /// on a low-priority thread, skipping models Tuck already has.
    fn scan_models(self: &Arc<Self>) -> Result<(), String> {
        let dir = self.models_dir()?;
        {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "Dictation service unavailable")?;
            if runtime.scanning_models {
                return Ok(());
            }
            runtime.scanning_models = true;
        }
        self.emit_state();
        let service = self.clone();
        let spawned = std::thread::Builder::new()
            .name("tuck-model-scan".into())
            .spawn(move || {
                let mut found: Vec<model::Found> = Vec::new();
                for candidate in model::find_candidates(&service.locations) {
                    let id = candidate.entry.id;
                    if service.is_shutdown()
                        || dir.join(id).is_file()
                        || found.iter().any(|item| item.id == id)
                    {
                        continue;
                    }
                    if let Ok(mut runtime) = service.runtime.lock() {
                        runtime.checking_models.push(id.to_owned());
                    }
                    service.emit_state();
                    let matched =
                        model::verify(&candidate.path, candidate.entry, &service.verified, true)
                            .is_ok();
                    if matched {
                        found.push(model::Found {
                            id,
                            path: candidate.path,
                            app: candidate.app,
                        });
                    }
                    if let Ok(mut runtime) = service.runtime.lock() {
                        runtime.checking_models.retain(|item| item != id);
                        runtime.found_models = found.clone();
                    }
                }
                if let Ok(mut runtime) = service.runtime.lock() {
                    runtime.found_models = found;
                    runtime.scanning_models = false;
                }
                service.emit_state();
            });
        if let Err(error) = spawned {
            if let Ok(mut runtime) = self.runtime.lock() {
                runtime.scanning_models = false;
            }
            return Err(format!("Could not look for models: {error}"));
        }
        Ok(())
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

/// What to store after a download so it never changes the model in use. With no stored
/// selection the active model is only "first installed", which a new file could displace,
/// so it is pinned. Returns `None` when the stored selection should stay as it is.
fn selection_after_download(
    active_before: Option<&'static str>,
    has_selection: bool,
    downloaded: &'static str,
) -> Option<&'static str> {
    match active_before {
        None => Some(downloaded),
        Some(active) if !has_selection => Some(active),
        Some(_) => None,
    }
}

/// Arms the dual-Shift monitor only while it is the configured hold shortcut.
fn sync_dual_shift(service: &Arc<DictationService>) {
    let enabled = is_dual_shift_shortcut(&service.settings().hold_to_talk_shortcut)
        && !service.shortcuts_paused.load(Ordering::Acquire);
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
    if let Ok(dir) = service.models_dir() {
        if let Ok(mut verified) = service.verified.lock() {
            *verified = model::VerifyCache::load(&dir);
        }
    }
    // A settings file that cannot be used must not take dictation down with it, but the
    // owner has to be told, or the settings page looks like it ignored their choices.
    let (settings, notice) = load_settings(&app);
    if let Ok(mut current) = service.settings.write() {
        *current = settings.clone();
    }
    if let Ok(mut runtime) = service.runtime.lock() {
        runtime.settings_notice = notice;
    }
    // Failures here are recorded per shortcut and shown beside the field.
    if let Err(error) = register_shortcuts(&app, service, &settings) {
        eprintln!("Dictation shortcut registration warning: {error}");
    }
    sync_dual_shift(service);
    service.emit_state();
    Ok(())
}

fn shortcut_taken_message(shortcut: &str, error: &str) -> String {
    if error.contains("already registered") {
        format!("{shortcut} is already used by another app. Record a different shortcut.")
    } else {
        format!("Windows would not register {shortcut}. Record a different shortcut.")
    }
}

fn invalid_shortcut_message(shortcut: &str) -> String {
    format!("\"{shortcut}\" is not a shortcut Tuck can use. Press a modifier and a key, like Ctrl+Alt+Space.")
}

/// A shortcut the owner just changed must work, or the save is refused. One that did not
/// change is recorded as a problem instead: if another app holds Ctrl+Shift+Space, that
/// must not stop the owner flipping "Review before inserting" or fixing the other shortcut.
fn registration_outcome(changed: bool, failure: Option<String>) -> Result<Option<String>, String> {
    match failure {
        Some(reason) if changed => Err(reason),
        other => Ok(other),
    }
}

/// `prior` is what the settings were before this change. At startup it is the settings
/// themselves, so nothing counts as changed and nothing can fail hard.
pub fn register_shortcuts(
    app: &AppHandle,
    service: &Arc<DictationService>,
    prior: &DictationSettings,
) -> Result<(), String> {
    let settings = service.settings();
    let shortcuts = app.global_shortcut();
    let toggle = Shortcut::try_from(settings.toggle_shortcut.as_str())
        .map_err(|_| invalid_shortcut_message(&settings.toggle_shortcut))?;
    let hold = if is_dual_shift_shortcut(&settings.hold_to_talk_shortcut) {
        None
    } else {
        Some(
            Shortcut::try_from(settings.hold_to_talk_shortcut.as_str())
                .map_err(|_| invalid_shortcut_message(&settings.hold_to_talk_shortcut))?,
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
    let mut registered = Vec::new();
    let mut attempt =
        |shortcut: Shortcut, text: &str, changed: bool| match shortcuts.register(shortcut) {
            Ok(()) => {
                registered.push(shortcut);
                Ok(None)
            }
            Err(error) => registration_outcome(
                changed,
                Some(shortcut_taken_message(text, &error.to_string())),
            ),
        };
    let toggle_result = attempt(
        toggle,
        &settings.toggle_shortcut,
        settings.toggle_shortcut != prior.toggle_shortcut,
    );
    let hold_result = match hold {
        Some(hold) if toggle_result.is_ok() => attempt(
            hold,
            &settings.hold_to_talk_shortcut,
            settings.hold_to_talk_shortcut != prior.hold_to_talk_shortcut,
        ),
        _ => Ok(None),
    };
    let problems =
        toggle_result.and_then(|toggle| hold_result.map(|hold| ShortcutProblems { toggle, hold }));
    match problems {
        Ok(problems) => {
            if let Ok(mut slot) = service.registered_shortcuts.lock() {
                *slot = registered;
            }
            if let Ok(mut runtime) = service.runtime.lock() {
                runtime.shortcut_problems = problems;
            }
            Ok(())
        }
        Err(error) => {
            for shortcut in &registered {
                let _ = shortcuts.unregister(*shortcut);
            }
            for shortcut in &previous {
                let _ = shortcuts.register(*shortcut);
            }
            Err(error)
        }
    }
}

/// While the recorder listens, Tuck's own shortcuts must not fire or swallow the keys.
/// A timer ends the pause in case the settings window goes away mid-recording.
fn pause_shortcuts(app: &AppHandle, service: &Arc<DictationService>, paused: bool) {
    let generation = service.pause_generation.fetch_add(1, Ordering::AcqRel) + 1;
    service.shortcuts_paused.store(paused, Ordering::Release);
    if paused {
        let shortcuts = app.global_shortcut();
        if let Ok(mut registered) = service.registered_shortcuts.lock() {
            for shortcut in registered.drain(..) {
                let _ = shortcuts.unregister(shortcut);
            }
        }
        let app = app.clone();
        let service = service.clone();
        let _ = std::thread::Builder::new()
            .name("tuck-shortcut-pause".into())
            .spawn(move || {
                std::thread::sleep(SHORTCUT_PAUSE_LIMIT);
                if service.pause_generation.load(Ordering::Acquire) == generation {
                    pause_shortcuts(&app, &service, false);
                    service.emit_state();
                }
            });
    } else {
        let current = service.settings();
        if let Err(error) = register_shortcuts(app, service, &current) {
            eprintln!("Dictation shortcut registration warning: {error}");
        }
    }
    sync_dual_shift(service);
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
        } else if ["idle", "review", "error", "modelMissing"].contains(&status.as_str()) {
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
        return Err("Shortcuts must be at most 64 characters".into());
    }
    if is_dual_shift_shortcut(&settings.toggle_shortcut) {
        return Err("Both Shift keys only work for Hold to talk.".into());
    }
    let toggle = Shortcut::try_from(settings.toggle_shortcut.as_str())
        .map_err(|_| invalid_shortcut_message(&settings.toggle_shortcut))?;
    if is_dual_shift_shortcut(&settings.hold_to_talk_shortcut) {
        return Ok(());
    }
    let hold = Shortcut::try_from(settings.hold_to_talk_shortcut.as_str())
        .map_err(|_| invalid_shortcut_message(&settings.hold_to_talk_shortcut))?;
    if toggle == hold {
        return Err("Start and stop and Hold to talk must be different shortcuts.".into());
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

/// Keeps every usable value. A bad shortcut resets only that shortcut, so one unreadable
/// value (a file from a newer or older build) does not throw away the rest.
fn parse_settings(bytes: &[u8]) -> (DictationSettings, Option<String>) {
    let mut settings: DictationSettings = match serde_json::from_slice(bytes) {
        Ok(settings) => settings,
        Err(error) => {
            return (
                DictationSettings::default(),
                Some(format!(
                    "The saved dictation settings could not be read ({error}). Using the defaults until you change something."
                )),
            )
        }
    };
    if settings.hold_to_talk_shortcut == "Ctrl+Alt+Space" {
        settings.hold_to_talk_shortcut = DEFAULT_HOLD.to_owned();
    }
    let mut resets = Vec::new();
    if is_dual_shift_shortcut(&settings.toggle_shortcut)
        || Shortcut::try_from(settings.toggle_shortcut.as_str()).is_err()
    {
        resets.push(format!(
            "Start and stop was \"{}\", now {DEFAULT_TOGGLE}",
            settings.toggle_shortcut
        ));
        settings.toggle_shortcut = DEFAULT_TOGGLE.to_owned();
    }
    if validate_settings(&settings).is_err() {
        resets.push(format!(
            "Hold to talk was \"{}\", now both Shift keys",
            settings.hold_to_talk_shortcut
        ));
        settings.hold_to_talk_shortcut = DEFAULT_HOLD.to_owned();
    }
    let notice = (!resets.is_empty())
        .then(|| format!("A saved shortcut could not be used. {}.", resets.join(". ")));
    (settings, notice)
}

fn load_settings(app: &AppHandle) -> (DictationSettings, Option<String>) {
    let failed = |reason: String| (DictationSettings::default(), Some(reason));
    let path = match settings_path(app) {
        Ok(path) => path,
        Err(error) => return failed(error),
    };
    match std::fs::read(path) {
        // Room for a selection with a long path to another app's folder.
        Ok(bytes) if bytes.len() <= MAX_SETTINGS_BYTES => parse_settings(&bytes),
        Ok(_) => failed("The saved dictation settings are too large. Using the defaults.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (DictationSettings::default(), None)
        }
        Err(error) => failed(format!("Could not read the dictation settings: {error}")),
    }
}

fn persist_settings(app: &AppHandle, settings: &DictationSettings) -> Result<(), String> {
    let path = settings_path(app)?;
    let parent = path.parent().ok_or("Invalid settings directory")?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("Could not create settings directory: {e}"))?;
    let bytes =
        serde_json::to_vec(settings).map_err(|e| format!("Could not serialize settings: {e}"))?;
    if bytes.len() > MAX_SETTINGS_BYTES {
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
    // The model is chosen only through its own commands, which use backend-verified paths.
    // A page can never point the recorder at a file by sending settings.
    let settings = DictationSettings {
        active_model: previous.active_model.clone(),
        active_model_path: previous.active_model_path.clone(),
        ..settings
    };
    let shortcuts_changed = settings.toggle_shortcut != previous.toggle_shortcut
        || settings.hold_to_talk_shortcut != previous.hold_to_talk_shortcut;
    let had_problems = service
        .runtime
        .lock()
        .is_ok_and(|runtime| runtime.shortcut_problems != ShortcutProblems::default());
    let resumed = service.shortcuts_paused.swap(false, Ordering::AcqRel);
    if resumed {
        service.pause_generation.fetch_add(1, Ordering::AcqRel);
    }
    *service
        .settings
        .write()
        .map_err(|_| "Dictation settings unavailable")? = settings.clone();
    let registered = if shortcuts_changed || had_problems || resumed {
        register_shortcuts(&app, &service, &previous)
    } else {
        Ok(())
    };
    if let Err(error) = registered.and_then(|_| persist_settings(&app, &settings)) {
        *service
            .settings
            .write()
            .map_err(|_| "Dictation settings unavailable")? = previous.clone();
        let _ = register_shortcuts(&app, &service, &previous);
        sync_dual_shift(&service);
        return Err(error);
    }
    if let Ok(mut runtime) = service.runtime.lock() {
        runtime.settings_notice = None;
    }
    sync_dual_shift(&service);
    service.emit_state();
    Ok(service.snapshot())
}

#[tauri::command]
pub fn dictation_shortcuts_pause(
    app: AppHandle,
    service: State<'_, Arc<DictationService>>,
    paused: bool,
) {
    pause_shortcuts(&app, &service, paused);
}

#[tauri::command]
pub fn dictation_model_download(
    service: State<'_, Arc<DictationService>>,
    id: String,
) -> Result<(), String> {
    service.download_model(&id)
}

#[tauri::command]
pub fn dictation_model_remove(
    service: State<'_, Arc<DictationService>>,
    id: String,
) -> Result<(), String> {
    service.remove_model(&id)
}

#[tauri::command]
pub fn dictation_model_use(
    service: State<'_, Arc<DictationService>>,
    id: String,
) -> Result<(), String> {
    service.use_model(&id)
}

#[tauri::command]
pub fn dictation_models_scan(service: State<'_, Arc<DictationService>>) -> Result<(), String> {
    service.scan_models()
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
    service.cancel_download()
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
        service.runtime.lock().unwrap().downloading_model = Some("ggml-tiny.bin");
        assert!(service
            .remove_model("ggml-tiny.bin")
            .unwrap_err()
            .contains("in use"));
        let mut runtime = service.runtime.lock().unwrap();
        runtime.downloading_model = None;
        runtime.removing_model = true;
        drop(runtime);
        assert!(service
            .remove_model("ggml-tiny.bin")
            .unwrap_err()
            .contains("in use"));
        let mut runtime = service.runtime.lock().unwrap();
        runtime.removing_model = false;
        runtime.active = Some(ActiveSession {
            stop: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            mode: RecordingMode::Toggle,
        });
        drop(runtime);
        assert!(service
            .remove_model("ggml-tiny.bin")
            .unwrap_err()
            .contains("in use"));
    }

    #[test]
    fn model_commands_only_accept_catalog_ids() {
        let service = Arc::new(DictationService::new());
        assert!(service.remove_model("../../secrets.bin").is_err());
        assert!(service.use_model("../../secrets.bin").is_err());
        assert!(service.download_model("../../secrets.bin").is_err());
        assert!(
            service.download_model("ggml-base.bin").is_err(),
            "detect-only entries are not offered for download"
        );
    }

    #[test]
    fn a_download_never_changes_the_model_in_use() {
        assert_eq!(
            selection_after_download(None, false, "ggml-small-q5_1.bin"),
            Some("ggml-small-q5_1.bin"),
            "nothing active: the new model fills the gap"
        );
        assert_eq!(
            selection_after_download(None, true, "ggml-small-q5_1.bin"),
            Some("ggml-small-q5_1.bin"),
            "the selected file is gone and nothing else is installed"
        );
        assert_eq!(
            selection_after_download(Some("ggml-small-q5_1.bin"), false, "ggml-base-q5_1.bin"),
            Some("ggml-small-q5_1.bin"),
            "active only by fallback: pinned, or Base would displace Small"
        );
        assert_eq!(
            selection_after_download(Some("ggml-tiny.bin"), true, "ggml-base-q5_1.bin"),
            None,
            "an explicit selection stays as it is"
        );
    }

    #[test]
    fn cancelling_a_download_does_not_touch_a_recording() {
        let service = DictationService::new();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel_download = Arc::new(AtomicBool::new(false));
        {
            let mut runtime = service.runtime.lock().unwrap();
            runtime.active = Some(ActiveSession {
                stop: stop.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                mode: RecordingMode::Toggle,
            });
            runtime.download_cancel = Some(cancel_download.clone());
        }
        service.cancel_download().unwrap();
        assert!(cancel_download.load(Ordering::Acquire));
        assert!(!stop.load(Ordering::Acquire));
    }

    #[test]
    fn old_settings_files_load_without_the_model_fields() {
        let (settings, notice) = parse_settings(
            br#"{"toggleShortcut":"Ctrl+Shift+Space","holdToTalkShortcut":"Shift+Shift","reviewBeforeInsert":true,"partialPreview":true}"#,
        );
        assert_eq!(notice, None);
        assert_eq!(settings.active_model, None);
        assert_eq!(settings.active_model_path, None);
        assert!(settings.review_before_insert);
        assert_eq!(settings.hold_to_talk_shortcut, "Shift+Shift");
    }

    #[test]
    fn selection_round_trips_and_old_defaults_stay_out_of_the_file() {
        let plain = serde_json::to_string(&DictationSettings::default()).unwrap();
        assert!(!plain.contains("activeModel"));
        let settings = DictationSettings {
            active_model: Some("ggml-base.bin".into()),
            active_model_path: Some(PathBuf::from("C:/x/ggml-base.bin")),
            ..DictationSettings::default()
        };
        let bytes = serde_json::to_vec(&settings).unwrap();
        assert_eq!(parse_settings(&bytes), (settings, None));
    }

    #[test]
    fn legacy_hold_shortcut_still_migrates() {
        let (settings, notice) = parse_settings(
            br#"{"toggleShortcut":"Ctrl+Shift+Space","holdToTalkShortcut":"Ctrl+Alt+Space","reviewBeforeInsert":false,"partialPreview":true}"#,
        );
        assert_eq!(settings.hold_to_talk_shortcut, DEFAULT_HOLD);
        assert_eq!(notice, None, "this migration is silent, as it always was");
    }

    #[test]
    fn a_bad_saved_shortcut_keeps_the_other_settings_and_says_so() {
        let (settings, notice) = parse_settings(
            br#"{"toggleShortcut":"Ctrl+Shift+Space","holdToTalkShortcut":"not a key","reviewBeforeInsert":true,"partialPreview":false}"#,
        );
        assert_eq!(settings.hold_to_talk_shortcut, DEFAULT_HOLD);
        assert!(settings.review_before_insert, "unrelated values survive");
        assert!(!settings.partial_preview);
        assert!(notice.unwrap().contains("not a key"));

        let (toggle, notice) = parse_settings(
            br#"{"toggleShortcut":"Shift+Shift","holdToTalkShortcut":"Ctrl+Alt+F9"}"#,
        );
        assert_eq!(toggle.toggle_shortcut, DEFAULT_TOGGLE);
        assert_eq!(toggle.hold_to_talk_shortcut, "Ctrl+Alt+F9");
        assert!(notice.is_some());
    }

    #[test]
    fn an_unreadable_settings_file_reports_why() {
        let (settings, notice) = parse_settings(b"{ not json");
        assert_eq!(settings, DictationSettings::default());
        assert!(notice.unwrap().contains("could not be read"));
    }

    /// This is why a build without dual-shift support rejected the owner's file: the
    /// shortcut parser alone cannot read Shift+Shift.
    #[test]
    fn the_shortcut_parser_alone_rejects_shift_shift() {
        assert!(Shortcut::try_from("Shift+Shift").is_err());
    }

    #[test]
    fn validation_refusals_explain_themselves() {
        let dual_toggle = DictationSettings {
            toggle_shortcut: "Shift+Shift".into(),
            ..DictationSettings::default()
        };
        assert!(validate_settings(&dual_toggle)
            .unwrap_err()
            .contains("Hold to talk"));
        let bad_hold = DictationSettings {
            hold_to_talk_shortcut: "nonsense+".into(),
            ..DictationSettings::default()
        };
        let reason = validate_settings(&bad_hold).unwrap_err();
        assert!(reason.contains("nonsense+"));
        assert!(!reason.contains("report this"), "no parser internals");
    }

    /// The reproduced bug: another process holds the toggle shortcut, so before this fix
    /// every save failed, even flipping an unrelated switch.
    #[test]
    fn a_taken_shortcut_only_blocks_the_save_that_changes_it() {
        let taken = "Ctrl+Shift+Space is already used".to_owned();
        assert_eq!(
            registration_outcome(false, Some(taken.clone())),
            Ok(Some(taken.clone())),
            "unchanged: recorded as a problem, save continues"
        );
        assert_eq!(
            registration_outcome(true, Some(taken.clone())),
            Err(taken),
            "changed: refused with the reason"
        );
        assert_eq!(registration_outcome(true, None), Ok(None));
    }

    #[test]
    fn taken_shortcut_message_is_plain_language() {
        let message = shortcut_taken_message(
            "Ctrl+Shift+Space",
            "HotKey already registered: HotKey { mods: Modifiers(CONTROL | SHIFT) }",
        );
        assert!(message.contains("Ctrl+Shift+Space is already used by another app"));
        assert!(!message.contains("Modifiers"));
    }

    /// Everything the recorder in `src/shortcut-recorder.ts` can produce must parse.
    #[test]
    fn recorded_shortcut_forms_parse() {
        for shortcut in [
            "Ctrl+Shift+Space",
            "Ctrl+Alt+A",
            "Super+Shift+1",
            "Alt+F9",
            "F13",
            "Ctrl+Alt+ArrowUp",
            "Ctrl+Alt+Enter",
            "Ctrl+Alt+Backquote",
            "Ctrl+Alt+Numpad0",
        ] {
            assert!(Shortcut::try_from(shortcut).is_ok(), "{shortcut}");
        }
    }

    #[test]
    fn model_removal_preserves_review_until_explicit_discard() {
        let service = DictationService::new();
        {
            let mut runtime = service.runtime.lock().unwrap();
            runtime.status = "review".into();
            runtime.transcript = Some("pending transcript".into());
        }
        assert!(service
            .remove_model("ggml-tiny.bin")
            .unwrap_err()
            .contains("in use"));
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
