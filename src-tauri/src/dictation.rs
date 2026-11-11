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
const EVENT_TRANSCRIPT: &str = "dictation_transcript";
const EVENT_ERROR: &str = "dictation_error";
const DEFAULT_TOGGLE: &str = "Ctrl+Shift+Space";
const DEFAULT_HOLD: &str = "RightAlt";
const MAX_TRANSCRIPT_BYTES: usize = 64 * 1024;
const MAX_SETTINGS_BYTES: usize = 64 * 1024;
/// `settingsVersion` in `dictation.json`. Version 1 migrated `Ctrl+Alt+Space`, version 2
/// migrated `Shift+Shift`; after that, either value in the file is the owner's own choice.
const SETTINGS_VERSION: u32 = 2;
const SHORTCUT_PAUSE_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

pub fn is_right_alt_shortcut(shortcut: &str) -> bool {
    let s = shortcut.trim().to_lowercase().replace(' ', "");
    matches!(s.as_str(), "rightalt" | "ralt" | "altright")
}

/// The modifier-only hold the native monitor watches, if the shortcut is one.
fn special_hold(shortcut: &str) -> Option<dual_shift::HoldKey> {
    if is_right_alt_shortcut(shortcut) {
        Some(dual_shift::HoldKey::RightAlt)
    } else if is_dual_shift_shortcut(shortcut) {
        Some(dual_shift::HoldKey::BothShifts)
    } else {
        None
    }
}

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
    pub keep_microphone_ready: bool,
    pub stopped_models: Vec<String>,
    #[serde(default)]
    pub settings_version: u32,
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
            keep_microphone_ready: true,
            stopped_models: Vec::new(),
            settings_version: SETTINGS_VERSION,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DictationSnapshot {
    pub backend: String,
    pub session_id: u64,
    pub revision: u64,
    pub audio_level: f32,
    /// Provisional words heard so far, until the final text replaces them.
    pub live_text: Option<String>,
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
        if let Some(live) = self.live_text.as_mut() {
            live.zeroize();
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
    session_id: u64,
    revision: u64,
    audio_level: f32,
    /// Provisional words from the live stream while recording. Never pasted or kept.
    live_text: Option<String>,
    recording_target: Option<target::ForegroundTarget>,
    focus_changed: bool,
    selection_generation: u64,
    repair_attempt: Option<(String, u64)>,
    status: String,
    message: Option<String>,
    transcript: Option<String>,
    review_target: Option<target::ForegroundTarget>,
    insertion_uncertain: bool,
    active: Option<ActiveSession>,
    downloaded_bytes: u64,
    downloading_model: Option<&'static str>,
    removing_model: bool,
    download_cancel: Option<Arc<AtomicBool>>,
    checking_models: Vec<String>,
    user_selection_check: Option<u64>,
    scanning_models: bool,
    found_models: Vec<model::Found>,
    model_error: Option<String>,
    shortcut_problems: ShortcutProblems,
    dual_shift_problem: Option<String>,
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
    storage_root: Option<PathBuf>,
    #[cfg(test)]
    acquisition_fixture: Option<AcquisitionFixture>,
    #[cfg(test)]
    scan_barrier: Option<Arc<dyn Fn() + Send + Sync>>,
    settings: RwLock<DictationSettings>,
    runtime: Mutex<Runtime>,
    insertion_gate: Mutex<()>,
    focus_observer_busy: Arc<AtomicBool>,
    model_busy: AtomicBool,
    engine_cache: Arc<audio::EngineCache>,
    microphone: Arc<audio::ReadyMicrophone>,
    capture_times: Arc<Mutex<(std::time::Instant, Option<std::time::Instant>)>>,
    registered_shortcuts: Mutex<Vec<Shortcut>>,
    settings_operation: Mutex<()>,
    is_shutdown: AtomicBool,
    verified: Mutex<model::VerifyCache>,
    locations: Vec<model::Location>,
    shortcuts_paused: AtomicBool,
    pause_generation: AtomicU64,
}

#[cfg(test)]
type FixtureNetwork = dyn Fn(&Path, &AtomicBool) -> Result<(), String> + Send + Sync;

#[cfg(test)]
struct AcquisitionFixture {
    entry: &'static model::CatalogEntry,
    network: Arc<FixtureNetwork>,
}

impl Default for DictationService {
    fn default() -> Self {
        Self::new()
    }
}

struct FocusObserverLease(Arc<AtomicBool>);
impl Drop for FocusObserverLease {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl DictationService {
    pub fn new() -> Self {
        Self {
            app: RwLock::new(None),
            storage_root: None,
            #[cfg(test)]
            acquisition_fixture: None,
            #[cfg(test)]
            scan_barrier: None,
            settings: RwLock::new(DictationSettings::default()),
            runtime: Mutex::new(Runtime {
                session_id: 0,
                revision: 0,
                audio_level: 0.0,
                live_text: None,
                recording_target: None,
                focus_changed: false,
                selection_generation: 0,
                repair_attempt: None,
                status: "idle".into(),
                message: None,
                transcript: None,
                review_target: None,
                insertion_uncertain: false,
                active: None,
                downloaded_bytes: 0,
                downloading_model: None,
                removing_model: false,
                download_cancel: None,
                checking_models: Vec::new(),
                user_selection_check: None,
                scanning_models: false,
                found_models: Vec::new(),
                model_error: None,
                shortcut_problems: ShortcutProblems::default(),
                dual_shift_problem: None,
                settings_notice: None,
            }),
            insertion_gate: Mutex::new(()),
            focus_observer_busy: Arc::new(AtomicBool::new(false)),
            model_busy: AtomicBool::new(false),
            engine_cache: Arc::default(),
            microphone: Arc::default(),
            capture_times: Arc::new(Mutex::new((std::time::Instant::now(), None))),
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
        if let Some(root) = &self.storage_root {
            return Ok(root.join("dictation"));
        }
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
        if let (Some(id), Some(path)) = (
            settings.active_model.as_deref(),
            settings.active_model_path.as_deref(),
        ) {
            if !model::local_model_exists(path) {
                return model::Resolution {
                    active: None,
                    notice: Some(format!(
                        "{} is missing. Open Settings to download it again.",
                        model::entry(id).map_or(id, |e| e.label)
                    )),
                };
            }
        }
        model::resolve_active(
            settings.active_model.as_deref(),
            settings.active_model_path.as_deref(),
            &dir,
            |path| model::path_is_known(path, &self.locations),
            |path| {
                !settings
                    .stopped_models
                    .iter()
                    .any(|id| path.file_name().is_some_and(|n| n == id.as_str()))
                    && model::local_model_exists(path)
            },
        )
    }

    // Looks for another verified copy of a missing selection, once per selection generation
    // and process. It never downloads: that takes a click, because a model can be hundreds of MB.
    fn repair_missing(self: &Arc<Self>) -> Result<(), String> {
        let settings = self.settings();
        let Some(id) = settings.active_model.as_deref() else {
            return Ok(());
        };
        let Some(path) = settings.active_model_path.as_deref() else {
            return Ok(());
        };
        if model::local_model_exists(path)
            || !model::path_is_known(path, &self.locations)
            || settings.stopped_models.iter().any(|stopped| stopped == id)
        {
            return Ok(());
        }
        {
            let mut runtime = self.runtime.lock().map_err(|_| "Dictation unavailable")?;
            let key = (id.to_owned(), runtime.selection_generation);
            if runtime.repair_attempt.as_ref() == Some(&key)
                || runtime.downloading_model.is_some()
                || runtime.removing_model
                || runtime.active.is_some()
                || runtime.status == "review"
                || runtime.status == "inserting"
                || self.is_shutdown()
            {
                return Ok(());
            }
            runtime.repair_attempt = Some(key);
        }
        self.start_model_download(id, true)
    }

    pub(crate) fn snapshot(&self) -> DictationSnapshot {
        let settings = self.settings.read().map(|s| s.clone()).unwrap_or_default();
        let resolution = self.resolve_model();
        let dir = self.models_dir().ok();
        let locked = self.runtime.lock();
        let (status, downloaded, can_insert, transcript, message, models, notice, scanning) =
            match locked.as_ref() {
                Ok(runtime) => (
                    runtime.status.clone(),
                    runtime.downloaded_bytes,
                    runtime.status == "review" && !runtime.insertion_uncertain,
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
                            available_memory: model::available_memory(),
                        },
                        model::local_model_exists,
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
                let mut problems = runtime.shortcut_problems.clone();
                if special_hold(&settings.hold_to_talk_shortcut).is_some() {
                    problems.hold.clone_from(&runtime.dual_shift_problem);
                }
                (problems, runtime.settings_notice.clone())
            })
            .unwrap_or_default();
        let audio_level = locked
            .as_ref()
            .map(|r| {
                if r.status == "recording" {
                    r.audio_level
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0);
        let (session_id, revision) = locked
            .as_ref()
            .map(|r| (r.session_id, r.revision))
            .unwrap_or_default();
        let live_text = locked.as_ref().ok().and_then(|r| {
            ["recording", "processing", "transcribing", "inserting"]
                .contains(&r.status.as_str())
                .then(|| r.live_text.clone())
                .flatten()
        });
        drop(locked);
        let active = resolution.active.as_ref();
        DictationSnapshot {
            backend: self.engine_cache.backend(),
            session_id,
            revision,
            audio_level,
            live_text,
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

    /// Why an update can't install right now. Installing stops every service, so anything
    /// in progress would be lost. Resting states, including a missing model, don't block:
    /// nothing could be finished or cancelled to get out of them.
    pub(crate) fn update_blocker(&self) -> Option<&'static str> {
        // A poisoned lock still holds readable state, and an update may be the cure
        let runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if runtime.downloading_model.is_some() {
            return Some("Wait for the model download or cancel it before installing the update.");
        }
        if runtime.removing_model {
            return Some("Wait for the model change to finish before installing the update.");
        }
        if self.model_busy.load(Ordering::Acquire) {
            return Some("Wait for dictation to finish before installing the update.");
        }
        let resting = matches!(runtime.status.as_str(), "idle" | "error" | "modelMissing");
        (runtime.active.is_some() || !resting)
            .then_some("Finish or cancel dictation before installing the update.")
    }

    fn emit_state(&self) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.revision += 1;
        }
        let app = self.app.read().ok().and_then(|slot| slot.clone());
        if let Some(app) = app {
            let _ = app.emit(EVENT_STATE, self.snapshot());
        }
    }

    fn session_status(&self, status: &str) {
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.status == "cancelling"
                || (status == "recording" && runtime.status != "starting")
            {
                return;
            }
            runtime.status = status.into();
        }
        self.emit_state();
    }

    fn set_status(&self, status: &str, message: Option<String>) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.status = status.to_owned();
            runtime.message = message;
        }
        self.emit_state();
    }

    /// Publish versioned meter snapshots so a late
    /// initial response or event cannot resurrect an earlier recording's text.
    fn mark_focus_changed(&self, session_id: u64) {
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.session_id == session_id && runtime.active.is_some() {
                runtime.focus_changed = true;
            }
        }
    }

    /// UIA providers may hang. At most one observer can exist, and it never
    /// owns a runtime lock or blocks capture/cancellation/session shutdown.
    fn start_focus_observer(
        self: &Arc<Self>,
        session_id: u64,
        stop: Arc<AtomicBool>,
        cancel: Arc<AtomicBool>,
        mut observe: impl FnMut() -> bool + Send + 'static,
    ) -> Option<std::thread::JoinHandle<()>> {
        if self.focus_observer_busy.swap(true, Ordering::AcqRel) {
            // An old provider is stuck: conservatively copy this session's final text.
            self.mark_focus_changed(session_id);
            return None;
        }
        let lease = FocusObserverLease(self.focus_observer_busy.clone());
        let service = Arc::downgrade(self);
        let result = std::thread::Builder::new()
            .name("tuck-focus-observer".into())
            .spawn(move || {
                let _lease = lease;
                while !stop.load(Ordering::Acquire) && !cancel.load(Ordering::Acquire) {
                    let unchanged = observe(); // No service lock or strong reference here.
                    if stop.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
                        break;
                    }
                    let Some(service) = service.upgrade() else {
                        break;
                    };
                    let current = service
                        .runtime
                        .lock()
                        .is_ok_and(|r| r.session_id == session_id && r.active.is_some());
                    if !current {
                        break;
                    }
                    if !unchanged {
                        service.mark_focus_changed(session_id);
                        break;
                    }
                    drop(service);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            });
        match result {
            Ok(handle) => Some(handle),
            Err(_) => {
                self.mark_focus_changed(session_id);
                None
            }
        }
    }

    /// Shows the stream's words so far. Only a recording that is still running may change
    /// them, so a late update can never reappear after the final text or a cancel.
    fn publish_live_text(&self, text: &str) {
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.status != "recording"
                || !runtime.active.as_ref().is_some_and(|a| {
                    !a.cancel.load(Ordering::Acquire) && !a.stop.load(Ordering::Acquire)
                })
            {
                return;
            }
            let text = text.trim();
            if runtime.live_text.as_deref().unwrap_or("") == text {
                return;
            }
            if let Some(mut old) = runtime.live_text.take() {
                old.zeroize();
            }
            runtime.live_text = (!text.is_empty()).then(|| text.to_owned());
            runtime.revision += 1;
        } else {
            return;
        }
        self.emit_state();
    }

    fn publish_audio_level(&self, level: f32) {
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.status != "recording"
                || !runtime.active.as_ref().is_some_and(|a| {
                    !a.cancel.load(Ordering::Acquire) && !a.stop.load(Ordering::Acquire)
                })
            {
                return;
            }
            runtime.audio_level = level;
            runtime.revision += 1;
        } else {
            return;
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

    fn keep_warm(self: &Arc<Self>) {
        if self.is_shutdown() || self.engine_cache.warming.swap(true, Ordering::AcqRel) {
            return;
        }
        let service = self.clone();
        std::thread::spawn(move || {
            // Serialize with model selection/removal, and never overlap active inference.
            let _operation = service.settings_operation.lock().unwrap();
            let can_warm = {
                let runtime = service.runtime.lock().unwrap();
                runtime.active.is_none()
                    && !runtime.removing_model
                    && !service.model_busy.swap(true, Ordering::AcqRel)
            };
            let active = can_warm.then(|| service.resolve_model().active).flatten();
            drop(_operation); // Capture may open while weights are still loading.
            if can_warm {
                if let Some(active) = active {
                    service.engine_cache.start_reaper();
                    if model::verify(&active.path, active.entry, &service.verified, false).is_ok() {
                        let _ = service.engine_cache.warm(&active.path);
                    }
                }
                service.model_busy.store(false, Ordering::Release);
            }
            service.engine_cache.warming.store(false, Ordering::Release);
            if can_warm {
                service.emit_state();
            }
        });
    }

    fn begin(self: &Arc<Self>, mode: RecordingMode) -> Result<(), String> {
        let pressed = std::time::Instant::now();
        self.app()?;
        if self.runtime.lock().is_ok_and(|r| r.status == "inserting") {
            return Err("Dictation is still inserting the last transcript".into());
        }
        // A transcript waiting in the island never blocks the next dictation.
        self.release_waiting_transcript();
        self.repair_missing()?;
        let operation = self
            .settings_operation
            .lock()
            .map_err(|_| "Settings unavailable")?;
        let resolution = self.resolve_model();
        let Some(active) = resolution.active else {
            self.report_no_model(
                resolution
                    .notice
                    .unwrap_or_else(|| "Choose a speech model in Settings.".into()),
            );
            return Err("No speech model is ready".into());
        };
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let original_target = target::capture_foreground();
        self.claim_session(stop.clone(), cancel.clone(), mode)?;
        self.capture_times.lock().unwrap().0 = pressed;
        {
            let mut r = self.runtime.lock().unwrap();
            r.recording_target = original_target.clone();
            r.focus_changed = false;
            r.audio_level = 0.0;
        }
        drop(operation);
        if let Some(target) = original_target.clone() {
            let session_id = self.runtime.lock().unwrap().session_id;
            self.start_focus_observer(session_id, stop.clone(), cancel.clone(), move || {
                target::verify_foreground_target(&target).is_ok()
            });
        }
        self.emit_state();

        let service = self.clone();
        std::thread::Builder::new()
            .name("tuck-dictation".into())
            .spawn(move || {
                // The microphone opens first. Verifying and loading happen beside it.
                let result = audio::record_and_transcribe(&service, &active, stop, cancel.clone());
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
                        service.finish_transcript(text, original_target, &cancel);
                    }
                    Err(error) if error == "Dictation stopped before microphone capture began" => {
                        service.finish_cancelled();
                    }
                    Err(error) => service.finish_error(error),
                }
            })
            .map_err(|error| {
                let message = format!("Could not start dictation worker: {error}");
                self.clear_active();
                self.set_status("error", Some(message.clone()));
                message
            })?;
        Ok(())
    }

    /// A press with no model must not disturb a transcript waiting for review.
    fn report_no_model(&self, notice: String) {
        let in_review = self
            .runtime
            .lock()
            .is_ok_and(|runtime| runtime.status == "review");
        if in_review {
            self.set_model_error(notice);
            self.emit_state();
        } else {
            self.set_status("modelMissing", Some(notice));
        }
    }

    /// The point where a session really starts, and so the only place a pending review
    /// transcript is given up.
    fn claim_session(
        &self,
        stop: Arc<AtomicBool>,
        cancel: Arc<AtomicBool>,
        mode: RecordingMode,
    ) -> Result<(), String> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "Dictation service unavailable")?;
        if runtime.status == "cancelling" {
            return Err("Dictation is cancelling. Wait for the speech engine to finish before recording again.".into());
        }
        if runtime.active.is_some() || runtime.removing_model || runtime.status == "inserting" {
            return Err("Dictation is already recording or transcribing".into());
        }
        if let Some(mut text) = runtime.transcript.take() {
            text.zeroize();
        }
        if let Some(mut text) = runtime.live_text.take() {
            text.zeroize();
        }
        runtime.review_target = None;
        runtime.insertion_uncertain = false;
        runtime.message = None;
        *self.capture_times.lock().unwrap() = (std::time::Instant::now(), None);
        runtime.active = Some(ActiveSession { stop, cancel, mode });
        runtime.session_id += 1;

        runtime.status = "starting".into();
        Ok(())
    }

    fn finish_transcript(
        &self,
        text: String,
        original_target: Option<target::ForegroundTarget>,
        cancel: &AtomicBool,
    ) {
        let source = Zeroizing::new(text);
        if cancel.load(Ordering::Acquire) {
            self.finish_cancelled();
            return;
        }
        let text = Zeroizing::new(source.trim().to_owned());
        if self.session_cancelled() {
            self.finish_cancelled();
            return;
        }
        if text.is_empty() {
            self.clear_active();
            self.set_status("idle", Some("No speech was recognized.".into()));
            return;
        }
        let settings = self.settings();
        {
            let mut runtime = self.runtime.lock().unwrap();
            if cancel.load(Ordering::Acquire) {
                drop(runtime);
                self.finish_cancelled();
                return;
            }
            runtime.status = "inserting".into();
        }
        self.emit_state();
        let mut inserted = false;
        let mut partial_insertion = false;
        let mut reason = None;
        let mut delivered_copy = false;
        if self.session_cancelled() {
            self.finish_cancelled();
            return;
        }
        if !settings.review_before_insert {
            match self.paste_transcript(text.as_str(), cancel) {
                Ok(pasted) => {
                    inserted = pasted;
                    delivered_copy = !pasted;
                    reason = Some(if pasted { "Inserted" } else { "Copied" }.into());
                }
                Err(error) => {
                    partial_insertion = error.may_have_inserted;
                    reason = Some(error.message);
                    // Never strand the words in a waiting transcript that blocks the next
                    // dictation: leave them on the clipboard instead.
                    if !partial_insertion
                        && !cancel.load(Ordering::Acquire)
                        && self.copy_transcript(text.as_str()).is_ok()
                    {
                        delivered_copy = true;
                        reason = Some("Copied".into());
                    }
                }
            }
        }
        let copied = delivered_copy;
        let review = !copied && (settings.review_before_insert || !inserted);
        if let Ok(mut runtime) = self.runtime.lock() {
            if cancel.load(Ordering::Acquire) {
                drop(runtime);
                self.finish_cancelled();
                return;
            }
            runtime.active = None;
            runtime.recording_target = None;
            if let Some(mut live) = runtime.live_text.take() {
                live.zeroize();
            }
            runtime.audio_level = 0.0;
            runtime.status = if review { "review" } else { "idle" }.into();
            runtime.message = reason.clone();
            // Keep text only during review. Immediate successful insertion drops it.
            runtime.transcript = review.then(|| text.as_str().to_owned());
            runtime.insertion_uncertain = partial_insertion;
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

    fn session_cancelled(&self) -> bool {
        self.runtime.lock().map_or(true, |runtime| {
            runtime
                .active
                .as_ref()
                .is_none_or(|active| active.cancel.load(Ordering::Acquire))
        })
    }

    fn finish_cancelled(&self) {
        self.clear_active();
        self.set_status("idle", None);
    }

    fn clear_active(&self) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.active = None;
            runtime.recording_target = None;

            runtime.audio_level = 0.0;
            runtime.focus_changed = false;
            if let Some(mut text) = runtime.transcript.take() {
                text.zeroize();
            }
            if let Some(mut text) = runtime.live_text.take() {
                text.zeroize();
            }
        }
    }

    fn stop(self: &Arc<Self>) -> Result<(), String> {
        self.stop_at(std::time::Instant::now())
    }

    fn stop_at(self: &Arc<Self>, released: std::time::Instant) -> Result<(), String> {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "Dictation service unavailable")?;
        let active = runtime
            .active
            .as_ref()
            .ok_or("Dictation is not recording")?;
        self.capture_times.lock().unwrap().1.get_or_insert(released);
        if active.stop.swap(true, Ordering::AcqRel) {
            return Err("Dictation is already stopping".into());
        }
        Ok(())
    }

    /// Ends the recording only if a hold started it; a toggle recording keeps running.
    fn stop_hold(self: &Arc<Self>) {
        let released = std::time::Instant::now();
        let holding = self.runtime.lock().is_ok_and(|runtime| {
            runtime
                .active
                .as_ref()
                .is_some_and(|session| matches!(session.mode, RecordingMode::HoldToTalk))
        });
        if holding {
            let _ = self.stop_at(released);
        }
    }

    fn cancel(&self) -> Result<(), String> {
        let _gate = self
            .insertion_gate
            .lock()
            .map_err(|_| "Insertion unavailable")?;
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

    /// Moves a transcript waiting for review onto the clipboard, so starting a new
    /// dictation loses nothing, and leaves the review state.
    fn release_waiting_transcript(&self) {
        let waiting = {
            let Ok(mut runtime) = self.runtime.lock() else {
                return;
            };
            if runtime.status != "review" {
                return;
            }
            let uncertain = runtime.insertion_uncertain;
            let text = runtime.transcript.take();
            runtime.review_target = None;
            runtime.insertion_uncertain = false;
            runtime.message = None;
            runtime.status = "idle".into();
            runtime.revision += 1;
            // Text that may already be partly in the app is not copied over the clipboard.
            text.filter(|_| !uncertain)
        };
        if let Some(mut text) = waiting {
            let _ = self.copy_transcript(&text);
            text.zeroize();
        }
    }

    fn copy_transcript(&self, text: &str) -> Result<(), String> {
        target::copy(text, &self.insertion_gate, self.main_window_handle()?)
    }

    fn main_window_handle(&self) -> Result<isize, String> {
        #[cfg(windows)]
        {
            Ok(self
                .app()?
                .get_webview_window("main")
                .ok_or_else(|| "Main window unavailable".to_owned())?
                .hwnd()
                .map_err(|e| e.to_string())?
                .0 as isize)
        }
        #[cfg(not(windows))]
        Ok(0)
    }

    fn paste_transcript(
        &self,
        text: &str,
        cancel: &AtomicBool,
    ) -> Result<bool, target::InsertionFailure> {
        #[cfg(windows)]
        let owner = self
            .app()?
            .get_webview_window("main")
            .ok_or_else(|| "Main window unavailable".to_owned())?
            .hwnd()
            .map_err(|e| e.to_string())?
            .0 as isize;
        #[cfg(not(windows))]
        let owner = 0;
        target::paste(text, cancel, &self.insertion_gate, owner)
    }

    fn finish_review(self: &Arc<Self>) -> Result<(), String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let text = {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "Dictation service unavailable")?;
            if runtime.status != "review" {
                return Err("There is no transcript awaiting review".into());
            }
            if runtime.insertion_uncertain {
                return Err("Some text may already have been inserted. Reconcile it manually before copying.".into());
            }
            let text = runtime
                .transcript
                .take()
                .ok_or("There is no transcript awaiting review")?;
            runtime.active = Some(ActiveSession {
                stop: Arc::new(AtomicBool::new(true)),
                cancel: cancel.clone(),
                mode: RecordingMode::Toggle,
            });
            runtime.status = "inserting".into();
            Zeroizing::new(text)
        };
        self.emit_state();
        let result = deliver_review(self.paste_transcript(&text, &cancel));
        let copied = matches!(result, Ok(ReviewDelivery::Copied));
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "Dictation service unavailable")?;
        if cancel.load(Ordering::Acquire) {
            drop(runtime);
            self.finish_cancelled();
            return Err("Dictation cancelled".into());
        }
        runtime.active = None;
        let result = match result {
            Ok(ReviewDelivery::Typed) => {
                runtime.review_target = None;
                runtime.message = Some("Inserted".into());
                runtime.status = "idle".into();
                Ok(())
            }
            Ok(ReviewDelivery::Copied) => {
                runtime.review_target = None;
                runtime.message = Some("Copied".into());
                runtime.status = "idle".into();
                Ok(())
            }
            Err(error) => {
                runtime.transcript = Some(text.to_string());
                runtime.status = "review".into();
                if error.may_have_inserted {
                    runtime.review_target = None;
                    runtime.insertion_uncertain = true;
                }
                runtime.message = Some(error.message.clone());
                Err(error.message)
            }
        };
        drop(runtime);
        self.emit_state();
        if copied {
            if let Ok(app) = self.app() {
                let _ = app.emit(
                    EVENT_TRANSCRIPT,
                    DictationTranscriptEvent {
                        text: String::new(),
                        inserted: false,
                        requires_review: false,
                        reason: Some("Copied".into()),
                    },
                );
            }
        }
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
        if let Some(mut text) = runtime.live_text.take() {
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
        self.start_model_download(id, false)
    }

    fn start_model_download(self: &Arc<Self>, id: &str, restoring: bool) -> Result<(), String> {
        let entry = model::entry(id)
            .filter(|entry| entry.offered || restoring)
            .ok_or("That model is not available to download")?;
        #[cfg(test)]
        let entry = self
            .acquisition_fixture
            .as_ref()
            .map_or(entry, |fixture| fixture.entry);
        let dir = self.models_dir()?;
        let cancel = Arc::new(AtomicBool::new(false));
        let generation = {
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
            if self.is_shutdown() {
                return Err("Tuck is shutting down".into());
            }
            if restoring {
                let desired = self.settings();
                if desired.active_model.as_deref() != Some(entry.id)
                    || desired
                        .active_model_path
                        .as_deref()
                        .is_none_or(model::local_model_exists)
                    || runtime.repair_attempt.as_ref()
                        != Some(&(entry.id.to_owned(), runtime.selection_generation))
                {
                    return Ok(());
                }
            }

            runtime.settings_notice = Some(format!(
                "{} {}\u{2026}",
                if restoring {
                    "Looking for"
                } else {
                    "Checking existing copies of"
                },
                entry.label
            ));
            runtime.downloading_model = Some(entry.id);
            runtime.download_cancel = Some(cancel.clone());
            runtime.downloaded_bytes = 0;
            runtime.model_error = None;
            runtime.selection_generation
        };
        self.emit_state();
        let active_before = self.resolve_model().active.map(|active| active.entry.id);
        let selection_before = self.settings().active_model;
        let dir_for_pin = self.models_dir()?;
        let service = self.clone();
        let spawned = std::thread::Builder::new()
            .name("tuck-model-download".into())
            .spawn(move || {
                let mut last_event = std::time::Instant::now();
                #[cfg(test)]
                let catalog = service
                    .acquisition_fixture
                    .as_ref()
                    .map_or(model::CATALOG, |f| std::slice::from_ref(f.entry));
                #[cfg(not(test))]
                let existing = model::verified_existing(
                    &dir,
                    entry,
                    &service.locations,
                    &service.verified,
                    &cancel,
                );
                #[cfg(test)]
                let existing = model::verified_existing_with_catalog(
                    &dir,
                    entry,
                    &service.locations,
                    &service.verified,
                    &cancel,
                    catalog,
                );
                let reused_path = existing.as_ref().ok().and_then(|p| p.clone());
                let needs_download = restoring && matches!(existing, Ok(None));
                let result = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime
                        .block_on(model::acquire(existing, || async {
                            if needs_download {
                                Ok(())
                            } else if service.is_shutdown()
                                || cancel.load(Ordering::Acquire)
                                || service
                                    .runtime
                                    .lock()
                                    .map_or(true, |r| r.selection_generation != generation)
                            {
                                Err("Model operation cancelled".into())
                            } else {
                                #[cfg(test)]
                                if let Some(fixture) = &service.acquisition_fixture {
                                    return (fixture.network)(&dir, &cancel);
                                }
                                model::download(
                                    &dir,
                                    entry,
                                    &service.verified,
                                    &cancel,
                                    |received| {
                                        if let Ok(mut runtime) = service.runtime.lock() {
                                            runtime.downloaded_bytes = received;
                                        }
                                        if last_event.elapsed()
                                            >= std::time::Duration::from_millis(250)
                                        {
                                            last_event = std::time::Instant::now();
                                            service.emit_state();
                                        }
                                    },
                                )
                                .await
                            }
                        }))
                        .map(|_| ()),
                    Err(error) => Err(format!("Could not start acquisition runtime: {error}")),
                };
                let mut error = result.as_ref().err().cloned();
                let mut activated = false;
                let mut superseded = false;
                let mut cleanup_generation = generation;
                // Only when the owner did not choose a model during the download.
                if error.is_none()
                    && !needs_download
                    && !cancel.load(Ordering::Acquire)
                    && !service.is_shutdown()
                {
                    // A fallback model removed during the download is not worth pinning.
                    let still_there = selection_before.is_some()
                        || active_before
                            .is_some_and(|id| model::local_model_exists(&dir_for_pin.join(id)));
                    let selection = if restoring {
                        Some(entry.id)
                    } else {
                        selection_after_download(
                            active_before.filter(|_| still_there),
                            selection_before.is_some(),
                            entry.id,
                        )
                    };
                    if let Some(id) = selection {
                        match service.commit_selection_inner(
                            Some(id.to_owned()),
                            reused_path
                                .clone()
                                .filter(|path| id == entry.id && path != &dir.join(entry.id)),
                            Some(generation),
                            None,
                            true,
                        ) {
                            Ok(committed_generation) => {
                                activated = committed_generation.is_some() && id == entry.id;
                                superseded = committed_generation.is_none();
                                if let Some(committed_generation) = committed_generation {
                                    cleanup_generation = committed_generation;
                                }
                            }
                            Err(activation_error) => error = Some(activation_error),
                        }
                    }
                }
                let reported = error.clone().filter(|_| !cancel.load(Ordering::Acquire));
                if let Ok(mut runtime) = service.runtime.lock() {
                    runtime.downloading_model = None;
                    runtime.download_cancel = None;
                    runtime.downloaded_bytes = 0;
                    if runtime.selection_generation == cleanup_generation {
                        runtime.model_error = reported.clone();
                        runtime.settings_notice = Some(if cancel.load(Ordering::Acquire) {
                            "Model download cancelled; use Download to retry".into()
                        } else if error.is_some() {
                            "Model download failed; use Download to retry".into()
                        } else if needs_download {
                            format!("{} is missing; use Download to get it again", entry.label)
                        } else if superseded {
                            format!(
                                "{} is available; the model selection was left unchanged",
                                entry.label
                            )
                        } else if let Some(path) = reused_path {
                            format!(
                                "{} existing {} from {}",
                                if activated { "Using" } else { "Verified" },
                                entry.label,
                                model::app_for(&path, &service.locations).unwrap_or("Tuck")
                            )
                        } else {
                            format!("{} is ready", entry.label)
                        });
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

    fn set_dual_shift_problem(&self, problem: Option<String>) {
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.dual_shift_problem == problem {
                return;
            }
            runtime.dual_shift_problem = problem;
        }
        self.emit_state();
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
        self.commit_checked_selection(id, path, None, None)
    }
    fn commit_checked_selection(
        &self,
        id: Option<String>,
        path: Option<PathBuf>,
        generation: Option<u64>,
        stop: Option<&str>,
    ) -> Result<(), String> {
        self.commit_selection_inner(id, path, generation, stop, false)
            .map(|_| ())
    }
    fn commit_selection_inner(
        &self,
        id: Option<String>,
        path: Option<PathBuf>,
        generation: Option<u64>,
        stop: Option<&str>,
        automatic: bool,
    ) -> Result<Option<u64>, String> {
        let settings_path = match &self.storage_root {
            Some(root) => root.join("dictation.json"),
            None => settings_path(&self.app()?)?,
        };
        let _operation = self
            .settings_operation
            .lock()
            .map_err(|_| "Dictation settings unavailable")?;
        let mut runtime = self.runtime.lock().map_err(|_| "Dictation unavailable")?;
        let committed = apply_selection_commit(&mut runtime, generation, automatic, || {
            let mut next = self.settings();
            if let Some(id) = id.as_ref() {
                next.stopped_models.retain(|item| item != id);
            }
            if let Some(id) = stop {
                if !next.stopped_models.iter().any(|item| item == id) {
                    next.stopped_models.push(id.into());
                }
            }
            next.active_model = id;
            next.active_model_path = path;
            persist_settings_at(&settings_path, &next)?;
            *self
                .settings
                .write()
                .map_err(|_| "Dictation settings unavailable")? = next;
            Ok(())
        })?;
        // Capture ownership while still holding the lock: a later user choice
        // must prevent this worker from replacing that choice's notice.
        Ok(committed.then_some(runtime.selection_generation))
    }

    /// Verifies off the UI thread, then selects. Other apps' files are used where they are.
    fn use_model(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let entry = model::entry(id).ok_or("That model is not in Tuck's catalog")?;
        let own = self.models_dir()?.join(entry.id);
        let (external, generation) = {
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
            if !model::local_model_exists(&own) && external.is_none() {
                return Err(format!(
                    "{} is not on this computer. Download it first.",
                    entry.label
                ));
            }
            runtime.selection_generation = runtime.selection_generation.wrapping_add(1);
            if let Some(cancel) = &runtime.download_cancel {
                cancel.store(true, Ordering::Release);
            }
            let current_generation = runtime.selection_generation;
            if let Some((_, attempted_generation)) = &mut runtime.repair_attempt {
                *attempted_generation = current_generation;
            }
            runtime.checking_models.push(entry.id.to_owned());
            runtime.user_selection_check = Some(current_generation);
            runtime.model_error = None;
            (external, runtime.selection_generation)
        };
        self.emit_state();
        let service = self.clone();
        let spawned = std::thread::Builder::new()
            .name("tuck-model-check".into())
            .spawn(move || {
                let own_installed = model::local_model_exists(&own);
                let path = if own_installed {
                    own
                } else {
                    external.clone().unwrap_or(own)
                };
                let result = model::verify(&path, entry, &service.verified, true).and_then(|_| {
                    service.commit_checked_selection(
                        Some(entry.id.to_owned()),
                        (!own_installed).then_some(path),
                        Some(generation),
                        None,
                    )
                });
                if let Ok(mut runtime) = service.runtime.lock() {
                    runtime.checking_models.retain(|item| item != entry.id);
                    if runtime.user_selection_check == Some(generation) {
                        runtime.user_selection_check = None;
                    }
                    if runtime.selection_generation == generation || result.is_ok() {
                        runtime.model_error = result.err();
                    }
                }
                service.clear_missing_status();
                service.emit_state();
            });
        if let Err(error) = spawned {
            if let Ok(mut runtime) = self.runtime.lock() {
                runtime.checking_models.retain(|item| item != entry.id);
                if runtime.user_selection_check == Some(generation) {
                    runtime.user_selection_check = None;
                }
            }
            self.emit_state();
            return Err(format!("Could not check the model: {error}"));
        }
        Ok(())
    }

    /// Deletes a model Tuck downloaded. For another app's file it only stops using it.
    fn remove_model(&self, id: &str) -> Result<(), String> {
        let entry = model::entry(id).ok_or("That model is not in Tuck's catalog")?;
        let settings = self.settings();
        let borrowed =
            settings.active_model.as_deref() == Some(id) && settings.active_model_path.is_some();
        {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| "Dictation service unavailable")?;
            if runtime.active.is_some()
                || (runtime.downloading_model == Some(entry.id) && !borrowed)
                || runtime.removing_model
                || runtime.status == "review"
                || runtime.status == "inserting"
                || self.model_busy.load(Ordering::Acquire)
            {
                return Err("The model is in use; stop dictation before removing it".into());
            }
            runtime.selection_generation = runtime.selection_generation.wrapping_add(1);
            runtime.removing_model = true;
            runtime.model_error = None;
        }
        self.engine_cache.clear();
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
            return self.commit_checked_selection(None, None, None, Some(entry.id));
        }
        let path = self.models_dir()?.join(entry.id);
        if !model::is_owned(&path, entry) {
            return self.commit_checked_selection(None, None, None, Some(entry.id));
        }
        model::remove_owned(&path, entry)?;
        model::forget(&path, &self.verified);
        if selected {
            self.commit_selection(None, None)?;
        }
        Ok(())
    }

    /// On demand only. Lists by name and size, then hashes only what could be a match,
    /// on a low-priority thread, skipping models Tuck already has.
    fn scan_models(self: &Arc<Self>) -> Result<(), String> {
        self.repair_missing()?;
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
                #[cfg(test)]
                let candidates = model::find_candidates_with_catalog(
                    &service.locations,
                    service
                        .acquisition_fixture
                        .as_ref()
                        .map_or(model::CATALOG, |f| std::slice::from_ref(f.entry)),
                );
                #[cfg(not(test))]
                let candidates = model::find_candidates(&service.locations);
                for candidate in candidates {
                    let id = candidate.entry.id;
                    if service.is_shutdown()
                        || model::local_model_exists(&dir.join(id))
                        || found.iter().any(|item| item.id == id)
                    {
                        continue;
                    }
                    if let Ok(mut runtime) = service.runtime.lock() {
                        runtime.checking_models.push(id.to_owned());
                    }
                    service.emit_state();
                    #[cfg(test)]
                    if let Some(barrier) = &service.scan_barrier {
                        barrier();
                    }
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
        self.microphone.shutdown();
        self.is_shutdown.store(true, Ordering::Release);
        target::shutdown_restore_workers();
        dual_shift::stop();
        let _ = self.cancel();
        // Give a download a moment to delete its part file. If the process wins the race,
        // the sweep at next start removes it.
        if self.cancel_download().is_ok() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(750);
            while std::time::Instant::now() < deadline
                && self
                    .runtime
                    .lock()
                    .is_ok_and(|runtime| runtime.downloading_model.is_some())
            {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    }

    pub fn is_shutdown(&self) -> bool {
        self.is_shutdown.load(Ordering::Acquire)
    }
}

/// Called with the runtime and settings-operation locks held. A download is not a
/// user selection: it must neither displace nor invalidate one requested while it ran.
fn apply_selection_commit(
    runtime: &mut Runtime,
    generation: Option<u64>,
    automatic: bool,
    commit: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    if automatic
        && (generation != Some(runtime.selection_generation)
            || runtime.user_selection_check.is_some()
            || runtime
                .download_cancel
                .as_ref()
                .is_some_and(|cancel| cancel.load(Ordering::Acquire)))
    {
        return Ok(false);
    }
    validate_selection_commit(runtime, generation)?;
    commit()?;
    if !automatic {
        if let Some(cancel) = &runtime.download_cancel {
            cancel.store(true, Ordering::Release);
        }
    }
    runtime.selection_generation = runtime.selection_generation.wrapping_add(1);
    Ok(true)
}

fn validate_selection_commit(runtime: &Runtime, generation: Option<u64>) -> Result<(), String> {
    if runtime.active.is_some() || runtime.status == "inserting" {
        return Err("Stop dictation before changing the model".into());
    }
    if generation.is_some() && runtime.removing_model {
        return Err("Model removal is in progress".into());
    }
    if generation.is_some_and(|expected| expected != runtime.selection_generation) {
        return Err("A newer model selection superseded this check".into());
    }
    Ok(())
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

/// Arms the modifier-only hold monitor only while one is the configured hold shortcut.
fn sync_dual_shift(service: &Arc<DictationService>) {
    let key = special_hold(&service.settings().hold_to_talk_shortcut);
    if let Some(key) = key {
        dual_shift::set_key(key);
    }
    let enabled = key.is_some() && !service.shortcuts_paused.load(Ordering::Acquire);
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
                    service.set_dual_shift_problem(Some(message));
                }
                dual_shift::HoldChange::Available => service.set_dual_shift_problem(None),
            }
        })
    });
}

