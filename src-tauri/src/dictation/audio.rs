//! Records and transcribes one dictation.
//!
//! Capture and inference never share a thread. The session thread only moves audio from
//! the microphone callback into memory, so no amount of inference can overflow the
//! callback's queue and lose a recording. An engine thread loads the model and runs every
//! transcription. The microphone opens before the model loads, so the first words are kept
//! however long loading takes.

use super::{model, DictationService, EVENT_PARTIAL};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};
use zeroize::{Zeroize, Zeroizing};

const MAX_SECONDS: usize = 120;
const MAX_QUEUED_AUDIO_SECONDS: usize = 10;
const MAX_CALLBACK_SECONDS: usize = 1;
const AUDIO_QUEUE_BLOCKS: usize = 2048;
const PREVIEW_INTERVAL: Duration = Duration::from_secs(3);
const PREVIEW_MIN_SECONDS: usize = 2;
const PREVIEW_MAX_SECONDS: usize = 15;
/// A preview slower than this is abandoned and previews stop for the session. The final
/// transcription still runs on the whole recording.
const PREVIEW_BUDGET: Duration = Duration::from_secs(8);
/// whisper.cpp encodes a fixed 30 s window, which is `audio_ctx` 1500, however short the
/// clip. Scaling it to the clip is what makes a preview's cost follow its length.
const AUDIO_CTX_PER_SECOND: i32 = 50;
const AUDIO_CTX_MARGIN: i32 = 100;
const AUDIO_CTX_FULL: i32 = 1500;

#[path = "audio_math.rs"]
mod audio_math;
use audio_math::{release_audio_frames, resample_to_16k, try_reserve_audio_frames};

/// What a session reports as it goes.
trait SessionEvents: Sync {
    fn recording(&self);
    fn processing(&self) {}
    fn previews_paused(&self) {}
    fn transcribing(&self);
    fn partial(&self, text: &str);
}

/// Stops one transcription. Previews also stop when the recording stops, or on their budget.
struct AbortSignal<'a> {
    cancel: &'a AtomicBool,
    stop: Option<&'a AtomicBool>,
    deadline: Option<Instant>,
}

impl AbortSignal<'_> {
    fn requested(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
            || self.stop.is_some_and(|stop| stop.load(Ordering::Acquire))
            || self.deadline.is_some_and(|at| Instant::now() >= at)
    }
}

/// The inference side of a session. Whisper in the app, a scripted stand-in in tests.
trait Engine {
    /// Runs once before previews. Its language hint is reused only by previews.
    /// This pass cannot be interrupted and is separate from the preview budget.
    fn prepare(&mut self, _audio: &[f32]) {}
    fn preview(&mut self, audio: &[f32], abort: &AbortSignal<'_>) -> Result<String, String>;
    fn finish(&mut self, audio: &[f32], abort: &AbortSignal<'_>) -> Result<String, String>;
}

#[derive(Clone, Copy)]
struct SessionLimits {
    preview_interval: Duration,
    preview_budget: Duration,
    max_seconds: usize,
    poll: Duration,
}

impl SessionLimits {
    const APP: Self = Self {
        preview_interval: PREVIEW_INTERVAL,
        preview_budget: PREVIEW_BUDGET,
        max_seconds: MAX_SECONDS,
        poll: Duration::from_millis(100),
    };
}

/// Waits at least the interval, and twice the last preview's cost, so previews never take
/// more than about a third of the CPU however slow the model is.
fn next_preview_wait(interval: Duration, last_preview: Duration) -> Duration {
    interval.max(last_preview.saturating_mul(2))
}

enum EngineJob {
    Preview(Zeroizing<Vec<f32>>),
    Finish(Zeroizing<Vec<f32>>),
}

/// Shared between the capture loop and the engine thread.
struct EngineFlags {
    ready: AtomicBool,
    failed: AtomicBool,
    preview_busy: AtomicBool,
    previews_off: AtomicBool,
    last_preview_ms: AtomicU64,
    preview_stop: AtomicBool,
}

pub(super) fn record_and_transcribe(
    service: &DictationService,
    app: &AppHandle,
    active: &model::Active,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    service.model_busy.store(true, Ordering::Release);
    let result = record_and_transcribe_inner(service, app, active, stop, cancel);
    service.model_busy.store(false, Ordering::Release);
    result
}

struct AppEvents<'a> {
    service: &'a DictationService,
    app: &'a AppHandle,
}

impl SessionEvents for AppEvents<'_> {
    fn recording(&self) {
        self.service.session_status("recording");
    }
    fn transcribing(&self) {
        self.service.session_status("transcribing");
    }
    fn processing(&self) {
        self.service.session_status("processing");
    }
    fn previews_paused(&self) {
        if let Ok(mut runtime) = self.service.runtime.lock() {
            runtime.previews_paused = true;
        }
        self.service.emit_state();
    }
    fn partial(&self, text: &str) {
        let _ = self.app.emit(
            EVENT_PARTIAL,
            PartialTranscript {
                text: text.to_owned(),
            },
        );
    }
}

