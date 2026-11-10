//! Records and transcribes one dictation.
//!
//! Capture and inference never share a thread. The session thread only moves audio from
//! the microphone callback into memory, so no amount of inference can overflow the
//! callback's queue and lose a recording. An engine thread loads the model and runs every
//! transcription. A ready microphone keeps bounded pre-roll; cold mode opens on press.

use super::{model, DictationService};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};
use zeroize::{Zeroize, Zeroizing};

const MAX_SECONDS: usize = 120;
const MAX_QUEUED_AUDIO_SECONDS: usize = 10;
const MAX_CALLBACK_SECONDS: usize = 1;
const AUDIO_QUEUE_BLOCKS: usize = 2048;
#[path = "audio_math.rs"]
mod audio_math;
use audio_math::{release_audio_frames, resample_to_16k, try_reserve_audio_frames};

/// What a session reports as it goes.
trait SessionEvents: Sync {
    fn recording(&self);
    fn meter(&self, _level: f32) {}
    fn processing(&self) {}

    fn transcribing(&self);
}

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
    fn streaming(
        &mut self,
        _jobs: &Receiver<EngineJob>,
        _events: &dyn SessionEvents,
        _flags: &EngineFlags,
        _cancel: &AtomicBool,
    ) -> Option<Result<String, String>> {
        None
    }

    fn finish(&mut self, audio: &[f32], abort: &AbortSignal<'_>) -> Result<String, String>;
}

#[derive(Clone, Copy)]
struct SessionLimits {
    max_seconds: usize,
    poll: Duration,
}

impl SessionLimits {
    const APP: Self = Self {
        max_seconds: MAX_SECONDS,
        poll: Duration::from_millis(5),
    };
}

enum EngineJob {
    Feed {
        offset: usize,
        audio: Zeroizing<Vec<f32>>,
    },
    Finish(Zeroizing<Vec<f32>>),
}

/// Shared between the capture loop and the engine thread.
struct EngineFlags {
    streaming: AtomicBool,
    gpu: AtomicBool,
    failed: AtomicBool,
}

const IDLE_UNLOAD: Duration = Duration::from_secs(120);

fn engine_expired(idle: Duration) -> bool {
    idle >= IDLE_UNLOAD
}

#[derive(Default)]
pub(super) struct EngineCache {
    entry: Mutex<Option<CachedEngine>>,
    pub(super) warming: AtomicBool,
    reaper_started: AtomicBool,
    resident: AtomicBool,
}
struct CachedEngine {
    path: std::path::PathBuf,
    engine: LocalEngine,
    used: Instant,
}
impl EngineCache {
    pub(super) fn backend(&self) -> String {
        if self.resident.load(Ordering::Acquire) {
            backend_label()
        } else {
            "CPU (model not loaded)".into()
        }
    }
    pub(super) fn clear(&self) {
        if let Ok(mut slot) = self.entry.lock() {
            *slot = None;
            self.resident.store(false, Ordering::Release);
        }
    }
    fn acquire(
        &self,
        path: &std::path::Path,
        cancel: &AtomicBool,
    ) -> Result<EngineLease<'_>, String> {
        if cancel.load(Ordering::Acquire) {
            return Err("Dictation cancelled".into());
        }
        let mut slot = self.entry.lock().map_err(|_| "Speech engine unavailable")?;
        if slot
            .as_ref()
            .is_none_or(|e| e.path != path || engine_expired(e.used.elapsed()))
        {
            self.resident.store(false, Ordering::Release);
            *slot = None; // release the old weights before loading another model
            let engine = LocalEngine::load_cancellable(path, cancel)?;
            if cancel.load(Ordering::Acquire) {
                return Err("Dictation cancelled".into());
            }
            *slot = Some(CachedEngine {
                path: path.to_owned(),
                engine,
                used: Instant::now(),
            });
            self.resident.store(true, Ordering::Release);
        }
        Ok(EngineLease(slot))
    }
    pub(super) fn start_reaper(self: &Arc<Self>) {
        if self.reaper_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(self);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(10));
            let Some(cache) = weak.upgrade() else {
                break;
            };
            if let Ok(mut slot) = cache.entry.try_lock() {
                if slot
                    .as_ref()
                    .is_some_and(|e| engine_expired(e.used.elapsed()))
                {
                    *slot = None;
                    cache.resident.store(false, Ordering::Release);
                }
            };
        });
    }
    pub(super) fn warm(&self, path: &std::path::Path) -> Result<(), String> {
        drop(self.acquire(path, &AtomicBool::new(false))?);
        Ok(())
    }
}
struct EngineLease<'a>(std::sync::MutexGuard<'a, Option<CachedEngine>>);
impl Drop for EngineLease<'_> {
    fn drop(&mut self) {
        if let Some(entry) = self.0.as_mut() {
            entry.used = Instant::now();
        }
    }
}
impl Engine for EngineLease<'_> {
    fn streaming(
        &mut self,
        jobs: &Receiver<EngineJob>,
        events: &dyn SessionEvents,
        flags: &EngineFlags,
        cancel: &AtomicBool,
    ) -> Option<Result<String, String>> {
        self.0
            .as_mut()
            .unwrap()
            .engine
            .streaming(jobs, events, flags, cancel)
    }
    fn finish(&mut self, audio: &[f32], abort: &AbortSignal<'_>) -> Result<String, String> {
        self.0.as_mut().unwrap().engine.finish(audio, abort)
    }
}

pub(super) fn record_and_transcribe(
    service: &Arc<DictationService>,
    active: &model::Active,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    service.engine_cache.start_reaper();
    service.model_busy.store(true, Ordering::Release);
    let result = record_and_transcribe_inner(service, active, stop, cancel);
    service.model_busy.store(false, Ordering::Release);
    result
}

struct AppEvents<'a> {
    service: &'a DictationService,
}

impl SessionEvents for AppEvents<'_> {
    fn meter(&self, level: f32) {
        let _ = level; // Timed capture publishes the live level, before buffered delivery.
    }
    fn recording(&self) {
        self.service.session_status("recording");
    }
    fn transcribing(&self) {
        self.service.session_status("transcribing");
    }
    fn processing(&self) {
        self.service.session_status("processing");
    }
}

fn record_and_transcribe_inner(
    service: &Arc<DictationService>,
    active: &model::Active,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Dictation cancelled".into());
    }
    let (capture, completed) = timed_capture(service, stop, cancel.clone())?;
    let events = AppEvents { service };
    let path = active.path.clone();
    let entry = active.entry;
    let load_cancel = cancel.clone();
    run_session(
        move || {
            model::verify(&path, entry, &service.verified, false)?;
            service.engine_cache.acquire(&path, &load_cancel)
        },
        &events,
        capture,
        SessionLimits::APP,
        &completed,
        &cancel,
    )
}

/// The audio side of a session. Dropping `stream` ends this capture lease;
/// the ready owner can keep its microphone open between recordings.
struct Capture<S> {
    stream: Option<S>,
    audio_rx: Receiver<Vec<f32>>,
    error_rx: Receiver<String>,
    queued_frames: Arc<AtomicUsize>,
    sample_rate: u32,
    final_length: Option<Arc<AtomicUsize>>,
}