/// Initialize settings and shortcuts, and restore only a missing saved external model.
/// Never accesses the microphone or loads a speech model.
pub fn start(app: AppHandle, service: &Arc<DictationService>) -> Result<(), String> {
    if let Ok(mut slot) = service.app.write() {
        *slot = Some(app.clone());
    }
    if let Ok(dir) = service.models_dir() {
        model::sweep_partial_downloads(&dir);
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
    service.repair_missing()?;
    audio::start_ready_microphone(service);
    service.emit_state();
    Ok(())
}

/// A shortcut the owner just chose should simply be replaced. One that worked before may be
/// held only for now, so it can also be retried.
fn shortcut_taken_message(shortcut: &str, error: &str, changed: bool) -> String {
    let what = if error.contains("already registered") {
        format!("{shortcut} is in use by another app")
    } else {
        format!("Windows would not register {shortcut}")
    };
    if changed {
        format!("{what}. Record a different shortcut.")
    } else {
        format!("{what} right now. Try again once it is free, or record a different shortcut.")
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
    let hold = if special_hold(&settings.hold_to_talk_shortcut).is_some() {
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
                Some(shortcut_taken_message(text, &error.to_string(), changed)),
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
    if special_hold(&settings.toggle_shortcut).is_some() {
        return Err("Right Alt and both Shift keys only work for Hold to talk.".into());
    }
    let toggle = Shortcut::try_from(settings.toggle_shortcut.as_str())
        .map_err(|_| invalid_shortcut_message(&settings.toggle_shortcut))?;
    if special_hold(&settings.hold_to_talk_shortcut).is_some() {
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

/// Reads one field. A wrong type costs only that field; null counts as absent.
fn settings_field<T: serde::de::DeserializeOwned>(
    map: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
    bad: &mut Vec<&'static str>,
) -> Option<T> {
    let value = map.get(key).filter(|value| !value.is_null())?;
    let parsed = serde_json::from_value(value.clone()).ok();
    if parsed.is_none() {
        bad.push(key);
    }
    parsed
}

/// Keeps every usable value. A bad field or shortcut resets only itself, so one unreadable
/// value (a file from a newer or older build) does not throw away the rest.
fn parse_settings(bytes: &[u8]) -> (DictationSettings, Option<String>) {
    let parsed = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| error.to_string())
        .and_then(|value| match value {
            serde_json::Value::Object(map) => Ok(map),
            _ => Err("it is not a settings object".to_owned()),
        });
    let map = match parsed {
        Ok(map) => map,
        Err(error) => {
            return (
                DictationSettings::default(),
                Some(format!(
                    "The saved dictation settings could not be read ({error}). Using the defaults until you change something."
                )),
            )
        }
    };
    let defaults = DictationSettings::default();
    let mut bad = Vec::new();
    let mut settings = DictationSettings {
        toggle_shortcut: settings_field(&map, "toggleShortcut", &mut bad)
            .unwrap_or(defaults.toggle_shortcut),
        hold_to_talk_shortcut: settings_field(&map, "holdToTalkShortcut", &mut bad)
            .unwrap_or(defaults.hold_to_talk_shortcut),
        keep_microphone_ready: settings_field(&map, "keepMicrophoneReady", &mut bad)
            .unwrap_or(true),
        review_before_insert: settings_field(&map, "reviewBeforeInsert", &mut bad)
            .unwrap_or(defaults.review_before_insert),
        stopped_models: settings_field(&map, "stoppedModels", &mut bad).unwrap_or_default(),
        settings_version: settings_field(&map, "settingsVersion", &mut bad).unwrap_or(0),
        active_model: settings_field(&map, "activeModel", &mut bad),
        active_model_path: settings_field(&map, "activeModelPath", &mut bad),
    };
    // Only a file from before version 1 can hold the old default. After that the same value
    // is a shortcut the owner chose, so it stays.
    if settings.settings_version < 1 && settings.hold_to_talk_shortcut == "Ctrl+Alt+Space" {
        settings.hold_to_talk_shortcut = DEFAULT_HOLD.to_owned();
    }
    // Before version 2 both Shift keys was the default, so a file holding it never chose it.
    if settings.settings_version < 2 && is_dual_shift_shortcut(&settings.hold_to_talk_shortcut) {
        settings.hold_to_talk_shortcut = DEFAULT_HOLD.to_owned();
    }
    settings.settings_version = SETTINGS_VERSION;
    let mut resets = Vec::new();
    if !bad.is_empty() {
        resets.push(format!(
            "These were unreadable and reset: {}",
            bad.join(", ")
        ));
    }
    if special_hold(&settings.toggle_shortcut).is_some()
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
            "Hold to talk was \"{}\", now Right Alt",
            settings.hold_to_talk_shortcut
        ));
        settings.hold_to_talk_shortcut = DEFAULT_HOLD.to_owned();
    }
    let notice = (!resets.is_empty()).then(|| {
        format!(
            "Some saved dictation settings could not be used. {}.",
            resets.join(". ")
        )
    });
    (settings, notice)
}

/// A file with no version gets one straight away, so the migration above runs only once.
fn needs_version_stamp(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| {
            value.as_object().map(|map| {
                map.get("settingsVersion")
                    .and_then(serde_json::Value::as_u64)
                    .is_none_or(|version| version < u64::from(SETTINGS_VERSION))
            })
        })
        .unwrap_or(false)
}