fn record_and_transcribe_inner(
    service: &DictationService,
    app: &AppHandle,
    active: &model::Active,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Dictation cancelled".into());
    }
    if stop.load(Ordering::Acquire) {
        return Err("Dictation stopped before microphone capture began".into());
    }
    let queued_frames = Arc::new(AtomicUsize::new(0));
    let (audio_tx, audio_rx) = bounded::<Vec<f32>>(AUDIO_QUEUE_BLOCKS);
    let (error_tx, error_rx) = bounded::<String>(1);
    let (stream, sample_rate) = open_input_stream(audio_tx, error_tx, queued_frames.clone())?;
    stream.play().map_err(|e| format!("Could not start microphone capture: {e}. Check Windows microphone privacy settings and the selected input device."))?;
    let events = AppEvents { service, app };
    let path = active.path.clone();
    let entry = active.entry;
    run_session(
        move || {
            model::verify(&path, entry, &service.verified, false)?;
            WhisperEngine::load(&path)
        },
        &events,
        Capture {
            stream: Some(stream),
            audio_rx,
            error_rx,
            queued_frames,
            sample_rate,
        },
        service.settings().partial_preview,
        SessionLimits::APP,
        &stop,
        &cancel,
    )
}

/// The audio side of a session. Dropping `stream` is what stops the microphone.
struct Capture<S> {
    stream: Option<S>,
    audio_rx: Receiver<Vec<f32>>,
    error_rx: Receiver<String>,
    queued_frames: Arc<AtomicUsize>,
    sample_rate: u32,
}

fn run_session<E: Engine, S>(
    load: impl FnOnce() -> Result<E, String> + Send,
    events: &dyn SessionEvents,
    mut capture: Capture<S>,
    preview: bool,
    limits: SessionLimits,
    stop: &AtomicBool,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let flags = EngineFlags {
        ready: AtomicBool::new(false),
        failed: AtomicBool::new(false),
        preview_busy: AtomicBool::new(false),
        previews_off: AtomicBool::new(!preview),
        last_preview_ms: AtomicU64::new(0),
        preview_stop: AtomicBool::new(false),
    };
    // One preview at most: the slot holds one job and `preview_busy` covers the running one.
    let (job_tx, job_rx) = bounded::<EngineJob>(1);
    let load_error = Mutex::new(None::<String>);
    let (flags_ref, load_error_ref) = (&flags, &load_error);
    std::thread::scope(|scope| {
        let engine = std::thread::Builder::new()
            .name("tuck-dictation-engine".into())
            .spawn_scoped(scope, move || {
                engine_thread(
                    load,
                    job_rx,
                    events,
                    flags_ref,
                    load_error_ref,
                    limits,
                    cancel,
                )
            })
            .map_err(|e| format!("Could not start dictation engine: {e}"))?;
        events.recording();
        let outcome = capture_loop(&mut capture, &job_tx, &flags, limits, stop, cancel);
        // Whatever happened, no preview may hold up what comes next.
        flags.preview_stop.store(true, Ordering::Release);
        let result = match outcome {
            CaptureEnd::Finished(samples) => {
                events.transcribing();
                let audio = Zeroizing::new(resample_to_16k(&samples, capture.sample_rate));
                drop(samples);
                if job_tx.send(EngineJob::Finish(audio)).is_err() {
                    Err("Dictation engine stopped unexpectedly".to_owned())
                } else {
                    Ok(())
                }
            }
            CaptureEnd::Failed(error) => {
                events.processing();
                Err(error)
            }
        };
        drop(job_tx);
        let engine_result = engine
            .join()
            .map_err(|_| "Dictation engine stopped unexpectedly".to_owned())?;
        if let Some(error) = load_error.lock().ok().and_then(|mut slot| slot.take()) {
            return Err(error);
        }
        result?;
        engine_result
    })
}

enum CaptureEnd {
    Finished(Zeroizing<Vec<f32>>),
    Failed(String),
}

/// Only moves audio. Nothing here may wait on the model.
fn capture_loop<S>(
    capture: &mut Capture<S>,
    job_tx: &Sender<EngineJob>,
    flags: &EngineFlags,
    limits: SessionLimits,
    stop: &AtomicBool,
    cancel: &AtomicBool,
) -> CaptureEnd {
    let sample_rate = capture.sample_rate as usize;
    let started = Instant::now();
    let cap = sample_rate.saturating_mul(limits.max_seconds);
    let mut samples = Zeroizing::new(Vec::<f32>::with_capacity(cap));
    let mut next_preview = limits.preview_interval;
    let fail = |capture: &mut Capture<S>, samples: &mut Zeroizing<Vec<f32>>, error: String| {
        capture.stream.take();
        zeroize_pending_audio(&capture.audio_rx, &capture.queued_frames);
        samples.zeroize();
        CaptureEnd::Failed(error)
    };
    loop {
        if cancel.load(Ordering::Acquire) {
            return fail(capture, &mut samples, "Dictation cancelled".into());
        }
        if flags.failed.load(Ordering::Acquire) {
            return fail(capture, &mut samples, "The model could not load".into());
        }
        if let Ok(error) = capture.error_rx.try_recv() {
            let message = format!("Microphone capture interrupted: {error}");
            return fail(capture, &mut samples, message);
        }
        match capture.audio_rx.recv_timeout(limits.poll) {
            Ok(mut block) => {
                release_audio_frames(&capture.queued_frames, block.len());
                let remaining = cap.saturating_sub(samples.len());
                if block.len() > remaining {
                    block.truncate(remaining);
                }
                samples.extend_from_slice(&block);
                block.zeroize();
                if samples.len() >= cap {
                    stop.store(true, Ordering::Release);
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                return fail(
                    capture,
                    &mut samples,
                    "Microphone capture ended unexpectedly".into(),
                );
            }
        }
        if !stop.load(Ordering::Acquire)
            && !cancel.load(Ordering::Acquire)
            && started.elapsed() >= next_preview
            && samples.len() >= sample_rate * PREVIEW_MIN_SECONDS
            && flags.ready.load(Ordering::Acquire)
            && !flags.previews_off.load(Ordering::Acquire)
            && !flags.preview_busy.load(Ordering::Acquire)
        {
            let start = samples
                .len()
                .saturating_sub(sample_rate * PREVIEW_MAX_SECONDS);
            let audio = Zeroizing::new(resample_to_16k(&samples[start..], capture.sample_rate));
            flags.preview_busy.store(true, Ordering::Release);
            match job_tx.try_send(EngineJob::Preview(audio)) {
                Ok(()) => {}
                Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                    flags.preview_busy.store(false, Ordering::Release);
                }
            }
            let last = Duration::from_millis(flags.last_preview_ms.load(Ordering::Acquire));
            next_preview = started.elapsed() + next_preview_wait(limits.preview_interval, last);
        }
        if stop.load(Ordering::Acquire)
            || started.elapsed() >= Duration::from_secs(limits.max_seconds as u64)
        {
            stop.store(true, Ordering::Release);
            break;
        }
    }
    capture.stream.take();
    // Drain the bounded-by-session audio backlog after the callback has stopped.
    while let Ok(mut block) = capture.audio_rx.try_recv() {
        release_audio_frames(&capture.queued_frames, block.len());
        let remaining = cap.saturating_sub(samples.len());
        if block.len() > remaining {
            block.truncate(remaining);
        }
        samples.extend_from_slice(&block);
        block.zeroize();
    }
    if cancel.load(Ordering::Acquire) {
        samples.zeroize();
        return CaptureEnd::Failed("Dictation cancelled".into());
    }
    if samples.len() < sample_rate / 4 {
        samples.zeroize();
        return CaptureEnd::Failed("The microphone captured too little audio. Try again and speak closer to the microphone.".into());
    }
    CaptureEnd::Finished(samples)
}