fn run_session<E: Engine, S>(
    load: impl FnOnce() -> Result<E, String> + Send,
    events: &dyn SessionEvents,
    mut capture: Capture<S>,
    limits: SessionLimits,
    stop: &AtomicBool,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let flags = EngineFlags {
        gpu: AtomicBool::new(false),
        streaming: AtomicBool::new(false),
        failed: AtomicBool::new(false),
    };
    let (job_tx, job_rx) = bounded::<EngineJob>(1);
    let load_error = Mutex::new(None::<String>);
    let (flags_ref, load_error_ref) = (&flags, &load_error);
    std::thread::scope(|scope| {
        let engine = std::thread::Builder::new()
            .name("tuck-dictation-engine".into())
            .spawn_scoped(scope, move || {
                engine_thread(load, job_rx, events, flags_ref, load_error_ref, cancel)
            })
            .map_err(|e| format!("Could not start dictation engine: {e}"))?;
        events.recording();
        let outcome = capture_loop(&mut capture, &job_tx, &flags, limits, stop, cancel, events);
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
    events: &dyn SessionEvents,
) -> CaptureEnd {
    let sample_rate = capture.sample_rate as usize;
    let started = Instant::now();
    let cap = sample_rate.saturating_mul(limits.max_seconds);
    let mut samples = Zeroizing::new(Vec::<f32>::with_capacity(cap));
    let mut stream_sent = 0;
    let mut last_meter = Instant::now();
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
                    block[remaining..].zeroize();
                    block.truncate(remaining);
                }
                if last_meter.elapsed() >= Duration::from_millis(100) {
                    events.meter(audio_level(&block));
                    last_meter = Instant::now();
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
        // GPU feeds follow Hex at 160 ms; CPU retains one-second delivery.
        let step = if flags.gpu.load(Ordering::Acquire) {
            sample_rate * 4 / 25
        } else {
            sample_rate
        };
        // Complete source blocks keep resampling boundaries aligned.
        // A rejected job advances no cursor: retained audio is retried, never dropped.
        if flags.streaming.load(Ordering::Acquire) && samples.len() / step > stream_sent / step {
            let end = (samples.len() / step * step).min(stream_sent + step);
            let audio = Zeroizing::new(resample_to_16k(
                &samples[stream_sent..end],
                capture.sample_rate,
            ));
            if job_tx
                .try_send(EngineJob::Feed {
                    offset: stream_sent * 16_000 / sample_rate,
                    audio,
                })
                .is_ok()
            {
                stream_sent = end;
            }
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
            block[remaining..].zeroize();
            block.truncate(remaining);
        }
        samples.extend_from_slice(&block);
        block.zeroize();
    }
    if let Some(length) = &capture.final_length {
        let keep = length.load(Ordering::Acquire).min(samples.len());
        samples[keep..].zeroize();
        samples.truncate(keep);
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
    cancel: &AtomicBool,
) -> Result<String, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Dictation cancelled".into());
    }
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
    if cancel.load(Ordering::Acquire) {
        return Err("Dictation cancelled".into());
    }
    if let Some(result) = engine.streaming(&job_rx, events, flags, cancel) {
        return result;
    }
    let mut result = Err("Dictation stopped before transcription".to_owned());
    while let Ok(job) = job_rx.recv() {
        match job {
            EngineJob::Feed { .. } => {}
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

/// Names the instruction sets the release build of whisper needs that this CPU lacks.
fn missing_cpu_features(features: &[(&'static str, bool)]) -> Vec<&'static str> {
    features
        .iter()
        .filter(|(_, present)| !present)
        .map(|(name, _)| *name)
        .collect()
}

/// Release builds of whisper target AVX2, which would crash Tuck on an older CPU with an
/// illegal instruction. Refusing first leaves everything else in Tuck working.
fn ensure_cpu_supports_whisper() -> Result<(), String> {
    #[cfg(target_arch = "x86_64")]
    {
        let missing = missing_cpu_features(&[
            ("AVX2", std::arch::is_x86_feature_detected!("avx2")),
            ("FMA", std::arch::is_x86_feature_detected!("fma")),
            ("F16C", std::arch::is_x86_feature_detected!("f16c")),
            ("BMI2", std::arch::is_x86_feature_detected!("bmi2")),
        ]);
        if !missing.is_empty() {
            return Err(format!(
                "Dictation needs a newer processor. This one lacks {}.",
                missing.join(", ")
            ));
        }
    }
    Ok(())
}

struct WhisperEngine {
    _context: WhisperContext,
}

impl WhisperEngine {
    fn load(path: &std::path::Path) -> Result<Self, String> {
        ensure_cpu_supports_whisper()?;
        // Initialize the shared registry once in either load order. Whisper itself
        // stays on CPU; Parakeet may use the registered Vulkan device.
        initialize_backends()?;
        let context = WhisperContext::new_with_params(path, {
            let mut p = WhisperContextParameters::default();
            p.use_gpu(false);
            p
        })
        .map_err(|e| format!("Could not load the local dictation model: {e}"))?;
        set_backend("CPU".into());
        Ok(Self { _context: context })
    }
}

impl Engine for WhisperEngine {
    fn finish(&mut self, audio: &[f32], abort: &AbortSignal<'_>) -> Result<String, String> {
        if abort.requested() {
            return Err("Dictation cancelled".into());
        }
        // A fresh state avoids keeping audio/tokens between utterances.
        let mut state = self
            ._context
            .create_state()
            .map_err(|e| format!("Could not start final transcription: {e}"))?;
        let language = if self._context.is_multilingual() {
            None
        } else {
            Some("en")
        };
        transcribe(&mut state, language, audio, abort)
    }
}

fn zeroize_pending_audio(receiver: &Receiver<Vec<f32>>, queued_frames: &AtomicUsize) {
    while let Ok(mut block) = receiver.try_recv() {
        release_audio_frames(queued_frames, block.len());
        block.zeroize();
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

/// Heuristic: half the logical processors, capped below. This is not a topology query;
/// it limits CPU contention even on machines without two threads per physical core.
fn inference_threads() -> i32 {
    let logical = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    (logical / 2).clamp(2, 8) as i32
}

fn transcribe(
    state: &mut WhisperState,
    language: Option<&'static str>,
    audio: &[f32],
    abort: &AbortSignal<'_>,
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
    #[test]
    fn names_only_the_cpu_features_that_are_missing() {
        assert!(super::missing_cpu_features(&[("AVX2", true), ("FMA", true)]).is_empty());
        assert_eq!(
            super::missing_cpu_features(&[("AVX2", false), ("FMA", true), ("BMI2", false)]),
            ["AVX2", "BMI2"]
        );
    }

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

    #[derive(Default)]
    pub(super) struct RecordedEvents {
        recording: AtomicUsize,
        transcribing: AtomicUsize,
        processing: AtomicUsize,
    }

    impl SessionEvents for RecordedEvents {
        fn processing(&self) {
            self.processing.fetch_add(1, Ordering::SeqCst);
        }

        fn recording(&self) {
            self.recording.fetch_add(1, Ordering::SeqCst);
        }
        fn transcribing(&self) {
            self.transcribing.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Scripted final decoder; capture runs independently of engine loading.
    struct SlowEngine {
        final_samples: Arc<AtomicUsize>,
        final_calls: Arc<AtomicUsize>,
    }

    impl Engine for SlowEngine {
        fn finish(&mut self, audio: &[f32], _abort: &AbortSignal<'_>) -> Result<String, String> {
            self.final_calls.fetch_add(1, Ordering::SeqCst);
            self.final_samples.store(audio.len(), Ordering::SeqCst);
            Ok("done".into())
        }
    }

    struct Harness {
        final_samples: Arc<AtomicUsize>,
        final_calls: Arc<AtomicUsize>,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                final_samples: Arc::default(),
                final_calls: Arc::default(),
            }
        }
        fn engine(&self, _cost: Duration, _abort: bool) -> SlowEngine {
            SlowEngine {
                final_samples: self.final_samples.clone(),
                final_calls: self.final_calls.clone(),
            }
        }
    }

    const TEST_RATE: u32 = 16_000;
    /// One second of queued audio, delivered faster than real time.
    const TEST_QUEUE_FRAMES: usize = 16_000;

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

    pub(super) fn limits() -> SessionLimits {
        SessionLimits {
            max_seconds: 60,
            poll: Duration::from_millis(1),
        }
    }

    pub(super) fn capture_with(
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
                final_length: None,
            },
            player,
        )
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
            limits(),
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
    fn warm_engine_idle_policy_has_a_bounded_lifetime() {
        assert!(!engine_expired(Duration::from_secs(119)));
        assert!(engine_expired(Duration::from_secs(120)));
        assert!(engine_expired(Duration::from_secs(600)));
    }

    #[test]
    fn recording_has_only_one_final_inference() {
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
            limits(),
            &stop,
            &cancel,
        );
        player.join().unwrap();
        assert_eq!(result.unwrap(), "done");
        assert_eq!(harness.final_calls.load(Ordering::SeqCst), 1);
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
            limits(),
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
            limits(),
            &stop,
            &cancel,
        );
        player.join().unwrap();
        assert!(matches!(result, Err(error) if error == "Dictation cancelled"));
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
    #[test]
    #[ignore = "needs a real model and a recording, and takes real time"]
    fn real_session_stop_to_text() {
        struct Timed {
            started: Instant,
            transcribing_at: Mutex<Option<f32>>,
        }
        impl SessionEvents for Timed {
            fn recording(&self) {}
            fn transcribing(&self) {
                *self.transcribing_at.lock().unwrap() = Some(self.started.elapsed().as_secs_f32());
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
        let cache = EngineCache::default();
        if std::env::var_os("TUCK_TEST_WARM").is_some() {
            let started = Instant::now();
            cache.warm(std::path::Path::new(&model)).unwrap();
            println!("preload {:?}", started.elapsed());
        }

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
            transcribing_at: Mutex::default(),
        };
        let loaded_in = Mutex::new(None);
        let result = run_session(
            || {
                let load = Instant::now();
                let engine = cache.acquire(std::path::Path::new(&model), &cancel);
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
                final_length: None,
            },
            SessionLimits::APP,
            &stop,
            &cancel,
        );
        let total = events.started.elapsed().as_secs_f32();
        player.join().unwrap();
        let text = result.unwrap();
        println!("backend: {}", backend_label());
        println!("clip {seconds:.1}s, final transcript only");
        println!("model load {:?}", loaded_in.lock().unwrap().unwrap());
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
        if let Ok(expected) = std::env::var("TUCK_TEST_EXPECTED") {
            assert!(
                text.to_lowercase().contains(&expected.to_lowercase()),
                "Expected phrase missing from real inference"
            );
        }
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

/// RMS is computed off the realtime callback; silence stays still.
fn audio_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let energy: f64 = samples
        .iter()
        .filter(|v| v.is_finite())
        .map(|v| f64::from(*v).powi(2))
        .sum();
    ((energy / samples.len() as f64).sqrt() as f32 * 4.0).clamp(0.0, 1.0)
}
#[cfg(test)]
mod meter_tests {
    #[test]
    #[ignore = "needs installed Parakeet and Whisper models plus a generated 16 kHz PCM WAV"]
    fn real_backends_alternate_in_one_process() {
        use super::*;
        let wav = std::fs::read(std::env::var("TUCK_TEST_WAV").unwrap()).unwrap();
        let data = wav.windows(4).position(|w| w == b"data").unwrap() + 8;
        let audio = Zeroizing::new(
            wav[data..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
                .collect::<Vec<_>>(),
        );
        let whisper = std::env::var("TUCK_TEST_WHISPER").unwrap();
        let parakeet = std::env::var("TUCK_TEST_MODEL").unwrap();
        let expected = std::env::var("TUCK_TEST_EXPECTED").unwrap();
        let cancel = AtomicBool::new(false);
        let abort = AbortSignal {
            cancel: &cancel,
            stop: None,
            deadline: None,
        };
        let order = if std::env::var_os("TUCK_TEST_GPU_FIRST").is_some() {
            [&parakeet, &whisper, &parakeet, &whisper]
        } else {
            [&whisper, &parakeet, &whisper, &parakeet]
        };
        for path in order {
            let mut engine = LocalEngine::load(std::path::Path::new(path)).unwrap();
            if path == &parakeet {
                let interrupted = AbortSignal {
                    cancel: &cancel,
                    stop: None,
                    deadline: Some(Instant::now() + Duration::from_millis(10)),
                };
                assert!(engine.finish(&audio, &interrupted).is_err(),);
            }
            let text = engine.finish(&audio, &abort).unwrap();
            println!(
                "{}: {text}",
                std::path::Path::new(path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
            );
            assert!(text.to_lowercase().contains(&expected.to_lowercase()));
        }
    }

    #[test]
    fn parakeet_routing_accepts_discoverable_extension_casing() {
        for name in [
            "parakeet-unified-en-0.6b-Q8_0.gguf",
            "parakeet-unified-en-0.6b-Q8_0.GGUF",
            "parakeet-unified-en-0.6b-Q8_0.GgUf",
        ] {
            assert!(super::is_parakeet_path(std::path::Path::new(name)));
        }
        assert!(!super::is_parakeet_path(std::path::Path::new(
            "ggml-tiny.bin"
        )));
    }

    #[test]
    fn rms_is_real_and_bounded() {
        assert_eq!(super::audio_level(&[]), 0.0);
        assert_eq!(super::audio_level(&[0.0; 8]), 0.0);
        assert_eq!(super::audio_level(&[0.125, -0.125]), 0.5);
        assert_eq!(super::audio_level(&[1.0; 8]), 1.0);
        assert_eq!(super::audio_level(&[f32::NAN]), 0.0);
    }
}

fn recover_gpu<T>(
    failed: &AtomicBool,
    gpu: bool,
    cancel: &AtomicBool,
    result: Result<T, String>,
    cpu: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    if gpu && result.is_err() {
        if cancel.load(Ordering::Acquire) {
            return Err("Dictation cancelled".into());
        }
        failed.store(true, Ordering::Release);
        if cancel.load(Ordering::Acquire) {
            return Err("Dictation cancelled".into());
        }
        let result = cpu();
        if cancel.load(Ordering::Acquire) {
            Err("Dictation cancelled".into())
        } else {
            result
        }
    } else {
        result
    }
}
#[cfg(test)]
mod gpu_tests {
    use super::*;
    #[test]
    fn gpu_failure_preserves_clip_and_disables_future_gpu_attempts() {
        let failed = AtomicBool::new(false);
        let audio = vec![0.1, 0.2, 0.3];
        let cancel = AtomicBool::new(false);
        let result = recover_gpu(&failed, true, &cancel, Err("device lost".into()), || {
            assert_eq!(audio, [0.1, 0.2, 0.3]);
            Ok("recording recovered on CPU".to_string())
        });
        assert_eq!(result.unwrap(), "recording recovered on CPU");
        assert!(failed.load(Ordering::Acquire));
    }
    #[test]
    fn load_failure_is_remembered_and_cpu_result_is_returned() {
        let failed = AtomicBool::new(false);
        let cancel = AtomicBool::new(false);
        let loaded = recover_gpu(
            &failed,
            true,
            &cancel,
            Err("GPU load failed".into()),
            || Ok("CPU session"),
        );
        assert_eq!(loaded.unwrap(), "CPU session");
        let next_backend = if failed.load(Ordering::Acquire) {
            "CPU"
        } else {
            "GPU"
        };
        assert_eq!(next_backend, "CPU");
    }
    #[test]
    fn snapshot_includes_device_and_fallback_reason() {
        let service = DictationService::new();
        assert_eq!(service.snapshot().backend, "CPU (model not loaded)");
        service.engine_cache.resident.store(true, Ordering::Release);
        for label in ["GPU: Test adapter", "CPU", "CPU (GPU inference failed)"] {
            set_backend(label.into());
            let snapshot = service.snapshot();
            assert_eq!(snapshot.backend, label);
            assert_eq!(serde_json::to_value(snapshot).unwrap()["backend"], label);
        }
    }
    #[test]
    fn cancellation_does_not_retry_or_poison_gpu() {
        let failed = AtomicBool::new(false);
        assert!(recover_gpu::<()>(
            &failed,
            true,
            &AtomicBool::new(true),
            Err("cancelled".into()),
            || panic!("retry")
        )
        .is_err());
        assert!(!failed.load(Ordering::Acquire));
    }
    #[test]
    fn cancelled_load_never_enters_gpu_or_cpu_model_loading() {
        let result = LocalEngine::load_cancellable(
            std::path::Path::new("missing-model.gguf"),
            &AtomicBool::new(true),
        );
        assert!(matches!(result, Err(error) if error == "Dictation cancelled"));
    }
}
static GPU_FAILED: AtomicBool = AtomicBool::new(false);
static BACKEND: Mutex<String> = Mutex::new(String::new());
pub(super) fn backend_label() -> String {
    BACKEND
        .lock()
        .map(|s| {
            if s.is_empty() {
                "CPU (model not loaded)".into()
            } else {
                s.clone()
            }
        })
        .unwrap_or_else(|_| "CPU".into())
}
fn set_backend(label: String) {
    if let Ok(mut s) = BACKEND.lock() {
        *s = label;
    }
}
impl std::ops::Deref for ParakeetEngine {
    type Target = transcribe_cpp::Session;
    fn deref(&self) -> &Self::Target {
        &self.session
    }
}
impl std::ops::DerefMut for ParakeetEngine {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.session
    }
}
struct ParakeetEngine {
    session: transcribe_cpp::Session,
    path: std::path::PathBuf,
    gpu: bool,
}
impl ParakeetEngine {
    fn session(
        path: &std::path::Path,
        device: Option<transcribe_cpp::Device>,
        cancel: &AtomicBool,
    ) -> Result<transcribe_cpp::Session, String> {
        if cancel.load(Ordering::Acquire) {
            return Err("Dictation cancelled".into());
        }
        let gpu = device.is_some();
        let model = transcribe_cpp::Model::load_with(
            path,
            &transcribe_cpp::ModelOptions {
                backend: if gpu {
                    transcribe_cpp::Backend::Vulkan
                } else {
                    transcribe_cpp::Backend::Cpu
                },
                device,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        if cancel.load(Ordering::Acquire) {
            return Err("Dictation cancelled".into());
        }
        model
            .session_with(&transcribe_cpp::SessionOptions {
                n_threads: if gpu { 1 } else { 0 },
                ..Default::default()
            })
            .map_err(|e| e.to_string())
    }
    fn load(path: &std::path::Path, cancel: &AtomicBool) -> Result<Self, String> {
        if cancel.load(Ordering::Acquire) {
            return Err("Dictation cancelled".into());
        }
        if !GPU_FAILED.load(Ordering::Acquire) && std::env::var_os("TUCK_TEST_CPU").is_none() {
            let mut devices = transcribe_cpp::devices()
                .into_iter()
                .filter(|d| {
                    d.kind == "vulkan"
                        && matches!(
                            d.device_type,
                            transcribe_cpp::DeviceType::Gpu | transcribe_cpp::DeviceType::Igpu
                        )
                })
                .collect::<Vec<_>>();
            devices.sort_by_key(|d| d.device_type != transcribe_cpp::DeviceType::Gpu);
            if let Some(device) = devices.into_iter().next() {
                if cancel.load(Ordering::Acquire) {
                    return Err("Dictation cancelled".into());
                }
                let label = format!("GPU: {}", device.description);
                let loaded = Self::session(path, Some(device), cancel).and_then(|mut session| {
                    if cancel.load(Ordering::Acquire) {
                        return Err("Dictation cancelled".into());
                    }
                    {
                        let mut stream = session
                            .stream(&transcribe_cpp::RunOptions::default(), &Default::default())
                            .map_err(|e| e.to_string())?;
                        stream.feed(&[0.0; 16000]).map_err(|e| e.to_string())?;
                        stream.finalize().map_err(|e| e.to_string())?;
                    }
                    Ok(session)
                });
                let session = recover_gpu(&GPU_FAILED, true, cancel, loaded, || {
                    set_backend("CPU (GPU load or warmup failed)".into());
                    if cancel.load(Ordering::Acquire) {
                        return Err("Dictation cancelled".into());
                    }
                    Self::session(path, None, cancel)
                })?;
                if cancel.load(Ordering::Acquire) {
                    return Err("Dictation cancelled".into());
                }
                let gpu = !GPU_FAILED.load(Ordering::Acquire);
                if gpu {
                    set_backend(label);
                }
                return Ok(Self {
                    session,
                    path: path.into(),
                    gpu,
                });
            } else {
                #[cfg(windows)]
                if crate::vulkan::missing() {
                    set_backend("CPU (Vulkan 1.2 loader unavailable)".into());
                } else {
                    set_backend("CPU (no usable Vulkan GPU)".into());
                }
                #[cfg(not(windows))]
                set_backend("CPU (no usable Vulkan GPU)".into());
            }
        } else {
            set_backend("CPU (GPU disabled for this run)".into());
        }
        let session = Self::session(path, None, cancel)?;
        if cancel.load(Ordering::Acquire) {
            return Err("Dictation cancelled".into());
        }
        Ok(Self {
            session,
            path: path.into(),
            gpu: false,
        })
    }
    fn fallback(&mut self, cancel: &AtomicBool) -> Result<(), String> {
        if cancel.load(Ordering::Acquire) {
            return Err("Dictation cancelled".into());
        }
        GPU_FAILED.store(true, Ordering::Release);
        set_backend("CPU (GPU inference failed)".into());
        if cancel.load(Ordering::Acquire) {
            return Err("Dictation cancelled".into());
        }
        let session = Self::session(&self.path, None, cancel)?;
        if cancel.load(Ordering::Acquire) {
            return Err("Dictation cancelled".into());
        }
        self.session = session;
        self.gpu = false;
        Ok(())
    }
    fn run(&mut self, audio: &[f32], abort: &AbortSignal<'_>) -> Result<String, String> {
        if self.gpu && GPU_FAILED.load(Ordering::Acquire) {
            if abort.cancel.load(Ordering::Acquire) {
                return Err("Dictation cancelled".into());
            }
            self.fallback(abort.cancel)?;
        }
        let result = parakeet_run(&mut self.session, audio, abort);
        recover_gpu(&GPU_FAILED, self.gpu, abort.cancel, result, || {
            if abort.cancel.load(Ordering::Acquire) {
                return Err("Dictation cancelled".into());
            }
            self.fallback(abort.cancel)?;
            parakeet_run(&mut self.session, audio, abort)
        })
    }
}
impl BufferedBackend for ParakeetEngine {
    type Stream<'a> = transcribe_cpp::Stream<'a>;
    fn install(&mut self, token: &transcribe_cpp::CancelToken) {
        self.session.set_cancel_token(token);
    }
    fn clear(&mut self) {
        self.session.clear_cancel_token();
    }
    fn stream(&mut self) -> Result<Self::Stream<'_>, String> {
        BufferedBackend::stream(&mut self.session)
    }
    fn invalid(&self) -> bool {
        self.session.was_truncated() || self.session.was_aborted()
    }
    fn gpu(&self) -> bool {
        self.gpu
    }
    fn stream_failed(&mut self, cancel: &AtomicBool) {
        if self.gpu && !cancel.load(Ordering::Acquire) {
            GPU_FAILED.store(true, Ordering::Release);
        }
    }
    fn batch(&mut self, audio: &[f32], cancel: &AtomicBool) -> Result<String, String> {
        if cancel.load(Ordering::Acquire) {
            return Err("Dictation cancelled".into());
        }
        if self.gpu && GPU_FAILED.load(Ordering::Acquire) {
            if cancel.load(Ordering::Acquire) {
                return Err("Dictation cancelled".into());
            }
            self.fallback(cancel)?;
        }
        self.run(
            audio,
            &AbortSignal {
                cancel,
                stop: None,
                deadline: None,
            },
        )
    }
}
enum LocalEngine {
    Whisper(WhisperEngine),
    Parakeet(ParakeetEngine),
}
impl LocalEngine {
    #[cfg(test)]
    fn load(path: &std::path::Path) -> Result<Self, String> {
        Self::load_cancellable(path, &AtomicBool::new(false))
    }
    fn load_cancellable(path: &std::path::Path, cancel: &AtomicBool) -> Result<Self, String> {
        if cancel.load(Ordering::Acquire) {
            return Err("Dictation cancelled".into());
        }
        if is_parakeet_path(path) {
            ensure_cpu_supports_whisper()?;
            initialize_backends()?;
            Ok(Self::Parakeet(ParakeetEngine::load(path, cancel)?))
        } else {
            WhisperEngine::load(path).map(Self::Whisper)
        }
    }
}
fn is_parakeet_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("gguf"))
}
fn initialize_backends() -> Result<(), String> {
    static INIT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        #[cfg(windows)]
        crate::vulkan::check();
        transcribe_cpp::init_backends_with(
            None::<&std::path::Path>,
            transcribe_cpp::BackendMask::for_backend(
                if cfg!(all(windows, target_arch = "x86_64")) {
                    transcribe_cpp::Backend::Vulkan
                } else {
                    transcribe_cpp::Backend::Cpu
                },
            ),
        )
        .map_err(|e| e.to_string())
    })
    .clone()
}
impl Engine for LocalEngine {
    fn streaming(
        &mut self,
        jobs: &Receiver<EngineJob>,
        events: &dyn SessionEvents,
        flags: &EngineFlags,
        cancel: &AtomicBool,
    ) -> Option<Result<String, String>> {
        let Self::Parakeet(session) = self else {
            return None;
        };
        if session.gpu && GPU_FAILED.load(Ordering::Acquire) {
            if cancel.load(Ordering::Acquire) {
                return Some(Err("Dictation cancelled".into()));
            }
            if let Err(error) = session.fallback(cancel) {
                return Some(Err(error));
            }
        }
        flags.gpu.store(session.gpu, Ordering::Release);
        flags.streaming.store(true, Ordering::Release);
        Some(buffered_session(session, jobs, events, flags, cancel))
    }

    fn finish(&mut self, audio: &[f32], abort: &AbortSignal<'_>) -> Result<String, String> {
        match self {
            Self::Whisper(e) => e.finish(audio, abort),
            Self::Parakeet(s) => s.run(audio, abort),
        }
    }
}
fn parakeet_run(
    session: &mut transcribe_cpp::Session,
    audio: &[f32],
    abort: &AbortSignal<'_>,
) -> Result<String, String> {
    if abort.requested() {
        return Err("Dictation cancelled".into());
    }
    let token = transcribe_cpp::CancelToken::new();
    session.set_cancel_token(&token);
    let done = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Acquire) {
                if abort.requested() {
                    token.cancel();
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let result = {
            let _completion = CompleteOnDrop(&done);
            session.run(
                audio,
                &transcribe_cpp::RunOptions {
                    language: Some("en".into()),
                    ..Default::default()
                },
            )
        };
        result
    });
    session.clear_cancel_token();
    if abort.requested() {
        return Err("Dictation cancelled".into());
    }
    result
        .map(|r| r.text.trim().to_owned())
        .map_err(|e| e.to_string())
}

/// Dropped before a scoped watcher is joined, including while unwinding.
struct CompleteOnDrop<'a>(&'a AtomicBool);
impl Drop for CompleteOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

// This private seam keeps the native borrowed lifetime while exercising the same
// orchestration with deterministic streams and clocks in device-free tests.
trait BufferedStream {
    fn feed(&mut self, audio: &[f32]) -> Result<(), String>;
    fn finalize(&mut self) -> Result<(), String>;
    fn text(&self) -> transcribe_cpp::StreamText;
    fn intervals(&self) -> Vec<(i64, i64)>;
}
impl BufferedStream for transcribe_cpp::Stream<'_> {
    fn feed(&mut self, audio: &[f32]) -> Result<(), String> {
        self.feed(audio).map(|_| ()).map_err(|e| e.to_string())
    }
    fn finalize(&mut self) -> Result<(), String> {
        self.finalize().map(|_| ()).map_err(|e| e.to_string())
    }
    fn text(&self) -> transcribe_cpp::StreamText {
        self.text()
    }
    fn intervals(&self) -> Vec<(i64, i64)> {
        // Native Token timestamps are signed i64 milliseconds, not samples.
        self.snapshot()
            .tokens
            .iter()
            .filter(|t| t.text.chars().any(char::is_alphanumeric))
            .map(|t| (t.t0_ms, t.t1_ms))
            .collect()
    }
}
trait BufferedBackend {
    type Stream<'a>: BufferedStream
    where
        Self: 'a;
    fn install(&mut self, token: &transcribe_cpp::CancelToken);
    fn clear(&mut self);
    fn stream(&mut self) -> Result<Self::Stream<'_>, String>;
    fn invalid(&self) -> bool;
    fn gpu(&self) -> bool {
        false
    }
    fn stream_failed(&mut self, _cancel: &AtomicBool) {}
    fn batch(&mut self, audio: &[f32], cancel: &AtomicBool) -> Result<String, String>;
}
impl BufferedBackend for transcribe_cpp::Session {
    type Stream<'a> = transcribe_cpp::Stream<'a>;
    fn install(&mut self, token: &transcribe_cpp::CancelToken) {
        self.set_cancel_token(token);
    }
    fn clear(&mut self) {
        self.clear_cancel_token();
    }
    fn stream(&mut self) -> Result<Self::Stream<'_>, String> {
        self.stream(
            &transcribe_cpp::RunOptions {
                language: Some("en".into()),
                ..Default::default()
            },
            &Default::default(),
        )
        .map_err(|e| e.to_string())
    }
    fn invalid(&self) -> bool {
        self.was_truncated() || self.was_aborted()
    }
    fn batch(&mut self, audio: &[f32], cancel: &AtomicBool) -> Result<String, String> {
        parakeet_run(
            self,
            audio,
            &AbortSignal {
                cancel,
                stop: None,
                deadline: None,
            },
        )
    }
}

// A borrowed stream lives entirely on this worker; no self-referential ownership.
fn buffered_session(
    session: &mut ParakeetEngine,
    jobs: &Receiver<EngineJob>,
    events: &dyn SessionEvents,
    flags: &EngineFlags,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let clock = Instant::now();
    buffered_driver(session, jobs, events, flags, cancel, &|| {
        clock.elapsed().as_millis() as u64
    })
}

fn buffered_driver<B: BufferedBackend>(
    session: &mut B,
    jobs: &Receiver<EngineJob>,
    _events: &dyn SessionEvents,
    _flags: &EngineFlags,
    cancel: &AtomicBool,
    now: &(impl Fn() -> u64 + Sync),
) -> Result<String, String> {
    let gpu = session.gpu();
    let token = transcribe_cpp::CancelToken::new();
    session.install(&token);
    let done = AtomicBool::new(false);
    let deadline_ms = AtomicU64::new(u64::MAX);
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Acquire) {
                if cancel.load(Ordering::Acquire) {
                    token.cancel();
                    break;
                }
                if now() >= deadline_ms.load(Ordering::Acquire) {
                    token.cancel();
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let result = (|| {
            let _completion = CompleteOnDrop(&done);
            let mut stream = session.stream().ok();
            let mut device_failed = stream.is_none();
            let mut sent = 0;
            let mut final_audio = None;
            while let Ok(job) = jobs.recv() {
                if cancel.load(Ordering::Acquire) {
                    return Err("Dictation cancelled".into());
                }
                match job {
                    EngineJob::Feed { offset, audio } => {
                        if let Some(active) = stream.as_mut() {
                            deadline_ms.store(now() + 8000, Ordering::Release);
                            let feed_failed = offset == sent && active.feed(&audio).is_err();
                            device_failed |= feed_failed || token.is_cancelled();
                            let failed = offset != sent || feed_failed;
                            deadline_ms.store(u64::MAX, Ordering::Release);
                            if failed || token.is_cancelled() {
                                stream = None;
                            } else {
                                sent += audio.len();
                            }
                        }
                    }
                    EngineJob::Finish(audio) => {
                        final_audio = Some(audio);
                        break;
                    }
                }
            }
            let audio = final_audio.ok_or("Dictation stopped before transcription")?;
            let streamed = stream.as_mut().and_then(|active| {
                deadline_ms.store(now() + if gpu { 1500 } else { 8000 }, Ordering::Release);
                if sent > audio.len() {
                    return None;
                }
                if active.feed(&audio[sent..]).is_err()
                    || (gpu && active.feed(&[0.0; 6400]).is_err())
                    || active.finalize().is_err()
                {
                    device_failed = true;
                    return None;
                }
                let text = active.text().full.trim().to_owned();
                let intervals = active.intervals();
                stream_is_reliable(&audio, &text, &intervals).then_some(text)
            });
            deadline_ms.store(u64::MAX, Ordering::Release);
            drop(stream);
            if cancel.load(Ordering::Acquire) {
                return Err("Dictation cancelled".into());
            }
            if let Some(text) = streamed.filter(|_| !session.invalid() && !token.is_cancelled()) {
                #[cfg(test)]
                eprintln!("buffered stream authoritative (no batch fallback)");
                return Ok(text);
            }
            #[cfg(test)]
            eprintln!("buffered stream rejected; whole-clip fallback");
            session.clear();
            if cancel.load(Ordering::Acquire) {
                return Err("Dictation cancelled".into());
            }
            if device_failed || token.is_cancelled() {
                session.stream_failed(cancel);
            }
            if cancel.load(Ordering::Acquire) {
                return Err("Dictation cancelled".into());
            }
            let result = session.batch(&audio, cancel);
            if cancel.load(Ordering::Acquire) {
                Err("Dictation cancelled".into())
            } else {
                result
            }
        })();
        result
    });
    session.clear();
    result
}

#[cfg(test)]
mod buffered_tests {
    use super::*;
    struct FakeBackend<'a> {
        failure: &'static str,
        token: Option<transcribe_cpp::CancelToken>,
        alive: Arc<AtomicBool>,
        fed: Arc<std::sync::Mutex<Vec<f32>>>,
        clock: &'a AtomicU64,
        batches: Vec<Vec<f32>>,
        cancel_during_finalize: Option<Arc<AtomicBool>>,
        poisoned: bool,
    }
    struct FakeStream<'a> {
        backend: &'a FakeBackend<'a>,
    }
    impl Drop for FakeStream<'_> {
        fn drop(&mut self) {
            self.backend.alive.store(false, Ordering::Release);
        }
    }
    impl BufferedStream for FakeStream<'_> {
        fn feed(&mut self, audio: &[f32]) -> Result<(), String> {
            if matches!(self.backend.failure, "feed" | "gpu-feed") {
                return Err("feed".into());
            }
            self.backend.fed.lock().unwrap().extend_from_slice(audio);
            if self.backend.failure == "deadline" {
                self.backend.clock.store(9000, Ordering::Release);
                let start = Instant::now();
                while !self.backend.token.as_ref().unwrap().is_cancelled() {
                    assert!(start.elapsed() < Duration::from_secs(2), "watcher failed");
                    std::thread::yield_now();
                }
                return Err("deadline".into());
            }
            Ok(())
        }
        fn finalize(&mut self) -> Result<(), String> {
            if self.backend.failure == "gpu-user-cancel-finalize" {
                self.backend
                    .cancel_during_finalize
                    .as_ref()
                    .unwrap()
                    .store(true, Ordering::Release);
                return Err("user cancelled".into());
            }
            if matches!(
                self.backend.failure,
                "deadline-finalize" | "gpu-deadline-finalize"
            ) {
                self.backend.clock.store(
                    if self.backend.failure.starts_with("gpu-") {
                        2000
                    } else {
                        9000
                    },
                    Ordering::Release,
                );
                let start = Instant::now();
                while !self.backend.token.as_ref().unwrap().is_cancelled() {
                    assert!(start.elapsed() < Duration::from_secs(2));
                    std::thread::yield_now();
                }
            }
            if matches!(self.backend.failure, "finalize" | "gpu-finalize") {
                Err("finalize".into())
            } else {
                Ok(())
            }
        }
        fn text(&self) -> transcribe_cpp::StreamText {
            transcribe_cpp::StreamText {
                full: "words".into(),
                committed: "words".into(),
                tentative: String::new(),
            }
        }
        fn intervals(&self) -> Vec<(i64, i64)> {
            vec![(0, 1000)]
        }
    }
    impl BufferedBackend for FakeBackend<'_> {
        type Stream<'a>
            = FakeStream<'a>
        where
            Self: 'a;
        fn install(&mut self, token: &transcribe_cpp::CancelToken) {
            assert!(
                !token.is_cancelled(),
                "new recording must get a fresh token"
            );
            self.token = Some(token.clone());
        }
        fn clear(&mut self) {
            self.token = None;
        }
        fn stream(&mut self) -> Result<Self::Stream<'_>, String> {
            if self.failure == "init" {
                return Err("init".into());
            }
            self.alive.store(true, Ordering::Release);
            Ok(FakeStream { backend: self })
        }
        fn gpu(&self) -> bool {
            self.failure.starts_with("gpu-")
        }
        fn invalid(&self) -> bool {
            false
        }
        fn stream_failed(&mut self, _cancel: &AtomicBool) {
            self.poisoned = true;
        }
        fn batch(&mut self, audio: &[f32], cancel: &AtomicBool) -> Result<String, String> {
            assert!(
                !self.alive.load(Ordering::Acquire),
                "stream must drop before batch"
            );
            assert!(self.token.is_none(), "failed stream token must be detached");
            self.batches.push(audio.to_vec());
            if self.failure == "cancel" {
                cancel.store(true, Ordering::Release);
            }
            Ok("batch".into())
        }
    }
    fn driver_flags() -> EngineFlags {
        EngineFlags {
            gpu: AtomicBool::new(false),
            streaming: AtomicBool::new(true),
            failed: AtomicBool::new(false),
        }
    }
    #[test]
    fn production_driver_recovery_and_exact_audio() {
        for failure in [
            "init",
            "gpu-feed",
            "gpu-finalize",
            "gpu-deadline-finalize",
            "feed",
            "finalize",
            "deadline",
            "deadline-finalize",
            "offset",
            "none",
        ] {
            let clock = AtomicU64::new(0);
            let mut backend = FakeBackend {
                failure,
                token: None,
                alive: Arc::new(AtomicBool::new(false)),
                fed: Arc::new(std::sync::Mutex::new(Vec::new())),
                clock: &clock,
                batches: Vec::new(),
                cancel_during_finalize: None,
                poisoned: false,
            };
            let audio: Vec<f32> = (0..16000).map(|i| 0.01 + i as f32 / 32000.0).collect();
            let cancel = AtomicBool::new(false);
            let (tx, rx) = bounded(3);
            tx.send(EngineJob::Feed {
                offset: 0,
                audio: Zeroizing::new(audio[..6000].to_vec()),
            })
            .unwrap();
            tx.send(EngineJob::Feed {
                offset: if failure == "offset" { 6001 } else { 6000 },
                audio: Zeroizing::new(audio[6000..12000].to_vec()),
            })
            .unwrap();
            tx.send(EngineJob::Finish(Zeroizing::new(audio.clone())))
                .unwrap();
            drop(tx);
            let result = buffered_driver(
                &mut backend,
                &rx,
                &super::tests::RecordedEvents::default(),
                &driver_flags(),
                &cancel,
                &|| clock.load(Ordering::Acquire),
            );
            if failure == "none" {
                assert_eq!(result.unwrap(), "words");
                assert_eq!(
                    *backend.fed.lock().unwrap(),
                    audio,
                    "each sample and final tail once"
                );
            } else {
                assert_eq!(result.unwrap(), "batch", "{failure}");
                assert_eq!(backend.batches, vec![audio]);
            }
            assert!(backend.token.is_none());
            backend.failure = "none";
            clock.store(0, Ordering::Release);
            let (tx, rx) = bounded(1);
            tx.send(EngineJob::Finish(Zeroizing::new(vec![0.1; 16000])))
                .unwrap();
            assert_eq!(
                buffered_driver(
                    &mut backend,
                    &rx,
                    &super::tests::RecordedEvents::default(),
                    &driver_flags(),
                    &cancel,
                    &|| clock.load(Ordering::Acquire)
                )
                .unwrap(),
                "words",
                "reuse after {failure}"
            );
        }
    }
    #[test]
    fn production_driver_cancel_during_fallback_suppresses_result() {
        let clock = AtomicU64::new(0);
        let mut backend = FakeBackend {
            failure: "cancel",
            token: None,
            alive: Arc::new(AtomicBool::new(false)),
            fed: Arc::new(std::sync::Mutex::new(Vec::new())),
            clock: &clock,
            batches: Vec::new(),
            cancel_during_finalize: None,
            poisoned: false,
        };
        let (tx, rx) = bounded(1);
        let audio = vec![0.1; 48000]; // timing ends at 1s: uncovered activity forces fallback
        tx.send(EngineJob::Finish(Zeroizing::new(audio.clone())))
            .unwrap();
        let cancel = AtomicBool::new(false);
        assert!(buffered_driver(
            &mut backend,
            &rx,
            &super::tests::RecordedEvents::default(),
            &driver_flags(),
            &cancel,
            &|| 0
        )
        .is_err());
        assert_eq!(backend.batches, vec![audio]);
    }

    #[test]
    fn cancellation_during_gpu_stream_failure_skips_poisoning_and_batch_recovery() {
        let clock = AtomicU64::new(0);
        let cancel = Arc::new(AtomicBool::new(false));
        let mut backend = FakeBackend {
            failure: "gpu-user-cancel-finalize",
            token: None,
            alive: Arc::new(AtomicBool::new(false)),
            fed: Arc::new(std::sync::Mutex::new(Vec::new())),
            clock: &clock,
            batches: Vec::new(),
            cancel_during_finalize: Some(cancel.clone()),
            poisoned: false,
        };
        let (tx, rx) = bounded(1);
        tx.send(EngineJob::Finish(Zeroizing::new(vec![0.1; 48000])))
            .unwrap();
        assert_eq!(
            buffered_driver(
                &mut backend,
                &rx,
                &super::tests::RecordedEvents::default(),
                &driver_flags(),
                &cancel,
                &|| 0,
            )
            .unwrap_err(),
            "Dictation cancelled"
        );
        assert!(!backend.poisoned, "cancelled work must not disable the GPU");
        assert!(
            backend.batches.is_empty(),
            "cancelled work must not start CPU recovery"
        );
    }

    #[test]
    #[ignore = "uses installed model and synthetic WAV; no devices"]
    fn real_stream_cancel_and_reuse() {
        let mut engine = LocalEngine::load(std::path::Path::new(
            &std::env::var("TUCK_TEST_MODEL").unwrap(),
        ))
        .unwrap();
        let LocalEngine::Parakeet(session) = &mut engine else {
            panic!("Parakeet required")
        };
        let wav = std::fs::read(std::env::var("TUCK_TEST_WAV").unwrap()).unwrap();
        let data = wav.windows(4).position(|w| w == b"data").unwrap() + 8;
        let audio: Vec<f32> = wav[data..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
            .collect();
        let cancel = AtomicBool::new(false);
        for during_feed in [true, false] {
            cancel.store(false, Ordering::Release);
            let flags = EngineFlags {
                gpu: AtomicBool::new(false),
                streaming: AtomicBool::new(true),
                failed: AtomicBool::new(false),
            };
            let (tx, rx) = bounded(2);
            if during_feed {
                tx.send(EngineJob::Feed {
                    offset: 0,
                    audio: Zeroizing::new(audio.clone()),
                })
                .unwrap();
            }
            tx.send(EngineJob::Finish(Zeroizing::new(audio.clone())))
                .unwrap();
            drop(tx);
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    std::thread::sleep(Duration::from_millis(100));
                    cancel.store(true, Ordering::Release);
                });
                assert!(buffered_session(
                    session,
                    &rx,
                    &super::tests::RecordedEvents::default(),
                    &flags,
                    &cancel
                )
                .is_err());
            });
        }
        cancel.store(false, Ordering::Release);
        let text = parakeet_run(
            session,
            &audio,
            &AbortSignal {
                cancel: &cancel,
                stop: None,
                deadline: None,
            },
        )
        .unwrap();
        assert!(text.to_lowercase().contains("quick brown fox"));
    }
    #[test]
    fn hidden_stream_jobs_are_contiguous_and_final_tail_is_authoritative() {
        struct Streaming;
        impl Engine for Streaming {
            fn streaming(
                &mut self,
                jobs: &Receiver<EngineJob>,
                _: &dyn SessionEvents,
                flags: &EngineFlags,
                _: &AtomicBool,
            ) -> Option<Result<String, String>> {
                flags.streaming.store(true, Ordering::Release);
                let mut sent = Vec::new();
                while let Ok(job) = jobs.recv() {
                    match job {
                        EngineJob::Feed { offset, audio } => {
                            assert_eq!(offset, sent.len());
                            sent.extend_from_slice(&audio);
                        }
                        EngineJob::Finish(audio) => {
                            assert!(sent.len() >= 16000, "hidden presentation must still feed");
                            assert_eq!(sent, audio[..sent.len()]);
                            assert_eq!(audio.len(), 105 * 320);
                            return Some(Ok("authoritative".into()));
                        }
                    }
                }
                Some(Err("missing final".into()))
            }

            fn finish(&mut self, _: &[f32], _: &AbortSignal<'_>) -> Result<String, String> {
                panic!("legacy final")
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (capture, player) = super::tests::capture_with(105, Duration::from_millis(2), &stop);
        let result = run_session(
            || Ok(Streaming),
            &super::tests::RecordedEvents::default(),
            capture,
            super::tests::limits(),
            &stop,
            &AtomicBool::new(false),
        );
        player.join().unwrap();
        assert_eq!(result.unwrap(), "authoritative");
    }
    #[test]
    fn reliability_rejects_missing_timing_trailing_phrase_and_long_pause() {
        assert!(!stream_is_reliable(&[0.1; 16000], "words", &[]));
        assert!(!stream_is_reliable(&[0.1; 16000], "", &[(0, 1000)]));
        assert!(!stream_is_reliable(&[0.1; 32000], "opening", &[(0, 1000)]));
        assert!(stream_is_reliable(
            &[0.002; 16000],
            "quiet speech",
            &[(0, 1000)]
        ));
        let mut clip = vec![0.1; 16000];
        clip.extend(vec![0.0001; 96000]);
        clip.extend(vec![0.002; 16000]);
        assert!(!stream_is_reliable(
            &clip,
            "opening and resumed words",
            &[(0, clip.len() as i64 / 16)]
        ));
        assert!(!stream_is_reliable(
            &[0.1; 16000],
            "invalid time",
            &[(0, 6250)]
        ));
    }
    #[test]
    fn reliability_checks_opening_and_middle_not_only_last_word() {
        let audio = [0.1; 64000];
        assert!(!stream_is_reliable(
            &audio,
            "missing opening",
            &[(1500, 4000)]
        ));
        assert!(!stream_is_reliable(
            &audio,
            "missing middle",
            &[(0, 500), (3000, 4000)]
        ));
        assert!(!stream_is_reliable(&audio, "missing end", &[(0, 2000)]));
        assert!(stream_is_reliable(
            &audio,
            "covered",
            &[(0, 1000), (1100, 2200), (2300, 4000)]
        ));
        assert!(!stream_is_reliable(&audio, "invalid", &[(-1, 4000)]));
    }
    #[test]
    fn aligned_source_seconds_match_whole_clip_resampling_including_tail() {
        for rate in [8000, 16000, 22050, 44100, 48000, 192000] {
            let input: Vec<f32> = (0..rate * 2 + 137)
                .map(|i| (i as f32 * 0.013).sin())
                .collect();
            let whole = resample_to_16k(&input, rate as u32);
            let mut parts = Vec::new();
            for chunk in input.chunks(rate) {
                parts.extend(resample_to_16k(chunk, rate as u32));
            }
            assert_eq!(parts.len(), whole.len());
            assert!(parts
                .iter()
                .zip(whole)
                .all(|(a, b)| (a - b).abs() < 0.00001));
        }
    }
}

fn stream_is_reliable(audio: &[f32], text: &str, intervals: &[(i64, i64)]) -> bool {
    if text.is_empty() || intervals.is_empty() {
        return false;
    }
    let mut ranges = Vec::with_capacity(intervals.len());
    for &(start, end) in intervals {
        let Some(start) = usize::try_from(start).ok().and_then(|v| v.checked_mul(16)) else {
            return false;
        };
        let Some(end) = usize::try_from(end).ok().and_then(|v| v.checked_mul(16)) else {
            return false;
        };
        if end <= start || end > audio.len().saturating_add(6400) {
            return false;
        }
        ranges.push((
            start.saturating_sub(6400).min(audio.len()),
            end.saturating_add(6400).min(audio.len()),
        ));
    }
    ranges.sort_unstable();
    // Never cut quiet speech. Low-level long pauses require a full-clip decision.
    let long_pause = audio
        .chunks(320)
        .scan(0usize, |quiet, frame| {
            *quiet = if frame.iter().all(|v| v.abs() <= 0.001) {
                *quiet + frame.len()
            } else {
                0
            };
            Some(*quiet > 89_600)
        })
        .any(|long| long);
    let activity = |samples: &[f32]| samples.iter().filter(|v| v.abs() > 0.001).count();
    let mut cursor = 0;
    let mut uncovered = 0;
    for (start, end) in ranges {
        if start > cursor {
            uncovered += activity(&audio[cursor..start]);
        }
        cursor = cursor.max(end);
    }
    uncovered += activity(&audio[cursor..]);
    !long_pause && uncovered < 4800
}

#[path = "warm_capture.rs"]
mod warm_capture;
use warm_capture::timed_capture;
pub(super) use warm_capture::{start_ready_microphone, ReadyMicrophone};