fn load_settings(app: &AppHandle) -> (DictationSettings, Option<String>) {
    let failed = |reason: String| (DictationSettings::default(), Some(reason));
    let path = match settings_path(app) {
        Ok(path) => path,
        Err(error) => return failed(error),
    };
    match std::fs::read(&path) {
        // Room for a selection with a long path to another app's folder.
        Ok(bytes) if bytes.len() <= MAX_SETTINGS_BYTES => {
            let (settings, notice) = parse_settings(&bytes);
            let is_json = serde_json::from_slice::<serde_json::Value>(&bytes).is_ok();
            if !is_json {
                // The next save overwrites this file, so keep what cannot be read.
                let _ = std::fs::write(path.with_extension("json.bad"), &bytes);
            }
            if needs_version_stamp(&bytes) {
                if let Err(error) = persist_settings(app, &settings) {
                    eprintln!("Could not update the dictation settings version: {error}");
                }
            }
            (settings, notice)
        }
        Ok(_) => failed("The saved dictation settings are too large. Using the defaults.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (DictationSettings::default(), None)
        }
        Err(error) => failed(format!("Could not read the dictation settings: {error}")),
    }
}

fn persist_settings(app: &AppHandle, settings: &DictationSettings) -> Result<(), String> {
    persist_settings_at(&settings_path(app)?, settings)
}