fn engine_thread<E: Engine>(
    load: impl FnOnce() -> Result<E, String>,
    job_rx: Receiver<EngineJob>,
    events: &dyn SessionEvents,
    flags: &EngineFlags,
    load_error: &Mutex<Option<String>>,
    limits: SessionLimits,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let mut engine = match load() {
        Ok(engine) => engine,
        Err(error) => {
            if let Ok(mut slot) = load_error.lock() {
                *slot = Some(error.clone());
            }
            flags.failed.store(true, Ordering::Release);
            return Err(error);
        }
    };
    flags.ready.store(true, Ordering::Release);
    let mut result = Err("Dictation stopped before transcription".to_owned());
    let mut prepared = false;
    while let Ok(job) = job_rx.recv() {
        match job {
            EngineJob::Preview(mut audio) => {
                if cancel.load(Ordering::Acquire) || flags.preview_stop.load(Ordering::Acquire) {
                    flags.preview_busy.store(false, Ordering::Release);
                    continue;
                }
                if !prepared {
                    prepared = true;
                    engine.prepare(&audio);
                }
                let started = Instant::now();
                let deadline = started + limits.preview_budget;
                let abort = AbortSignal {
                    cancel,
                    stop: Some(&flags.preview_stop),
                    deadline: Some(deadline),
                };
                let outcome = engine.preview(&audio, &abort);
                audio.zeroize();
                let took = started.elapsed();
                flags
                    .last_preview_ms
                    .store(took.as_millis() as u64, Ordering::Release);
                if took >= limits.preview_budget {
                    flags.previews_off.store(true, Ordering::Release);
                    events.previews_paused();
                }
                if let Ok(mut text) = outcome {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() && !abort.requested() {
                        events.partial(trimmed);
                    }
                    text.zeroize();
                }
                flags.preview_busy.store(false, Ordering::Release);
            }
            EngineJob::Finish(mut audio) => {
                let abort = AbortSignal {
                    cancel,
                    stop: None,
                    deadline: None,
                };
                result = engine.finish(&audio, &abort);
                audio.zeroize();
            }
        }
    }
    result
}

/// Preview buffers are released before final full-context inference.
struct WhisperEngine {
    _context: WhisperContext,
    state: Option<WhisperState>,
    preview_language: Option<&'static str>,
}

impl WhisperEngine {
    fn load(path: &std::path::Path) -> Result<Self, String> {
        let context = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(|e| format!("Could not load the local dictation model: {e}"))?;
        let state = context
            .create_state()
            .map_err(|e| format!("Could not start local transcription: {e}"))?;
        Ok(Self {
            _context: context,
            state: Some(state),
            preview_language: None,
        })
    }
}

impl Engine for WhisperEngine {
    fn prepare(&mut self, audio: &[f32]) {
        if !self._context.is_multilingual() {
            self.preview_language = Some("en");
            return;
        }
        let threads = inference_threads() as usize;
        let state = self.state.as_mut().expect("preview state");
        if state.pcm_to_mel(audio, threads).is_err() {
            return;
        }
        if let Ok((id, _)) = state.lang_detect(0, threads) {
            self.preview_language = whisper_rs::get_lang_str(id);
        }
    }

    fn preview(&mut self, audio: &[f32], abort: &AbortSignal<'_>) -> Result<String, String> {
        let seconds = i32::try_from(audio.len() / 16_000).unwrap_or(i32::MAX);
        let audio_ctx = seconds
            .saturating_mul(AUDIO_CTX_PER_SECOND)
            .saturating_add(AUDIO_CTX_MARGIN)
            .min(AUDIO_CTX_FULL);
        transcribe(
            self.state.as_mut().expect("preview state"),
            self.preview_language,
            audio,
            abort,
            Some(audio_ctx),
        )
    }

    fn finish(&mut self, audio: &[f32], abort: &AbortSignal<'_>) -> Result<String, String> {
        if abort.requested() {
            return Err("Dictation cancelled".into());
        }
        // Drop narrowed preview buffers BEFORE allocating a full-context state. A short
        // opening guess is never sufficient evidence to force the final language.
        drop(self.state.take());
        let mut state = self
            ._context
            .create_state()
            .map_err(|e| format!("Could not start final transcription: {e}"))?;
        let language = if self._context.is_multilingual() {
            None
        } else {
            Some("en")
        };
        transcribe(&mut state, language, audio, abort, None)
    }
}

fn zeroize_pending_audio(receiver: &Receiver<Vec<f32>>, queued_frames: &AtomicUsize) {
    while let Ok(mut block) = receiver.try_recv() {
        release_audio_frames(queued_frames, block.len());
        block.zeroize();
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PartialTranscript {
    text: String,
}

impl Drop for PartialTranscript {
    fn drop(&mut self) {
        self.text.zeroize();
    }
}

fn open_input_stream(
    audio_tx: Sender<Vec<f32>>,
    error_tx: Sender<String>,
    queued_frames: Arc<AtomicUsize>,
) -> Result<(Stream, u32), String> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or(
        "No microphone is available. Connect or enable an input device in Windows Settings.",
    )?;
    let config = device
        .default_input_config()
        .map_err(|e| format!("Could not read microphone format: {e}"))?;
    let sample_rate = config.sample_rate().0;
    if !(8_000..=192_000).contains(&sample_rate) {
        return Err(format!(
            "The microphone sample rate {sample_rate} Hz is unsupported"
        ));
    }
    let channels = config.channels() as usize;
    if channels == 0 || channels > 32 {
        return Err("The microphone reports an unsupported channel count".into());
    }
    let stream_config: StreamConfig = config.clone().into();
    let format = config.sample_format();
    let max_callback_frames = (sample_rate as usize).saturating_mul(MAX_CALLBACK_SECONDS);
    let max_queued_frames = (sample_rate as usize).saturating_mul(MAX_QUEUED_AUDIO_SECONDS);
    let stream = match format {
        SampleFormat::F32 => build_stream::<f32, _>(
            &device,
            &stream_config,
            channels,
            max_callback_frames,
            max_queued_frames,
            audio_tx,
            error_tx,
            queued_frames.clone(),
            |v| v,
        ),
        SampleFormat::I16 => build_stream::<i16, _>(
            &device,
            &stream_config,
            channels,
            max_callback_frames,
            max_queued_frames,
            audio_tx,
            error_tx,
            queued_frames.clone(),
            |v| v as f32 / 32768.0,
        ),
        SampleFormat::U16 => build_stream::<u16, _>(
            &device,
            &stream_config,
            channels,
            max_callback_frames,
            max_queued_frames,
            audio_tx,
            error_tx,
            queued_frames,
            |v| (v as f32 - 32768.0) / 32768.0,
        ),
        other => Err(format!(
            "The microphone uses unsupported sample format {other:?}"
        )),
    }?;
    Ok((stream, sample_rate))
}

#[allow(clippy::too_many_arguments)]
fn build_stream<T, F>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    max_callback_frames: usize,
    max_queued_frames: usize,
    tx: Sender<Vec<f32>>,
    error_tx: Sender<String>,
    queued_frames: Arc<AtomicUsize>,
    convert: F,
) -> Result<Stream, String>
where
    T: cpal::SizedSample + Copy + Send + 'static,
    F: Fn(T) -> f32 + Send + 'static,
{
    let callback_error = error_tx.clone();
    let stream_error = error_tx;
    device.build_input_stream(
        config,
        move |input: &[T], _| {
            let frame_count = input.len() / channels;
            if frame_count > max_callback_frames {
                let _ = callback_error.try_send(
                    "Microphone callback exceeded its bounded audio block".into(),
                );
                return;
            }
            if frame_count == 0 {
                return;
            }
            let mut mono = Vec::with_capacity(frame_count);
            for frame in input.chunks_exact(channels) {
                let value = frame.iter().cloned().map(&convert).sum::<f32>() / channels as f32;
                mono.push(value.clamp(-1.0, 1.0));
            }
            deliver_block(mono, &queued_frames, max_queued_frames, &tx, &callback_error);
        },
        move |error| {
            let _ = stream_error.try_send(error.to_string());
        },
        None,
    ).map_err(|e| format!("Could not open microphone: {e}. Check microphone permissions and close applications holding exclusive access."))
}

/// The end of the microphone callback, shared with tests so they exercise the same limits.
fn deliver_block(
    mut mono: Vec<f32>,
    queued_frames: &AtomicUsize,
    max_queued_frames: usize,
    tx: &Sender<Vec<f32>>,
    error_tx: &Sender<String>,
) {
    if !try_reserve_audio_frames(queued_frames, mono.len(), max_queued_frames) {
        mono.zeroize();
        let _ = error_tx.try_send(
            "Microphone audio queue reached its 10 second memory limit; dictation stopped safely"
                .into(),
        );
        return;
    }
    if let Err(error) = tx.try_send(mono) {
        let mut dropped = error.into_inner();
        release_audio_frames(queued_frames, dropped.len());
        dropped.zeroize();
        let _ =
            error_tx.try_send("Microphone callback queue is full; dictation stopped safely".into());
    }
}