fn persist_settings_at(path: &Path, settings: &DictationSettings) -> Result<(), String> {
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
    replace_file(&tmp, path).map_err(|e| format!("Could not commit settings: {e}"))
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
pub fn dictation_keep_warm(service: State<'_, Arc<DictationService>>) {
    service.keep_warm();
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
    // Before anything can refuse the save: a refused shortcut must not leave every shortcut
    // dead until the pause runs out.
    let resumed = service.shortcuts_paused.swap(false, Ordering::AcqRel);
    if resumed {
        service.pause_generation.fetch_add(1, Ordering::AcqRel);
    }
    let previous = service.settings();
    if let Err(error) = validate_settings(&settings) {
        if resumed {
            let _ = register_shortcuts(&app, &service, &previous);
            sync_dual_shift(&service);
            service.emit_state();
        }
        return Err(error);
    }
    // The model is chosen only through its own commands, which use backend-verified paths.
    // A page can never point the recorder at a file by sending settings.
    let settings = DictationSettings {
        stopped_models: previous.stopped_models.clone(),
        active_model: previous.active_model.clone(),
        active_model_path: previous.active_model_path.clone(),
        settings_version: SETTINGS_VERSION,
        ..settings
    };
    let shortcuts_changed = settings.toggle_shortcut != previous.toggle_shortcut
        || settings.hold_to_talk_shortcut != previous.hold_to_talk_shortcut;
    let had_problems = service
        .runtime
        .lock()
        .is_ok_and(|runtime| runtime.shortcut_problems != ShortcutProblems::default());
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

/// A shortcut another app held may be free now. Runs on "Try again" and when Settings opens.
#[tauri::command]
pub fn dictation_shortcuts_retry(app: AppHandle, service: State<'_, Arc<DictationService>>) {
    if service.shortcuts_paused.load(Ordering::Acquire) {
        return;
    }
    let current = service.settings();
    if let Err(error) = register_shortcuts(&app, &service, &current) {
        eprintln!("Dictation shortcut registration warning: {error}");
    }
    sync_dual_shift(&service);
    service.emit_state();
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

    const FIXTURE_ENTRY: model::CatalogEntry = model::CatalogEntry {
        id: "ggml-base-q5_1.bin",
        label: "Fixture Base",
        bytes: 3,
        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        speed: "",
        accuracy: "",
        note: None,
        offered: true,
        memory_mb: 1,
    };

    struct ServiceFixture {
        service: Arc<DictationService>,
        root: PathBuf,
        calls: Arc<AtomicU64>,
        entered: std::sync::mpsc::Receiver<()>,
        release: std::sync::mpsc::Sender<bool>,
    }
    impl ServiceFixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("tuck-service-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(root.join("dictation")).unwrap();
            std::fs::create_dir_all(root.join("external")).unwrap();
            let calls = Arc::new(AtomicU64::new(0));
            let (entered_tx, entered) = std::sync::mpsc::channel();
            let (release, release_rx) = std::sync::mpsc::channel();
            let release_rx = Mutex::new(release_rx);
            let count = calls.clone();
            let mut service = DictationService::new();
            service.storage_root = Some(root.clone());
            service.locations = vec![model::Location {
                app: "Hex",
                root: root.join("external"),
                depth: 2,
                huggingface: false,
            }];
            service.acquisition_fixture = Some(AcquisitionFixture {
                entry: &FIXTURE_ENTRY,
                network: Arc::new(move |dir, cancel| {
                    count.fetch_add(1, Ordering::SeqCst);
                    entered_tx.send(()).unwrap();
                    let success: bool = release_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(10))
                        .unwrap();
                    if cancel.load(Ordering::Acquire) || !success {
                        return Err("fixture acquisition failed".into());
                    }
                    std::fs::write(dir.join(FIXTURE_ENTRY.id), b"abc").map_err(|e| e.to_string())
                }),
            });
            let fixture = Self {
                service: Arc::new(service),
                root,
                calls,
                entered,
                release,
            };
            fixture.select_missing();
            fixture
        }
        fn select_missing(&self) {
            self.service
                .commit_selection(
                    Some(FIXTURE_ENTRY.id.into()),
                    Some(self.root.join("external/missing.bin")),
                )
                .unwrap();
        }
        fn wait(&self) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                if self
                    .service
                    .runtime
                    .lock()
                    .unwrap()
                    .downloading_model
                    .is_none()
                {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "worker did not finish"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        fn disk(&self) -> DictationSettings {
            serde_json::from_slice(&std::fs::read(self.root.join("dictation.json")).unwrap())
                .unwrap()
        }
        fn entered(&self) {
            self.entered
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
        }
    }
    impl Drop for ServiceFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn service_worker_rediscovers_moved_external_without_network() {
        let f = ServiceFixture::new();
        let moved = f.root.join("external").join(FIXTURE_ENTRY.id);
        std::fs::write(&moved, b"abc").unwrap();
        f.service.repair_missing().unwrap();
        f.wait();
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
        assert_eq!(f.disk().active_model_path.as_ref(), Some(&moved));
        assert_eq!(f.service.resolve_model().active.unwrap().path, moved);
        assert_eq!(std::fs::read(&moved).unwrap(), b"abc");
        let runtime = f.service.runtime.lock().unwrap();
        let notice = runtime.settings_notice.as_deref().unwrap();
        assert!(notice.starts_with("Using existing "), "{notice}");
    }

    #[test]
    fn service_worker_singleflight_survives_background_scan() {
        let mut f = ServiceFixture::new();
        let (scan_tx, scan_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let resume_rx = Mutex::new(resume_rx);
        Arc::get_mut(&mut f.service).unwrap().scan_barrier = Some(Arc::new(move || {
            scan_tx.send(()).unwrap();
            resume_rx
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
        }));
        f.service
            .start_model_download(FIXTURE_ENTRY.id, false)
            .unwrap();
        f.entered();
        // Candidate appears after acquisition preflight, before background discovery.
        std::fs::write(f.root.join("external").join(FIXTURE_ENTRY.id), b"bad").unwrap();
        let service = f.service.clone();
        std::thread::spawn(move || {
            service.repair_missing().unwrap();
            service.scan_models().unwrap();
        })
        .join()
        .unwrap();
        scan_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        assert!(f
            .service
            .start_model_download(FIXTURE_ENTRY.id, false)
            .is_err());
        f.release.send(true).unwrap();
        f.wait();
        resume_tx.send(()).unwrap();
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        assert_eq!(f.disk().active_model_path, None);
        assert_eq!(
            f.service.runtime.lock().unwrap().settings_notice.as_deref(),
            Some(format!("{} is ready", FIXTURE_ENTRY.label).as_str())
        );
        assert_eq!(
            f.service.resolve_model().active.unwrap().entry.id,
            FIXTURE_ENTRY.id
        );
    }

    #[test]
    fn service_worker_persistence_failure_remains_visible() {
        let f = ServiceFixture::new();
        let old = f.service.settings();
        f.service
            .start_model_download(FIXTURE_ENTRY.id, false)
            .unwrap();
        f.entered();
        // A directory at the destination forces the real atomic replace to fail.
        std::fs::remove_file(f.root.join("dictation.json")).unwrap();
        std::fs::create_dir(f.root.join("dictation.json")).unwrap();
        f.release.send(true).unwrap();
        f.wait();
        assert_eq!(
            f.service.settings().active_model_path,
            old.active_model_path
        );
        let runtime = f.service.runtime.lock().unwrap();
        assert!(runtime
            .model_error
            .as_ref()
            .unwrap()
            .contains("commit settings"));
        assert!(!runtime.settings_notice.as_ref().unwrap().contains("ready"));
        assert!(!runtime.settings_notice.as_ref().unwrap().contains("Using"));
        assert!(f.service.resolve_model().active.is_none());
    }

    #[test]
    fn service_worker_selection_and_user_check_guards() {
        for pending_check in [false, true] {
            let f = ServiceFixture::new();
            if pending_check {
                let mut r = f.service.runtime.lock().unwrap();
                r.user_selection_check = Some(r.selection_generation);
            }
            f.service
                .start_model_download(FIXTURE_ENTRY.id, false)
                .unwrap();
            f.entered();
            if !pending_check {
                f.service.commit_selection(None, None).unwrap();
                f.service.set_model_error("newer choice error".into());
            }
            f.release.send(true).unwrap();
            f.wait();
            assert_eq!(
                f.disk().active_model_path,
                f.service.settings().active_model_path
            );
            if pending_check {
                assert!(f.disk().active_model_path.is_some());
            } else {
                assert!(f.disk().active_model.is_none());
                assert_eq!(
                    f.service.runtime.lock().unwrap().model_error.as_deref(),
                    Some("newer choice error")
                );
            }
        }
    }

    #[test]
    fn service_worker_failure_or_cancel_never_downloads_again_until_a_click() {
        for cancel in [false, true] {
            let f = ServiceFixture::new();
            f.service
                .start_model_download(FIXTURE_ENTRY.id, false)
                .unwrap();
            f.entered();
            if cancel {
                f.service.cancel_download().unwrap();
            }
            f.release.send(false).unwrap();
            f.wait();
            f.service.repair_missing().unwrap();
            f.wait();
            assert_eq!(f.calls.load(Ordering::SeqCst), 1);
            f.service
                .start_model_download(FIXTURE_ENTRY.id, false)
                .unwrap();
            f.entered();
            f.release.send(true).unwrap();
            f.wait();
            assert_eq!(f.calls.load(Ordering::SeqCst), 2);
            assert!(f.service.resolve_model().active.is_some());
        }
    }

    #[test]
    fn service_worker_manual_reuse_preserves_other_selection_and_truthful_notice() {
        let f = ServiceFixture::new();
        let a = "ggml-tiny.bin";
        std::fs::write(f.root.join("dictation").join(a), b"selected").unwrap();
        f.service.commit_selection(Some(a.into()), None).unwrap();
        let borrowed = f.root.join("external").join(FIXTURE_ENTRY.id);
        std::fs::write(&borrowed, b"abc").unwrap();
        f.service
            .start_model_download(FIXTURE_ENTRY.id, false)
            .unwrap();
        f.wait();
        assert_eq!(f.disk().active_model.as_deref(), Some(a));
        assert_eq!(f.service.resolve_model().active.unwrap().entry.id, a);
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
        assert!(f
            .service
            .runtime
            .lock()
            .unwrap()
            .settings_notice
            .as_ref()
            .unwrap()
            .starts_with("Verified existing"));
        assert_eq!(std::fs::read(borrowed).unwrap(), b"abc");
    }

    #[test]
    fn service_worker_rejects_hash_mismatch_and_unsupported_filename() {
        for name in [FIXTURE_ENTRY.id, "unsupported.onnx"] {
            let f = ServiceFixture::new();
            let path = f.root.join("external").join(name);
            let bytes = if name == FIXTURE_ENTRY.id {
                b"bad"
            } else {
                b"abc"
            };
            std::fs::write(&path, bytes).unwrap();
            let selected = f.disk().active_model_path;
            f.service.repair_missing().unwrap();
            f.wait();
            assert_eq!(f.calls.load(Ordering::SeqCst), 0);
            assert_eq!(f.disk().active_model_path, selected);
            assert!(f.service.resolve_model().active.is_none());
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }

    #[test]
    fn service_worker_asks_before_downloading_a_missing_model() {
        let f = ServiceFixture::new();
        let selected = f.disk().active_model_path;
        f.service.repair_missing().unwrap();
        f.wait();
        assert_eq!(
            f.calls.load(Ordering::SeqCst),
            0,
            "no download without a click"
        );
        assert_eq!(f.disk().active_model_path, selected);
        {
            let runtime = f.service.runtime.lock().unwrap();
            assert_eq!(runtime.model_error, None);
            let notice = runtime.settings_notice.as_deref().unwrap();
            assert!(notice.contains("is missing"), "{notice}");
        }
        // The click: only now does the download run, and it makes the model usable.
        f.service
            .start_model_download(FIXTURE_ENTRY.id, false)
            .unwrap();
        f.entered();
        f.release.send(true).unwrap();
        f.wait();
        assert_eq!(f.calls.load(Ordering::SeqCst), 1);
        assert!(f.service.resolve_model().active.is_some());
    }

    #[test]
    fn service_worker_fresh_service_looks_again_for_a_missing_selection() {
        let mut f = ServiceFixture::new();
        f.service.repair_missing().unwrap();
        f.wait();
        let previous = f.service.clone();
        let copy = f.root.join("external").join(FIXTURE_ENTRY.id);
        std::fs::write(&copy, b"abc").unwrap();
        let mut fresh = DictationService::new();
        fresh.storage_root = Some(f.root.clone());
        fresh.locations = previous.locations.clone();
        *fresh.settings.write().unwrap() = f.disk();
        fresh.acquisition_fixture = Some(AcquisitionFixture {
            entry: &FIXTURE_ENTRY,
            network: previous
                .acquisition_fixture
                .as_ref()
                .unwrap()
                .network
                .clone(),
        });
        f.service = Arc::new(fresh);
        f.service.repair_missing().unwrap();
        f.wait();
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
        assert_eq!(f.service.resolve_model().active.unwrap().path, copy);
    }

    fn recording_service() -> DictationService {
        let service = DictationService::new();
        {
            let mut runtime = service.runtime.lock().unwrap();
            runtime.session_id = 1;
            runtime.status = "recording".into();
            runtime.active = Some(ActiveSession {
                stop: Arc::new(AtomicBool::new(false)),
                cancel: Arc::new(AtomicBool::new(false)),
                mode: RecordingMode::Toggle,
            });
        }
        service
    }

    #[test]
    fn update_waits_for_work_in_progress_but_not_for_resting_states() {
        let service = DictationService::new();
        for resting in ["idle", "error", "modelMissing"] {
            service.runtime.lock().unwrap().status = resting.into();
            assert_eq!(
                service.update_blocker(),
                None,
                "{resting} must allow an update"
            );
        }
        for busy in [
            "starting",
            "recording",
            "transcribing",
            "processing",
            "review",
            "inserting",
            "cancelling",
        ] {
            service.runtime.lock().unwrap().status = busy.into();
            assert!(
                service.update_blocker().is_some(),
                "{busy} must block an update"
            );
        }
        service.runtime.lock().unwrap().status = "idle".into();

        service.runtime.lock().unwrap().downloading_model = Some("ggml-tiny.bin");
        assert!(service.update_blocker().unwrap().contains("download"));
        service.runtime.lock().unwrap().downloading_model = None;

        service.runtime.lock().unwrap().removing_model = true;
        assert!(service.update_blocker().unwrap().contains("model change"));
        service.runtime.lock().unwrap().removing_model = false;

        service.model_busy.store(true, Ordering::Release);
        service.runtime.lock().unwrap().status = "transcribing".into();
        assert!(service.update_blocker().unwrap().contains("dictation"));
        service.runtime.lock().unwrap().status = "idle".into();
        service.model_busy.store(false, Ordering::Release);

        assert_eq!(service.update_blocker(), None);
        assert!(recording_service().update_blocker().is_some());
    }

    #[test]
    fn cancelled_finalization_does_not_keep_text_for_review() {
        let service = recording_service();
        service.cancel().unwrap();
        service.finish_transcript("discard me".into(), None, &AtomicBool::new(true));
        let snapshot = service.snapshot();
        assert_eq!(snapshot.status, "idle");
        assert!(!snapshot.backend.is_empty());
        assert!(snapshot.transcript.is_none());
    }

    #[test]
    fn model_change_is_rejected_during_recording() {
        let service = Arc::new(recording_service());
        assert!(service.use_model("tiny").is_err());
        assert_eq!(service.runtime.lock().unwrap().selection_generation, 0);
    }

    #[test]
    fn failed_restoration_is_single_attempt_per_launch_and_review_is_preserved() {
        fn fixture() -> Arc<DictationService> {
            let mut service = DictationService::new();
            let root = std::env::temp_dir().join("tuck-missing-repair-fixture");
            service.locations = vec![model::Location {
                app: "Test",
                root: root.clone(),
                depth: 1,
                huggingface: false,
            }];
            let mut settings = service.settings.write().unwrap();
            settings.active_model = Some(model::PARAKEET.into());
            settings.active_model_path = Some(root.join(model::PARAKEET));
            drop(settings);
            Arc::new(service)
        }
        let service = fixture();
        // No initialized app: acquiring the managed directory fails before any HTTP.
        assert!(service.repair_missing().is_err());
        assert!(service.runtime.lock().unwrap().repair_attempt.is_some());
        for _ in 0..3 {
            service.repair_missing().unwrap();
        }
        let fresh_launch = fixture();
        assert!(
            fresh_launch.repair_missing().is_err(),
            "restart permits one retry"
        );
        let review = fixture();
        review.runtime.lock().unwrap().status = "review".into();
        review.repair_missing().unwrap();
        let runtime = review.runtime.lock().unwrap();
        assert_eq!(runtime.status, "review");
        assert!(runtime.repair_attempt.is_none());
    }

    #[test]
    fn explicit_stop_or_new_choice_cancels_repair_and_prevents_stale_activation() {
        for choice in ["stop", "different model"] {
            let service = DictationService::new();
            let mut runtime = service.runtime.lock().unwrap();
            let cancel = Arc::new(AtomicBool::new(false));
            runtime.download_cancel = Some(cancel.clone());
            let generation = runtime.selection_generation;
            let mut selected = "original external";
            apply_selection_commit(&mut runtime, None, false, || {
                selected = choice;
                Ok(())
            })
            .unwrap();
            assert!(cancel.load(Ordering::Acquire));
            assert!(
                !apply_selection_commit(&mut runtime, Some(generation), true, || {
                    selected = "repaired";
                    Ok(())
                })
                .unwrap()
            );
            assert_eq!(selected, choice);
        }
    }

    #[test]
    fn download_pinning_cannot_overwrite_a_committed_newer_selection() {
        let service = DictationService::new();
        let mut runtime = service.runtime.lock().unwrap();
        let download_generation = runtime.selection_generation;
        runtime.selection_generation += 1; // user request
        let user_generation = runtime.selection_generation;
        let mut selected = "old";
        assert!(
            apply_selection_commit(&mut runtime, Some(user_generation), false, || {
                selected = "user";
                Ok(())
            })
            .unwrap()
        );
        assert!(
            !apply_selection_commit(&mut runtime, Some(download_generation), true, || {
                selected = "download";
                Ok(())
            })
            .unwrap()
        );
        assert_eq!(selected, "user");
    }

    #[test]
    fn background_scan_does_not_suppress_restoration_commit() {
        let service = DictationService::new();
        let mut runtime = service.runtime.lock().unwrap();
        let generation = runtime.selection_generation;
        runtime.checking_models.push(model::PARAKEET.into());
        let mut persisted = false;
        assert!(
            apply_selection_commit(&mut runtime, Some(generation), true, || {
                persisted = true;
                Ok(())
            })
            .unwrap()
        );
        assert!(persisted);
    }

    #[test]
    fn download_pinning_does_not_invalidate_in_progress_user_verification() {
        let service = DictationService::new();
        let mut runtime = service.runtime.lock().unwrap();
        let download_generation = runtime.selection_generation;
        runtime.selection_generation += 1; // newer user request still hashing
        runtime.checking_models.push("ggml-base.bin".into());
        let user_generation = runtime.selection_generation;
        runtime.user_selection_check = Some(user_generation);
        for token in [download_generation, user_generation] {
            // Also protects a user verification already pending when download began.
            assert!(
                !apply_selection_commit(&mut runtime, Some(token), true, || panic!(
                    "download displaced verification"
                ))
                .unwrap()
            );
            assert_eq!(runtime.selection_generation, user_generation);
        }
        assert!(
            apply_selection_commit(&mut runtime, Some(user_generation), false, || Ok(())).unwrap()
        );
    }

    #[test]
    fn stopped_models_persist_and_are_excluded_from_fallback() {
        let settings = DictationSettings {
            stopped_models: vec!["ggml-base.bin".into()],
            ..Default::default()
        };
        let bytes = serde_json::to_vec(&settings).unwrap();
        assert_eq!(
            parse_settings(&bytes).0.stopped_models,
            settings.stopped_models
        );
        let resolution = model::resolve_active(
            None,
            None,
            Path::new("C:/models"),
            |_| true,
            |path| {
                path.file_name().is_some_and(|n| n == "ggml-base.bin")
                    && !settings
                        .stopped_models
                        .iter()
                        .any(|id| path.file_name().is_some_and(|n| n == id.as_str()))
            },
        );
        assert!(resolution.active.is_none());
    }

    #[test]
    fn model_selection_commit_rechecks_session_and_generation() {
        let service = DictationService::new();
        let mut runtime = service.runtime.lock().unwrap();
        runtime.selection_generation = 2;
        assert!(validate_selection_commit(&runtime, Some(1)).is_err());
        assert!(validate_selection_commit(&runtime, Some(2)).is_ok());
        runtime.active = Some(ActiveSession {
            stop: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            mode: RecordingMode::Toggle,
        });
        assert!(validate_selection_commit(&runtime, Some(2)).is_err());
    }

    #[test]
    fn cancellation_cannot_be_overwritten_and_refuses_a_new_session() {
        let service = DictationService::new();
        service
            .claim_session(
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
                RecordingMode::Toggle,
            )
            .unwrap();
        service.cancel().unwrap();
        for phase in ["recording", "transcribing", "processing"] {
            service.session_status(phase);
        }
        assert_eq!(service.runtime.lock().unwrap().status, "cancelling");
        assert!(service
            .claim_session(
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
                RecordingMode::Toggle
            )
            .unwrap_err()
            .contains("Wait for the speech engine"));
    }

    #[test]
    fn live_words_show_only_while_recording_and_never_outlive_it() {
        let service = DictationService::new();
        let stop = Arc::new(AtomicBool::new(false));
        {
            let mut runtime = service.runtime.lock().unwrap();
            runtime.active = Some(ActiveSession {
                stop: stop.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                mode: RecordingMode::HoldToTalk,
            });
            runtime.status = "recording".into();
        }
        service.publish_live_text(" hello there ");
        assert_eq!(
            service.runtime.lock().unwrap().live_text.as_deref(),
            Some("hello there")
        );
        // After release the stream's late words are ignored; the final text decides.
        stop.store(true, Ordering::Release);
        service.publish_live_text("hello there friend");
        assert_eq!(
            service.runtime.lock().unwrap().live_text.as_deref(),
            Some("hello there")
        );
        service.clear_active();
        assert_eq!(service.runtime.lock().unwrap().live_text, None);
        // Nothing appears when no recording runs.
        service.runtime.lock().unwrap().status = "idle".into();
        service.publish_live_text("ghost");
        assert_eq!(service.runtime.lock().unwrap().live_text, None);
    }

    #[test]
    fn a_waiting_transcript_never_blocks_the_next_dictation() {
        let service = DictationService::new();
        {
            let mut runtime = service.runtime.lock().unwrap();
            runtime.status = "review".into();
            runtime.transcript = Some("hello hello".into());
            runtime.message = Some("The clipboard changed before paste.".into());
        }
        service.release_waiting_transcript();
        let runtime = service.runtime.lock().unwrap();
        assert_eq!(runtime.status, "idle");
        assert_eq!(runtime.transcript, None);
        assert_eq!(runtime.message, None);
        drop(runtime);
        assert!(service
            .claim_session(
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
                RecordingMode::HoldToTalk
            )
            .is_ok());
    }

    #[test]
    fn recording_only_follows_starting() {
        let service = DictationService::new();
        service.session_status("recording");
        assert_eq!(service.runtime.lock().unwrap().status, "idle");
        service
            .claim_session(
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
                RecordingMode::Toggle,
            )
            .unwrap();
        service.session_status("recording");
        service.session_status("processing");
        service.session_status("recording");
        assert_eq!(service.runtime.lock().unwrap().status, "processing");
    }

    #[test]
    fn settings_default_to_explicit_hotkeys_and_immediate_insert() {
        let settings = DictationSettings::default();
        assert_eq!(settings.toggle_shortcut, DEFAULT_TOGGLE);
        assert_eq!(settings.hold_to_talk_shortcut, DEFAULT_HOLD);
        assert!(!settings.review_before_insert);
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
        // Both Shift keys was the old default, so it moves to the new one.
        assert_eq!(settings.hold_to_talk_shortcut, "RightAlt");
    }

    #[test]
    fn both_shift_keys_chosen_after_the_migration_stays() {
        let (settings, notice) = parse_settings(
            br#"{"toggleShortcut":"Ctrl+Shift+Space","holdToTalkShortcut":"Shift+Shift","settingsVersion":2}"#,
        );
        assert_eq!(notice, None);
        assert_eq!(settings.hold_to_talk_shortcut, "Shift+Shift");
        let (settings, _) = parse_settings(
            br#"{"toggleShortcut":"Ctrl+Shift+Space","holdToTalkShortcut":"Shift+Shift","settingsVersion":1}"#,
        );
        assert_eq!(settings.hold_to_talk_shortcut, "RightAlt");
        assert!(needs_version_stamp(br#"{"settingsVersion":1}"#));
        assert!(!needs_version_stamp(br#"{"settingsVersion":2}"#));
    }

    #[test]
    fn right_alt_is_a_hold_only_shortcut() {
        assert!(is_right_alt_shortcut("RightAlt"));
        assert!(is_right_alt_shortcut("right alt"));
        assert!(!is_right_alt_shortcut("Alt"));
        let mut settings = DictationSettings::default();
        assert_eq!(settings.hold_to_talk_shortcut, "RightAlt");
        assert!(validate_settings(&settings).is_ok());
        settings.toggle_shortcut = "RightAlt".into();
        assert!(validate_settings(&settings).is_err());
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
        assert_eq!(settings.settings_version, SETTINGS_VERSION);
    }

    /// Review finding: a deliberate Ctrl+Alt+Space became both Shift on every restart.
    #[test]
    fn a_chosen_ctrl_alt_space_survives_a_restart() {
        let settings = DictationSettings {
            hold_to_talk_shortcut: "Ctrl+Alt+Space".into(),
            ..DictationSettings::default()
        };
        let saved = serde_json::to_vec(&settings).unwrap();
        assert!(!needs_version_stamp(&saved));
        let (restored, notice) = parse_settings(&saved);
        assert_eq!(restored.hold_to_talk_shortcut, "Ctrl+Alt+Space");
        assert_eq!(notice, None);
    }

    #[test]
    fn a_file_with_no_version_is_stamped_once() {
        assert!(needs_version_stamp(
            br#"{"toggleShortcut":"Ctrl+Shift+Space"}"#
        ));
        assert!(!needs_version_stamp(b"{ not json"));
    }

    #[test]
    fn one_wrong_field_type_keeps_every_other_setting() {
        let (settings, notice) = parse_settings(
            br#"{"toggleShortcut":"Ctrl+Alt+F8","holdToTalkShortcut":"Shift+Shift","reviewBeforeInsert":"yes","partialPreview":false,"activeModel":"ggml-base.bin"}"#,
        );
        assert_eq!(settings.toggle_shortcut, "Ctrl+Alt+F8");
        assert_eq!(settings.active_model.as_deref(), Some("ggml-base.bin"));
        assert!(!settings.review_before_insert, "the default for that field");
        assert!(notice.unwrap().contains("reviewBeforeInsert"));
    }

    fn pending_review(service: &DictationService) {
        let mut runtime = service.runtime.lock().unwrap();
        runtime.status = "review".into();
        runtime.transcript = Some("pending transcript".into());
    }

    /// Review finding: rolling across both Shift keys wiped a pending transcript.
    #[test]
    fn a_press_with_no_model_keeps_a_pending_review() {
        let service = DictationService::new();
        pending_review(&service);
        service.report_no_model("Choose a speech model in Settings.".into());
        let runtime = service.runtime.lock().unwrap();
        assert_eq!(runtime.status, "review");
        assert_eq!(runtime.transcript.as_deref(), Some("pending transcript"));
        assert!(runtime.model_error.is_some());
    }

    #[test]
    fn a_session_that_cannot_start_keeps_a_pending_review() {
        let service = DictationService::new();
        pending_review(&service);
        service.runtime.lock().unwrap().removing_model = true;
        assert!(service
            .claim_session(
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
                RecordingMode::HoldToTalk,
            )
            .is_err());
        assert_eq!(
            service.runtime.lock().unwrap().transcript.as_deref(),
            Some("pending transcript")
        );
    }

    #[test]
    fn a_session_that_starts_gives_up_the_review() {
        let service = DictationService::new();
        pending_review(&service);
        service
            .claim_session(
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicBool::new(false)),
                RecordingMode::HoldToTalk,
            )
            .unwrap();
        let runtime = service.runtime.lock().unwrap();
        assert!(runtime.transcript.is_none());
        assert_eq!(runtime.status, "starting");
    }

    #[test]
    fn a_dual_shift_failure_shows_on_the_hold_row_until_it_clears() {
        let service = DictationService::new();
        service.set_dual_shift_problem(Some("Holding both Shift keys is unavailable".into()));
        let shown = service.snapshot().shortcut_problems.hold.clone();
        assert_eq!(
            shown.as_deref(),
            Some("Holding both Shift keys is unavailable")
        );
        service.set_dual_shift_problem(None);
        assert_eq!(service.snapshot().shortcut_problems.hold, None);
    }

    #[test]
    fn a_bad_saved_shortcut_keeps_the_other_settings_and_says_so() {
        let (settings, notice) = parse_settings(
            br#"{"toggleShortcut":"Ctrl+Shift+Space","holdToTalkShortcut":"not a key","reviewBeforeInsert":true,"partialPreview":false}"#,
        );
        assert_eq!(settings.hold_to_talk_shortcut, DEFAULT_HOLD);
        assert!(settings.review_before_insert, "unrelated values survive");
        assert!(notice.unwrap().contains("not a key"));

        let (toggle, notice) = parse_settings(
            br#"{"toggleShortcut":"Shift+Shift","holdToTalkShortcut":"Ctrl+Alt+F9"}"#,
        );
        assert_eq!(toggle.toggle_shortcut, DEFAULT_TOGGLE);
        assert_eq!(toggle.hold_to_talk_shortcut, "Ctrl+Alt+F9");
        assert!(notice.is_some());
    }

    #[test]
    fn microphone_ready_defaults_on_and_false_round_trips() {
        let (legacy, _) = parse_settings(br#"{"settingsVersion":1}"#);
        assert!(legacy.keep_microphone_ready);
        let (disabled, _) = parse_settings(br#"{"settingsVersion":1,"keepMicrophoneReady":false}"#);
        assert!(!disabled.keep_microphone_ready);
        let bytes = serde_json::to_vec(&disabled).unwrap();
        assert_eq!(parse_settings(&bytes).0, disabled);
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
        let error = "HotKey already registered: HotKey { mods: Modifiers(CONTROL | SHIFT) }";
        let kept = shortcut_taken_message("Ctrl+Shift+Space", error, false);
        assert!(kept.contains("Ctrl+Shift+Space is in use by another app right now"));
        assert!(kept.contains("Try again"), "it may only be taken for now");
        assert!(!kept.contains("Modifiers"));
        let chosen = shortcut_taken_message("Ctrl+Shift+Space", error, true);
        assert!(chosen.contains("Record a different shortcut"));
        assert!(!chosen.contains("Try again"));
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

#[derive(Debug, PartialEq)]
enum ReviewDelivery {
    Typed,
    Copied,
}
fn deliver_review(
    result: Result<bool, target::InsertionFailure>,
) -> Result<ReviewDelivery, target::InsertionFailure> {
    result.map(|pasted| {
        if pasted {
            ReviewDelivery::Typed
        } else {
            ReviewDelivery::Copied
        }
    })
}
#[cfg(test)]
mod final_delivery_tests {
    use super::*;

    #[test]
    fn blocked_focus_provider_does_not_block_meter_cancel_or_grow_workers() {
        use std::sync::mpsc;
        use std::time::Duration;
        let service = Arc::new(DictationService::new());
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        service
            .claim_session(stop.clone(), cancel.clone(), RecordingMode::Toggle)
            .unwrap();
        service.session_status("recording");
        let id = service.runtime.lock().unwrap().session_id;
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let observer = service
            .start_focus_observer(id, stop, cancel.clone(), move || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                false
            })
            .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let worker_service = service.clone();
        let worker = std::thread::spawn(move || {
            worker_service.publish_audio_level(0.5);
            worker_service.cancel().unwrap();
            done_tx.send(()).unwrap();
        });
        let responsive = done_rx.recv_timeout(Duration::from_secs(2)).is_ok();
        if !responsive {
            release_tx.send(()).unwrap();
            observer.join().unwrap();
            panic!("Observer blocked capture or cancel");
        }
        worker.join().unwrap();
        assert!(cancel.load(Ordering::Acquire));
        service.finish_cancelled();
        let next_stop = Arc::new(AtomicBool::new(false));
        let next_cancel = Arc::new(AtomicBool::new(false));
        service
            .claim_session(
                next_stop.clone(),
                next_cancel.clone(),
                RecordingMode::Toggle,
            )
            .unwrap();
        let next_id = service.runtime.lock().unwrap().session_id;
        assert!(service
            .start_focus_observer(next_id, next_stop, next_cancel, || panic!(
                "Unbounded observer"
            ))
            .is_none());
        assert!(
            service.runtime.lock().unwrap().focus_changed,
            "Unobservable focus must copy instead of inserting"
        );
        release_tx.send(()).unwrap();
        observer.join().unwrap();
        assert!(!service.focus_observer_busy.load(Ordering::Acquire));
        service.runtime.lock().unwrap().focus_changed = false;
        service.mark_focus_changed(id);
        assert!(
            !service.runtime.lock().unwrap().focus_changed,
            "Old session cannot affect new session"
        );
    }

    #[test]
    fn legacy_review_preference_survives_without_preview_contract() {
        let settings: DictationSettings = serde_json::from_value(
            serde_json::json!({"reviewBeforeInsert":true,"partialPreview":true}),
        )
        .unwrap();
        assert!(settings.review_before_insert);
        assert!(serde_json::to_value(settings)
            .unwrap()
            .get("partialPreview")
            .is_none());
        let service = DictationService::new();
        {
            let mut runtime = service.runtime.lock().unwrap();
            runtime.status = "review".into();
            runtime.transcript = Some("ready".into());
            assert!(runtime.review_target.is_none());
        }
        let snapshot = service.snapshot();
        assert!(
            snapshot.can_insert,
            "Missing original target must still offer Insert / Copy"
        );
        let json = serde_json::to_value(snapshot).unwrap();
        assert!(json.get("preview").is_none());
        assert!(json.get("previewsPaused").is_none());
        service.runtime.lock().unwrap().insertion_uncertain = true;
        assert!(
            !service.snapshot().can_insert,
            "Ambiguous delivery disables retries"
        );
    }

    #[test]
    fn review_uses_paste_outcome_and_retains_errors() {
        assert_eq!(deliver_review(Ok(true)).unwrap(), ReviewDelivery::Typed);
        assert_eq!(deliver_review(Ok(false)).unwrap(), ReviewDelivery::Copied);
        assert!(deliver_review(Err("Clipboard changed".to_owned().into())).is_err());
        assert!(
            deliver_review(Err(target::InsertionFailure {
                message: "partial shortcut".into(),
                may_have_inserted: true
            }))
            .unwrap_err()
            .may_have_inserted
        );
    }
}