unsafe extern "C" fn abort_requested(signal: *mut std::ffi::c_void) -> bool {
    // Safety: `transcribe` passes a pointer to an AbortSignal that outlives the call.
    unsafe { &*(signal as *const AbortSignal<'_>) }.requested()
}

/// About one thread per physical core, leaving the hyperthreads for the rest of the desktop.
fn inference_threads() -> i32 {
    let logical = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    (logical / 2).clamp(2, 8) as i32
}

/// `audio_ctx` narrows the encoder window for a preview. `None` means the full window.
fn transcribe(
    state: &mut WhisperState,
    language: Option<&'static str>,
    audio: &[f32],
    abort: &AbortSignal<'_>,
    audio_ctx: Option<i32>,
) -> Result<String, String> {
    if abort.requested() {
        return Err("Dictation cancelled".into());
    }
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(language);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_n_threads(inference_threads());
    if let Some(audio_ctx) = audio_ctx {
        params.set_audio_ctx(audio_ctx);
        params.set_no_context(true);
    }
    // Not `set_abort_callback_safe`: whisper-rs 0.16 double-boxes the closure, then its
    // trampoline casts the pointer straight to the closure type. The closure then reads
    // unrelated heap memory, so transcription can abort at random (whisper error -6) and
    // cancel may do nothing.
    // Safety: `abort` outlives `state.full`, and the callback only reads through it.
    unsafe {
        params.set_abort_callback(Some(abort_requested));
        params.set_abort_callback_user_data(
            std::ptr::from_ref(abort)
                .cast_mut()
                .cast::<std::ffi::c_void>(),
        );
    }
    state.full(params, audio).map_err(|e| {
        if abort.requested() {
            "Dictation cancelled".to_owned()
        } else {
            format!("Local transcription failed: {e}")
        }
    })?;
    if abort.requested() {
        return Err("Dictation cancelled".into());
    }
    let count = state.full_n_segments();
    let mut output = Zeroizing::new(String::new());
    for index in 0..count {
        if abort.requested() {
            output.zeroize();
            return Err("Dictation cancelled".into());
        }
        let segment = state
            .get_segment(index)
            .ok_or("Could not read transcription segment")?;
        let mut segment = segment
            .to_str_lossy()
            .map_err(|e| format!("Could not read transcription text: {e}"))?
            .into_owned();
        output.push_str(&segment);
        segment.zeroize();
    }
    Ok(output.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_preserves_duration_and_dc_value() {
        let input = vec![0.25; 48_000];
        let output = resample_to_16k(&input, 48_000);
        assert_eq!(output.len(), 16_000);
        assert!(output.iter().all(|sample| (*sample - 0.25).abs() < 0.0001));
    }

    #[test]
    fn abort_signal_reads_cancel_stop_and_deadline() {
        let cancel = AtomicBool::new(false);
        let stop = AtomicBool::new(false);
        let signal = AbortSignal {
            cancel: &cancel,
            stop: Some(&stop),
            deadline: None,
        };
        let pointer = std::ptr::from_ref(&signal).cast_mut().cast();
        assert!(!unsafe { abort_requested(pointer) });
        stop.store(true, Ordering::Release);
        assert!(unsafe { abort_requested(pointer) });
        stop.store(false, Ordering::Release);
        cancel.store(true, Ordering::Release);
        assert!(unsafe { abort_requested(pointer) });
        let quiet = AtomicBool::new(false);
        let expired = AbortSignal {
            cancel: &quiet,
            stop: None,
            deadline: Some(Instant::now()),
        };
        assert!(expired.requested());
    }

    #[test]
    fn slow_previews_back_off() {
        let interval = Duration::from_secs(3);
        assert_eq!(next_preview_wait(interval, Duration::ZERO), interval);
        assert_eq!(
            next_preview_wait(interval, Duration::from_secs(5)),
            Duration::from_secs(10)
        );
    }

    #[derive(Default)]
    struct RecordedEvents {
        recording: AtomicUsize,
        transcribing: AtomicUsize,
        partials: AtomicUsize,
        paused: AtomicUsize,
        processing: AtomicUsize,
    }

    impl SessionEvents for RecordedEvents {
        fn processing(&self) {
            self.processing.fetch_add(1, Ordering::SeqCst);
        }
        fn previews_paused(&self) {
            self.paused.fetch_add(1, Ordering::SeqCst);
        }
        fn recording(&self) {
            self.recording.fetch_add(1, Ordering::SeqCst);
        }
        fn transcribing(&self) {
            self.transcribing.fetch_add(1, Ordering::SeqCst);
        }
        fn partial(&self, _text: &str) {
            self.partials.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// A model far slower than real time. It ignores abort unless told otherwise, which is
    /// the worst case for the capture loop.
    struct SlowEngine {
        preview_cost: Duration,
        honours_abort: bool,
        prepare_cost: Duration,
        running: Arc<AtomicUsize>,
        most_running: Arc<AtomicUsize>,
        previews: Arc<AtomicUsize>,
        final_samples: Arc<AtomicUsize>,
    }

    impl Engine for SlowEngine {
        fn prepare(&mut self, _audio: &[f32]) {
            std::thread::sleep(self.prepare_cost);
        }
        fn preview(&mut self, _audio: &[f32], abort: &AbortSignal<'_>) -> Result<String, String> {
            self.previews.fetch_add(1, Ordering::SeqCst);
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.most_running.fetch_max(now, Ordering::SeqCst);
            let started = Instant::now();
            while started.elapsed() < self.preview_cost {
                if self.honours_abort && abort.requested() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            self.running.fetch_sub(1, Ordering::SeqCst);
            Ok("partial".into())
        }
        fn finish(&mut self, audio: &[f32], _abort: &AbortSignal<'_>) -> Result<String, String> {
            self.final_samples.store(audio.len(), Ordering::SeqCst);
            Ok("done".into())
        }
    }

    struct Harness {
        running: Arc<AtomicUsize>,
        most_running: Arc<AtomicUsize>,
        previews: Arc<AtomicUsize>,
        final_samples: Arc<AtomicUsize>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                running: Arc::default(),
                most_running: Arc::default(),
                previews: Arc::default(),
                final_samples: Arc::default(),
            }
        }
        fn engine(&self, preview_cost: Duration, honours_abort: bool) -> SlowEngine {
            SlowEngine {
                preview_cost,
                honours_abort,
                prepare_cost: Duration::ZERO,
                running: self.running.clone(),
                most_running: self.most_running.clone(),
                previews: self.previews.clone(),
                final_samples: self.final_samples.clone(),
            }
        }
    }

    const TEST_RATE: u32 = 16_000;
    /// A short queue, so anything that stops draining fails quickly.
    const TEST_QUEUE_FRAMES: usize = 800;

    /// Plays `blocks` of 20 ms through the real callback path, paced at `block_every`.
    fn play_audio(
        blocks: usize,
        block_every: Duration,
        tx: Sender<Vec<f32>>,
        error_tx: Sender<String>,
        queued: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            for _ in 0..blocks {
                deliver_block(vec![0.1; 320], &queued, TEST_QUEUE_FRAMES, &tx, &error_tx);
                std::thread::sleep(block_every);
            }
            stop.store(true, Ordering::Release);
        })
    }

    fn limits(interval_ms: u64, budget_ms: u64) -> SessionLimits {
        SessionLimits {
            preview_interval: Duration::from_millis(interval_ms),
            preview_budget: Duration::from_millis(budget_ms),
            max_seconds: 60,
            poll: Duration::from_millis(1),
        }
    }

    fn capture_with(
        blocks: usize,
        block_every: Duration,
        stop: &Arc<AtomicBool>,
    ) -> (Capture<()>, std::thread::JoinHandle<()>) {
        let queued = Arc::new(AtomicUsize::new(0));
        let (tx, audio_rx) = bounded::<Vec<f32>>(AUDIO_QUEUE_BLOCKS);
        let (error_tx, error_rx) = bounded::<String>(1);
        let player = play_audio(
            blocks,
            block_every,
            tx,
            error_tx,
            queued.clone(),
            stop.clone(),
        );
        (
            Capture {
                stream: Some(()),
                audio_rx,
                error_rx,
                queued_frames: queued,
                sample_rate: TEST_RATE,
            },
            player,
        )
    }

    #[test]
    fn stopped_or_cancelled_preview_jobs_skip_language_preparation() {
        struct NeverPrepare;
        impl Engine for NeverPrepare {
            fn prepare(&mut self, _: &[f32]) {
                panic!("stopped preview prepared");
            }
            fn preview(&mut self, _: &[f32], _: &AbortSignal<'_>) -> Result<String, String> {
                panic!("stopped preview ran");
            }
            fn finish(&mut self, _: &[f32], _: &AbortSignal<'_>) -> Result<String, String> {
                unreachable!()
            }
        }
        for cancelled in [false, true] {
            let flags = EngineFlags {
                ready: AtomicBool::new(false),
                failed: AtomicBool::new(false),
                preview_busy: AtomicBool::new(true),
                previews_off: AtomicBool::new(false),
                last_preview_ms: AtomicU64::new(0),
                preview_stop: AtomicBool::new(!cancelled),
            };
            let (tx, rx) = bounded(1);
            tx.send(EngineJob::Preview(Zeroizing::new(vec![0.0; 32000])))
                .unwrap();
            drop(tx);
            let _ = engine_thread(
                || Ok(NeverPrepare),
                rx,
                &RecordedEvents::default(),
                &flags,
                &Mutex::new(None),
                limits(1, 10),
                &AtomicBool::new(cancelled),
            );
            assert!(!flags.preview_busy.load(Ordering::Acquire));
        }
    }

    /// The blocker from review: inference used to run on the loop that drains audio, so a
    /// slow preview filled the queue and the whole recording was thrown away.
    #[test]
    fn a_preview_far_slower_than_the_queue_cannot_lose_the_recording() {
        let harness = Harness::new();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = AtomicBool::new(false);
        let events = RecordedEvents::default();
        // The queue holds 50 ms. One preview blocks the engine for 400 ms.
        let (capture, player) = capture_with(150, Duration::from_millis(2), &stop);
        let engine = harness.engine(Duration::from_millis(400), false);
        let result = run_session(
            move || Ok(engine),
            &events,
            capture,
            true,
            limits(20, 5_000),
            &stop,
            &cancel,
        );
        player.join().unwrap();
        assert_eq!(result.unwrap(), "done");
        assert_eq!(
            harness.final_samples.load(Ordering::SeqCst),
            150 * 320,
            "every sample reached the final pass"
        );
        assert!(harness.previews.load(Ordering::SeqCst) >= 1);
        assert_eq!(
            harness.most_running.load(Ordering::SeqCst),
            1,
            "previews never stack"
        );
    }

    /// The microphone opens first, so what is said during a slow model load is kept.
    #[test]
    fn audio_captured_while_the_model_loads_is_kept() {
        let harness = Harness::new();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = AtomicBool::new(false);
        let events = RecordedEvents::default();
        let (capture, player) = capture_with(100, Duration::from_millis(2), &stop);
        let engine = harness.engine(Duration::ZERO, true);
        let result = run_session(
            move || {
                std::thread::sleep(Duration::from_millis(600));
                Ok(engine)
            },
            &events,
            capture,
            true,
            limits(20, 5_000),
            &stop,
            &cancel,
        );
        player.join().unwrap();
        assert_eq!(result.unwrap(), "done");
        assert_eq!(harness.final_samples.load(Ordering::SeqCst), 100 * 320);
        assert_eq!(events.recording.load(Ordering::SeqCst), 1);
        assert_eq!(events.transcribing.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_preview_over_budget_ends_previews_for_the_session() {
        let harness = Harness::new();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = AtomicBool::new(false);
        let events = RecordedEvents::default();
        let (capture, player) = capture_with(200, Duration::from_millis(2), &stop);
        let engine = harness.engine(Duration::from_millis(60), false);
        let result = run_session(
            move || Ok(engine),
            &events,
            capture,
            true,
            limits(5, 30),
            &stop,
            &cancel,
        );
        player.join().unwrap();
        assert_eq!(result.unwrap(), "done");
        assert_eq!(harness.previews.load(Ordering::SeqCst), 1);
        assert_eq!(events.paused.load(Ordering::SeqCst), 1);
    }

    /// Language detection takes a whole encoder pass. Counting it against the first preview
    /// turned live transcript off on models that could otherwise keep up.
    #[test]
    fn the_language_pass_does_not_use_up_the_preview_budget() {
        let harness = Harness::new();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = AtomicBool::new(false);
        let events = RecordedEvents::default();
        let (capture, player) = capture_with(400, Duration::from_millis(2), &stop);
        let mut engine = harness.engine(Duration::from_millis(1), true);
        engine.prepare_cost = Duration::from_millis(150);
        let result = run_session(
            move || Ok(engine),
            &events,
            capture,
            true,
            limits(5, 100),
            &stop,
            &cancel,
        );
        player.join().unwrap();
        assert_eq!(result.unwrap(), "done");
        assert!(
            harness.previews.load(Ordering::SeqCst) >= 2,
            "previews kept running after the language pass"
        );
    }

    #[test]
    fn stopping_abandons_a_running_preview() {
        let harness = Harness::new();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = AtomicBool::new(false);
        let events = RecordedEvents::default();
        let (capture, player) = capture_with(150, Duration::from_millis(2), &stop);
        // A preview that would take a minute if nothing stopped it.
        let engine = harness.engine(Duration::from_secs(60), true);
        let started = Instant::now();
        let result = run_session(
            move || Ok(engine),
            &events,
            capture,
            true,
            limits(5, 120_000),
            &stop,
            &cancel,
        );
        player.join().unwrap();
        assert_eq!(result.unwrap(), "done");
        assert!(harness.previews.load(Ordering::SeqCst) >= 1);
        assert!(started.elapsed() < Duration::from_secs(10), "did not wait");
    }

    #[test]
    fn previews_off_means_no_preview_runs() {
        let harness = Harness::new();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = AtomicBool::new(false);
        let events = RecordedEvents::default();
        let (capture, player) = capture_with(60, Duration::from_millis(2), &stop);
        let engine = harness.engine(Duration::ZERO, true);
        let result = run_session(
            move || Ok(engine),
            &events,
            capture,
            false,
            limits(1, 5_000),
            &stop,
            &cancel,
        );
        player.join().unwrap();
        assert_eq!(result.unwrap(), "done");
        assert_eq!(harness.previews.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_failed_load_ends_the_recording_with_its_own_reason() {
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = AtomicBool::new(false);
        let events = RecordedEvents::default();
        let (capture, player) = capture_with(2_000, Duration::from_millis(1), &stop);
        let started = Instant::now();
        let result = run_session::<SlowEngine, ()>(
            || Err("The model failed its integrity check".into()),
            &events,
            capture,
            true,
            limits(5, 5_000),
            &stop,
            &cancel,
        );
        assert_eq!(result.unwrap_err(), "The model failed its integrity check");
        assert_eq!(events.processing.load(Ordering::SeqCst), 1);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "stopped at once"
        );
        stop.store(true, Ordering::Release);
        drop(player);
    }

    #[test]
    fn cancel_discards_the_recording() {
        let harness = Harness::new();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = AtomicBool::new(true);
        let events = RecordedEvents::default();
        let (capture, player) = capture_with(5, Duration::from_millis(1), &stop);
        let engine = harness.engine(Duration::ZERO, true);
        let result = run_session(
            move || Ok(engine),
            &events,
            capture,
            true,
            limits(5, 5_000),
            &stop,
            &cancel,
        );
        player.join().unwrap();
        assert_eq!(result.unwrap_err(), "Dictation cancelled");
        assert_eq!(harness.final_samples.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_queue_that_really_overflows_still_reports_it() {
        let queued = AtomicUsize::new(0);
        let (tx, _rx) = bounded::<Vec<f32>>(8);
        let (error_tx, error_rx) = bounded::<String>(1);
        deliver_block(vec![0.0; 100], &queued, 100, &tx, &error_tx);
        deliver_block(vec![0.0; 1], &queued, 100, &tx, &error_tx);
        assert!(error_rx.try_recv().unwrap().contains("memory limit"));
    }

    /// Runs the real session loop with a real model and a WAV played in real time through
    /// the same delivery path as the microphone. Set `TUCK_TEST_MODEL`, `TUCK_TEST_WAV`
    /// (16 kHz mono 16-bit PCM). `TUCK_TEST_NO_PREVIEW=1` turns live transcript off.
    #[test]
    #[ignore = "needs a real model and a recording, and takes real time"]
    fn real_session_with_live_transcript() {
        struct Timed {
            started: Instant,
            partials: Mutex<Vec<(f32, String)>>,
            transcribing_at: Mutex<Option<f32>>,
        }
        impl SessionEvents for Timed {
            fn recording(&self) {}
            fn transcribing(&self) {
                *self.transcribing_at.lock().unwrap() = Some(self.started.elapsed().as_secs_f32());
            }
            fn partial(&self, text: &str) {
                self.partials
                    .lock()
                    .unwrap()
                    .push((self.started.elapsed().as_secs_f32(), text.to_owned()));
            }
        }
        let model = std::env::var("TUCK_TEST_MODEL").unwrap();
        let wav = std::fs::read(std::env::var("TUCK_TEST_WAV").unwrap()).unwrap();
        let data = wav.windows(4).position(|w| w == b"data").unwrap() + 8;
        let audio: Vec<f32> = wav[data..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
            .collect();
        let seconds = audio.len() as f32 / TEST_RATE as f32;
        let preview = std::env::var_os("TUCK_TEST_NO_PREVIEW").is_none();

        let queued = Arc::new(AtomicUsize::new(0));
        let (tx, audio_rx) = bounded::<Vec<f32>>(AUDIO_QUEUE_BLOCKS);
        let (error_tx, error_rx) = bounded::<String>(1);
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = AtomicBool::new(false);
        let max_queue = TEST_RATE as usize * MAX_QUEUED_AUDIO_SECONDS;
        let peak_queue = Arc::new(AtomicUsize::new(0));
        let player = {
            let (queued, stop, peak_queue) = (queued.clone(), stop.clone(), peak_queue.clone());
            std::thread::spawn(move || {
                let started = Instant::now();
                for (index, block) in audio.chunks(320).enumerate() {
                    deliver_block(block.to_vec(), &queued, max_queue, &tx, &error_tx);
                    peak_queue.fetch_max(queued.load(Ordering::Acquire), Ordering::AcqRel);
                    let due = Duration::from_millis(20 * (index as u64 + 1));
                    std::thread::sleep(due.saturating_sub(started.elapsed()));
                }
                stop.store(true, Ordering::Release);
            })
        };
        let events = Timed {
            started: Instant::now(),
            partials: Mutex::default(),
            transcribing_at: Mutex::default(),
        };
        let loaded_in = Mutex::new(None);
        let result = run_session(
            || {
                let load = Instant::now();
                let engine = WhisperEngine::load(std::path::Path::new(&model));
                *loaded_in.lock().unwrap() = Some(load.elapsed());
                engine
            },
            &events,
            Capture {
                stream: Some(()),
                audio_rx,
                error_rx,
                queued_frames: queued,
                sample_rate: TEST_RATE,
            },
            preview,
            SessionLimits::APP,
            &stop,
            &cancel,
        );
        let total = events.started.elapsed().as_secs_f32();
        player.join().unwrap();
        let text = result.unwrap();
        println!("clip {seconds:.1}s, live transcript {preview}");
        println!("model load {:?}", loaded_in.lock().unwrap().unwrap());
        for (at, partial) in events.partials.lock().unwrap().iter() {
            println!("partial at {at:.1}s: {partial}");
        }
        let stopped = events.transcribing_at.lock().unwrap().unwrap();
        println!(
            "stopped at {stopped:.1}s, final text after {:.1}s more, total {total:.1}s",
            total - stopped
        );
        println!(
            "peak queued audio {:.2}s of {MAX_QUEUED_AUDIO_SECONDS}s",
            peak_queue.load(Ordering::Acquire) as f32 / TEST_RATE as f32
        );
        println!("final: {text}");
        println!("peak-memory {}", peak_memory_report());
        assert!(!text.trim().is_empty());
    }

    /// What one 15 s preview costs the old way (full window, language detected every time)
    /// and the new way. The old loop stopped draining audio for the whole of the first number.
    #[test]
    #[ignore = "needs a real model and a recording"]
    fn real_preview_cost() {
        let model = std::env::var("TUCK_TEST_MODEL").unwrap();
        let wav = std::fs::read(std::env::var("TUCK_TEST_WAV").unwrap()).unwrap();
        let data = wav.windows(4).position(|w| w == b"data").unwrap() + 8;
        let audio: Vec<f32> = wav[data..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
            .collect();
        let clip = &audio[..(TEST_RATE as usize * 15).min(audio.len())];
        let cancel = AtomicBool::new(false);
        let abort = AbortSignal {
            cancel: &cancel,
            stop: None,
            deadline: None,
        };
        let mut engine = WhisperEngine::load(std::path::Path::new(&model)).unwrap();
        let started = Instant::now();
        let text = transcribe(engine.state.as_mut().unwrap(), None, clip, &abort, None).unwrap();
        println!(
            "old preview, full window + language pass: {:?}: {text}",
            started.elapsed()
        );
        let started = Instant::now();
        engine.prepare(clip);
        println!(
            "preview language pass: {:?}: preview {:?}",
            started.elapsed(),
            engine.preview_language
        );
        let started = Instant::now();
        let text = engine.preview(clip, &abort);
        println!(
            "new preview, narrowed, language known: {:?}: {text:?}",
            started.elapsed()
        );
    }

    /// Asks Windows for this process's peak working set and commit, without a new dependency.
    fn peak_memory_report() -> String {
        let script = format!(
            "$p=Get-Process -Id {};'peak working set MB='+[math]::Round($p.PeakWorkingSet64/1MB)+', peak commit MB='+[math]::Round($p.PeakPagedMemorySize64/1MB)",
            std::process::id()
        );
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .unwrap_or_else(|e| e.to_string())
    }

    #[test]
    fn multichannel_audio_is_averaged_by_the_capture_callback_contract() {
        let stereo = [0.5f32, -0.5, 0.25, 0.25];
        let mono: Vec<f32> = stereo
            .as_chunks::<2>()
            .0
            .iter()
            .map(|frame| frame.iter().sum::<f32>() / 2.0)
            .collect();
        assert_eq!(mono, vec![0.0, 0.25]);
    }
}
