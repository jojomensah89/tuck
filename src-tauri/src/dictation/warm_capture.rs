//! Warm microphone ownership and timestamped clip boundaries.

use super::*;

/// The owner alone holds cpal's stream. Idle audio is bounded and erased on eviction.
#[derive(Default)]
pub(crate) struct ReadyMicrophone {
    state: Mutex<ReadyState>,
    closed: AtomicBool,
    started: AtomicBool,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}
#[derive(Default)]
struct ReadyState {
    blocks: std::collections::VecDeque<(Instant, Zeroizing<Vec<f32>>)>,
    rate: u32,
    error: Option<String>,
}
impl ReadyState {
    fn prune(&mut self, now: Instant) {
        while self
            .blocks
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) >= Duration::from_secs(10))
        {
            self.blocks.pop_front();
        }
        let mut total: usize = self.blocks.iter().map(|(_, block)| block.len()).sum();
        while total > self.rate as usize * 10 {
            if let Some((_, block)) = self.blocks.pop_front() {
                total -= block.len();
            } else {
                break;
            }
        }
    }
}

impl ReadyMicrophone {
    pub(crate) fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
        self.state.lock().unwrap().blocks.clear();
    }
}

struct ReadyInput {
    stream: Option<Stream>,
    rx: Receiver<Vec<f32>>,
    erx: Receiver<String>,
    frames: Arc<AtomicUsize>,
}
impl Drop for ReadyInput {
    fn drop(&mut self) {
        self.stream.take();
        zeroize_pending_audio(&self.rx, &self.frames);
    }
}

pub(crate) fn start_ready_microphone(service: &Arc<DictationService>) {
    if service.microphone.started.swap(true, Ordering::AcqRel) {
        return;
    }
    let weak = Arc::downgrade(service);
    let owner = service.microphone.clone();
    let worker = std::thread::spawn(move || {
        let mut input: Option<ReadyInput> = None;
        let mut device_name = None;
        let mut checked = Instant::now() - Duration::from_secs(2);
        let mut settings_checked = checked;
        let mut enabled = false;
        loop {
            if owner.closed.load(Ordering::Acquire) {
                break;
            }
            if settings_checked.elapsed() >= Duration::from_millis(250) {
                let Some(service) = weak.upgrade() else {
                    break;
                };
                enabled = service.settings().keep_microphone_ready
                    && service.resolve_model().active.is_some();
                settings_checked = Instant::now();
            }
            if !enabled {
                input = None;
                {
                    let mut state = owner.state.lock().unwrap();
                    state.blocks.clear();
                    state.rate = 0;
                }
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            if checked.elapsed() >= Duration::from_secs(1) {
                checked = Instant::now();
                let current = default_input_id();
                if current != device_name {
                    input = None;
                    device_name = current;
                }
                if input.is_none() {
                    let frames = Arc::new(AtomicUsize::new(0));
                    let (tx, rx) = bounded(AUDIO_QUEUE_BLOCKS);
                    let (etx, erx) = bounded(1);
                    match open_input_stream(tx, etx, frames.clone()).and_then(|(s, r)| {
                        s.play().map_err(|e| e.to_string())?;
                        Ok((s, r))
                    }) {
                        Ok((stream, rate)) => {
                            let mut state = owner.state.lock().unwrap();
                            state.blocks.clear();
                            state.rate = rate;
                            state.error = None;
                            input = Some(ReadyInput {
                                stream: Some(stream),
                                rx,
                                erx,
                                frames,
                            });
                        }
                        Err(e) => owner.state.lock().unwrap().error = Some(e),
                    }
                }
            }
            if let Some(ReadyInput {
                rx, erx, frames, ..
            }) = &input
            {
                if let Ok(e) = erx.try_recv() {
                    owner.state.lock().unwrap().error = Some(e);
                    input = None;
                    continue;
                }
                if let Ok(block) = rx.recv_timeout(Duration::from_millis(5)) {
                    release_audio_frames(frames, block.len());
                    let mut state = owner.state.lock().unwrap();
                    let now = Instant::now();
                    let arrived =
                        now - Duration::from_secs_f64(block.len() as f64 / state.rate as f64);
                    let start = state
                        .blocks
                        .back()
                        .map(|(at, previous)| {
                            *at + Duration::from_secs_f64(previous.len() as f64 / state.rate as f64)
                        })
                        .map_or(arrived, |chained| block_start(chained, arrived));
                    state.blocks.push_back((start, Zeroizing::new(block)));
                    state.prune(now);
                }
            } else {
                std::thread::sleep(Duration::from_millis(50));
            }
            let mut state = owner.state.lock().unwrap();
            state.prune(Instant::now());
        }
        drop(input);
        owner.state.lock().unwrap().blocks.clear();
    });
    *service.microphone.worker.lock().unwrap() = Some(worker);
}

pub(super) struct TimedCapture {
    done: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for TimedCapture {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) fn timed_capture(
    service: &Arc<DictationService>,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> Result<(Capture<TimedCapture>, Arc<AtomicBool>), String> {
    let warm = service.settings().keep_microphone_ready && {
        let state = service.microphone.state.lock().unwrap();
        state.rate != 0 && state.error.is_none()
    };
    let owner = service.microphone.clone();
    let times = service.capture_times.clone();
    let meter_service = Arc::downgrade(service);
    let pressed = times.lock().unwrap().0;
    let frames = Arc::new(AtomicUsize::new(0));
    let queued = frames.clone();
    let (tx, rx) = bounded(AUDIO_QUEUE_BLOCKS);
    let (etx, erx) = bounded(1);
    let (ready_tx, ready_rx) = bounded(1);
    let done = Arc::new(AtomicBool::new(false));
    let completed = done.clone();
    let final_length = Arc::new(AtomicUsize::new(usize::MAX));
    let clip_length = final_length.clone();
    let worker = std::thread::spawn(move || {
        let cold_frames = Arc::new(AtomicUsize::new(0));
        let (cold_tx, cold_rx) = bounded(AUDIO_QUEUE_BLOCKS);
        let (cold_etx, cold_erx) = bounded(1);
        let cold = if warm {
            None
        } else {
            match open_input_stream(cold_tx, cold_etx, cold_frames.clone()).and_then(|(s, r)| {
                s.play().map_err(|e| e.to_string())?;
                Ok((s, r))
            }) {
                Ok(v) => Some(v),
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            }
        };
        let rate = if let Some((_, r)) = &cold {
            *r
        } else {
            let state = owner.state.lock().unwrap();
            if state.rate == 0 || state.error.is_some() {
                let _ = ready_tx
                    .send(Err(state.error.clone().unwrap_or_else(|| {
                        "Microphone is getting ready. Try again.".into()
                    })));
                return;
            }
            state.rate
        };
        let _ = ready_tx.send(Ok(rate));
        let mut timeline = CaptureTimeline::new(pressed, Instant::now(), warm, rate);
        let mut sent = 0;
        let mut last_meter = Instant::now();
        loop {
            if completed.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
                break;
            }
            let release = times.lock().unwrap().1;

            let mut blocks = Vec::new();
            if warm {
                let state = owner.state.lock().unwrap();
                if state.error.is_some() || state.rate != rate {
                    if release.is_some() {
                        timeline.finished(release, Instant::now() + Duration::from_secs(1), false);
                    } else {
                        let _ = etx.try_send("Microphone changed or disconnected".into());
                    }
                    break;
                }
                for (at, block) in &state.blocks {
                    let through = *at + Duration::from_secs_f64(block.len() as f64 / rate as f64);
                    if through > timeline.cursor {
                        blocks.push((*at, Zeroizing::new(block.to_vec())));
                    }
                }
                backdate_batch_to_now(&mut blocks, Instant::now(), rate);
            } else {
                if let Ok(e) = cold_erx.try_recv() {
                    if release.is_some() {
                        timeline.finished(release, Instant::now() + Duration::from_secs(1), false);
                    } else {
                        let _ = etx.try_send(e);
                    }
                    break;
                }
                while let Ok(block) = cold_rx.try_recv() {
                    release_audio_frames(&cold_frames, block.len());
                    let at = blocks
                        .last()
                        .map(|(at, previous): &(Instant, Zeroizing<Vec<f32>>)| {
                            *at + Duration::from_secs_f64(previous.len() as f64 / rate as f64)
                        })
                        .unwrap_or_else(|| {
                            if timeline.audio.is_empty() {
                                Instant::now()
                                    - Duration::from_secs_f64(block.len() as f64 / rate as f64)
                            } else {
                                timeline.cursor
                            }
                        });
                    blocks.push((at, Zeroizing::new(block)));
                }
                // A drain is one observed batch: anchoring its final sample at or
                // before now preserves spacing without manufacturing future audio.
                if let Some((at, last)) = blocks.last() {
                    let end = *at + Duration::from_secs_f64(last.len() as f64 / rate as f64);
                    let late = Instant::now().saturating_duration_since(end);
                    if late > STREAM_GAP {
                        for (at, _) in &mut blocks {
                            *at += late;
                        }
                    }
                }
                backdate_batch_to_now(&mut blocks, Instant::now(), rate);
            }
            for (at, block) in blocks {
                let part = timeline.push(at, &block);
                if last_meter.elapsed() >= Duration::from_millis(100) {
                    if let Some(service) = meter_service.upgrade() {
                        service.publish_audio_level(audio_level(part));
                    }
                    last_meter = Instant::now();
                }
            }
            // Keep one source second private until release timing is known. Earlier
            // whole seconds can feed Parakeet while capture continues.
            let safe =
                timeline.audio.len().saturating_sub(rate as usize) / rate as usize * rate as usize;
            while sent < safe && release.is_none() {
                if !send_capture_audio(
                    &tx,
                    &queued,
                    &completed,
                    &cancel,
                    &timeline.audio[sent..sent + rate as usize],
                    rate,
                ) {
                    break;
                }
                sent += rate as usize;
            }
            if timeline.finished(release, Instant::now(), cancel.load(Ordering::Acquire)) {
                break;
            }
            if release.is_none() && stop.load(Ordering::Acquire) {
                // The release time is written before the stop flag, so a release that
                // landed after this pass read `release` is visible now and keeps its tail.
                if released_since(&times) {
                    continue;
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if !cancel.load(Ordering::Acquire) && !completed.load(Ordering::Acquire) {
            for part in timeline.audio[sent.min(timeline.audio.len())..].chunks(rate as usize) {
                if !send_capture_audio(&tx, &queued, &completed, &cancel, part, rate) {
                    break;
                }
            }
        }
        drop(cold);
        zeroize_pending_audio(&cold_rx, &cold_frames);
        clip_length.store(timeline.audio.len(), Ordering::Release);
        completed.store(true, Ordering::Release);
    });
    let rate = ready_rx.recv().map_err(|_| "Microphone worker stopped")??;
    Ok((
        Capture {
            stream: Some(TimedCapture {
                done: done.clone(),
                worker: Some(worker),
            }),
            audio_rx: rx,
            error_rx: erx,
            queued_frames: frames,
            sample_rate: rate,
            final_length: Some(final_length),
        },
        done,
    ))
}

fn backdate_batch_to_now(blocks: &mut [(Instant, Zeroizing<Vec<f32>>)], now: Instant, rate: u32) {
    let Some((last_at, last)) = blocks.last() else {
        return;
    };
    let endpoint = *last_at + Duration::from_secs_f64(last.len() as f64 / rate as f64);
    let shift = endpoint.saturating_duration_since(now);
    if shift.is_zero() {
        return;
    }
    for (at, _) in blocks {
        if let Some(backdated) = at.checked_sub(shift) {
            *at = backdated;
        }
    }
}

fn released_since(times: &Mutex<(Instant, Option<Instant>)>) -> bool {
    times.lock().unwrap().1.is_some()
}

/// A block later than this against the previous block's end means the stream paused.
const STREAM_GAP: Duration = Duration::from_millis(100);
/// Pauses up to this long are filled with silence; longer ones are cut out of the clip.
const MAX_PADDED_GAP: Duration = Duration::from_millis(250);

/// Blocks normally follow one another exactly. After a pause in delivery the block
/// belongs where it arrived, or every later boundary would land too early.
fn block_start(chained: Instant, arrived: Instant) -> Instant {
    if arrived.saturating_duration_since(chained) > STREAM_GAP {
        arrived
    } else {
        chained
    }
}

/// Hex's release policy: five speech frames into the base tail, then eight quiet
/// frames, with a hard 600 ms cap. Clock injection is the caller's responsibility.
fn release_tail(audio: &[f32], rate: u32, release: usize) -> (Duration, bool) {
    // Only the recent floor and release region affect this decision.
    let skip = release.saturating_sub(rate as usize * 2).min(audio.len());
    let audio = &audio[skip..];
    let release = release.saturating_sub(skip);
    let frame = rate as usize / 50;
    let base = release.saturating_add(rate as usize / 5);
    let cap = release.saturating_add(rate as usize * 3 / 5);
    if audio.len() < base {
        return (Duration::from_millis(200), true);
    }
    let mut floor = None::<f32>;
    let flags: Vec<bool> = audio
        .chunks_exact(frame)
        .map(|samples| {
            let mean = samples.iter().sum::<f32>() / samples.len() as f32;
            let energy =
                samples.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / samples.len() as f32;
            let db = 10.0 * energy.max(1e-18).log10();
            let noise = match floor {
                Some(v) if db >= v => v + 0.06,
                // Cold capture can begin inside a word, without a quiet baseline.
                // Start conservatively so that word can grow the tail too.
                None => db.clamp(-90.0, -60.0),
                _ => db.max(-90.0),
            };
            floor = Some(noise);
            db > (noise + 12.0).max(-55.0)
        })
        .collect();
    let end = (base / frame).min(flags.len());
    let mut speech = 0;
    let mut gap = 0;
    for &active in flags[..end].iter().rev() {
        if active {
            speech += 1;
            gap = 0;
            if speech == 5 {
                break;
            }
        } else {
            gap += 1;
            if gap > 2 {
                break;
            }
        }
    }
    if speech < 5 {
        return (Duration::from_millis(200), false);
    }
    let mut quiet = 0;
    for (index, &active) in flags.iter().enumerate().skip(end) {
        quiet = if active { 0 } else { quiet + 1 };
        if quiet == 8 {
            let kept = ((index + 1) * frame).min(cap).saturating_sub(release);
            return (
                Duration::from_secs_f64(kept as f64 / rate as f64).max(Duration::from_millis(200)),
                false,
            );
        }
    }
    (Duration::from_millis(600), audio.len() < cap)
}

fn send_capture_audio(
    tx: &Sender<Vec<f32>>,
    queued: &AtomicUsize,
    done: &AtomicBool,
    cancel: &AtomicBool,
    part: &[f32],
    rate: u32,
) -> bool {
    let mut audio = part.to_vec();
    if !try_reserve_audio_frames(queued, audio.len(), rate as usize * 10) {
        audio.zeroize();
        return false;
    }
    loop {
        match tx.send_timeout(audio, Duration::from_millis(5)) {
            Ok(()) => return true,
            Err(e) => {
                let disconnected = e.is_disconnected();
                audio = e.into_inner();
                if disconnected || done.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
                    release_audio_frames(queued, audio.len());
                    audio.zeroize();
                    return false;
                }
            }
        }
    }
}

#[cfg(windows)]
fn default_input_id() -> Option<String> {
    use windows::Win32::Media::Audio::{
        eCapture, eConsole, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED,
    };
    // SAFETY: COM objects stay on this thread; GetId's allocation is freed exactly once.
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let result = (|| {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eCapture, eConsole)
                .ok()?;
            let id = device.GetId().ok()?;
            let value = id.to_string().ok();
            CoTaskMemFree(Some(id.0.cast()));
            value
        })();
        if initialized {
            CoUninitialize();
        }
        result
    }
}
#[cfg(not(windows))]
fn default_input_id() -> Option<String> {
    cpal::default_host()
        .default_input_device()
        .and_then(|d| d.name().ok())
}

/// Device-free timeline; `now` is supplied by the owner or a test clock.
struct CaptureTimeline {
    origin: Instant,
    cursor: Instant,
    rate: u32,
    audio: Zeroizing<Vec<f32>>,
}
impl CaptureTimeline {
    fn new(pressed: Instant, ready: Instant, warm: bool, rate: u32) -> Self {
        let origin = if warm {
            pressed - Duration::from_millis(450)
        } else {
            ready
        };
        Self {
            origin,
            cursor: origin,
            rate,
            // Reserved in full: growing would free copies of speech without erasing them.
            audio: Zeroizing::new(Vec::with_capacity(rate as usize * MAX_SECONDS)),
        }
    }
    fn push<'a>(&mut self, at: Instant, block: &'a [f32]) -> &'a [f32] {
        if self.audio.is_empty() {
            self.origin = self.origin.max(at);
            self.cursor = self.origin;
        }
        // Sample indexes are times measured from `origin`, so a pause in the stream
        // is either filled with silence or taken out of the timeline.
        let gap = at.saturating_duration_since(self.cursor);
        if gap > MAX_PADDED_GAP {
            self.origin += gap;
        } else if !gap.is_zero() {
            let silence = ((gap.as_secs_f64() * self.rate as f64).round() as usize)
                .min((self.rate as usize * MAX_SECONDS).saturating_sub(self.audio.len()));
            let padded = self.audio.len() + silence;
            self.audio.resize(padded, 0.0);
        }
        let from = ((self.cursor.saturating_duration_since(at).as_secs_f64() * self.rate as f64)
            .round() as usize)
            .min(block.len());
        let to =
            block.len().min(from.saturating_add(
                (self.rate as usize * MAX_SECONDS).saturating_sub(self.audio.len()),
            ));
        let part = &block[from..to];
        self.audio.extend_from_slice(part);
        if !part.is_empty() {
            self.cursor = at + Duration::from_secs_f64(to as f64 / self.rate as f64);
        }
        part
    }
    fn finished(&mut self, release: Option<Instant>, now: Instant, cancel: bool) -> bool {
        if cancel {
            self.audio.zeroize();
            return true;
        }
        if self.audio.len() >= self.rate as usize * MAX_SECONDS {
            return true;
        }
        let Some(release) = release else {
            return false;
        };
        let frame = (release.saturating_duration_since(self.origin).as_secs_f64()
            * self.rate as f64)
            .round() as usize;
        let (tail, waiting) = release_tail(&self.audio, self.rate, frame);
        let boundary = release + tail;
        if (!waiting && self.cursor >= boundary) || now >= boundary + Duration::from_millis(100) {
            let keep = frame
                .saturating_add((tail.as_secs_f64() * self.rate as f64).round() as usize)
                .min(self.audio.len());
            self.audio[keep..].zeroize();
            self.audio.truncate(keep);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod timing_tests {
    use super::*;
    struct FakeStream {
        epoch: Instant,
        now: Instant,
        timeline: CaptureTimeline,
    }
    impl FakeStream {
        fn new(warm: bool, rate: u32) -> Self {
            let epoch = Instant::now();
            let pressed = epoch + Duration::from_secs(1);
            Self {
                epoch,
                now: pressed,
                timeline: CaptureTimeline::new(pressed, pressed, warm, rate),
            }
        }
        fn deliver(&mut self, through_ms: usize, speech_until: Option<usize>) {
            let rate = self.timeline.rate as usize;
            let audio: Vec<f32> = (0..through_ms * rate / 1000)
                .map(|i| {
                    let ms = i * 1000 / rate;
                    if ms >= 900 && speech_until.is_some_and(|end| ms < end) {
                        (i as f32 * 0.17).sin() * 0.1
                    } else {
                        0.00001 * (i as f32 * 0.31).sin()
                    }
                })
                .collect();
            self.timeline.push(self.epoch, &audio);
            self.now = self.epoch + Duration::from_millis(through_ms as u64);
        }
        fn stop(&mut self, release_ms: u64) -> bool {
            self.timeline.finished(
                Some(self.epoch + Duration::from_millis(release_ms)),
                self.now,
                false,
            )
        }
    }
    #[test]
    fn ring_pre_roll_is_capped_and_delayed_stop_trims_exactly() {
        let mut stream = FakeStream::new(true, 16000);
        stream.deliver(2500, None);
        assert!(stream.stop(1500));
        assert_eq!(
            stream.timeline.origin,
            stream.epoch + Duration::from_millis(550)
        );
        assert_eq!(stream.timeline.audio.len(), 1150 * 16);
    }
    #[test]
    fn available_pre_roll_and_cold_mode_do_not_invent_samples() {
        let mut stream = FakeStream::new(true, 16000);
        stream
            .timeline
            .push(stream.epoch + Duration::from_millis(900), &[0.0; 1600]);
        assert_eq!(stream.timeline.audio.len(), 1600);
        assert_eq!(
            stream.timeline.origin,
            stream.epoch + Duration::from_millis(900)
        );
        let mut cold = FakeStream::new(false, 16000);
        cold.deliver(2000, None);
        assert!(cold.stop(1500));
        assert_eq!(cold.timeline.audio.len(), 700 * 16);
    }
    #[test]
    fn speech_grows_tail_until_quiet_or_cap() {
        let mut stream = FakeStream::new(true, 16000);
        stream.deliver(1700, Some(1850));
        assert!(!stream.stop(1500));
        stream.deliver(2010, Some(1850));
        assert!(stream.stop(1500));
        assert_eq!(stream.timeline.audio.len(), (2010 - 550) * 16);
        let mut continuous = FakeStream::new(true, 16000);
        continuous.deliver(2100, Some(5000));
        assert!(continuous.stop(1500));
        assert_eq!(continuous.timeline.audio.len(), (2100 - 550) * 16);
    }
    #[test]
    fn stalled_stream_wait_is_bounded_and_cancel_has_no_tail() {
        let mut stream = FakeStream::new(true, 16000);
        stream.deliver(1500, None);
        stream.now = stream.epoch + Duration::from_millis(1799);
        assert!(!stream.stop(1500));
        stream.now += Duration::from_millis(1);
        assert!(stream.stop(1500));
        let mut cancelled = FakeStream::new(true, 16000);
        cancelled.deliver(1600, Some(5000));
        assert!(cancelled.timeline.finished(None, cancelled.now, true));
        assert!(cancelled.timeline.audio.is_empty());
    }
    #[test]
    fn resampled_clips_keep_release_tail_at_44100_and_48000() {
        for rate in [44100, 48000] {
            let mut stream = FakeStream::new(true, rate);
            stream.deliver(2400, None);
            assert!(stream.stop(1500));
            assert_eq!(stream.timeline.audio.len(), 1150 * rate as usize / 1000);
            assert_eq!(resample_to_16k(&stream.timeline.audio, rate).len(), 18400);
        }
    }
    #[test]
    fn drained_batches_are_backdated_and_release_waits_for_post_release_audio() {
        for rate in [16000, 44100, 48000] {
            let now = Instant::now();
            let samples = rate as usize / 10;
            let mut batch = vec![
                (
                    now - Duration::from_millis(100),
                    Zeroizing::new(vec![0.0; samples]),
                ),
                (now, Zeroizing::new(vec![0.0; samples])),
                (
                    now + Duration::from_millis(100),
                    Zeroizing::new(vec![0.0; samples]),
                ),
            ];
            backdate_batch_to_now(&mut batch, now, rate);
            for (at, block) in &batch {
                assert!(
                    *at + Duration::from_secs_f64(block.len() as f64 / rate as f64) <= now,
                    "queued audio must not end in the future at {rate} Hz"
                );
            }
            assert_eq!(batch[1].0 - batch[0].0, Duration::from_millis(100));
            assert_eq!(batch[2].0 - batch[1].0, Duration::from_millis(100));

            let mut timeline = CaptureTimeline::new(batch[0].0, batch[0].0, false, rate);
            for (at, block) in &batch {
                timeline.push(*at, block);
            }
            let release = now;
            assert!(!timeline.finished(Some(release), now, false));
            timeline.push(now, &vec![0.0; samples]);
            assert!(!timeline.finished(Some(release), now + Duration::from_millis(100), false));
            timeline.push(now + Duration::from_millis(100), &vec![0.0; samples]);
            assert!(timeline.finished(Some(release), now + Duration::from_millis(200), false));
        }
    }
    #[test]
    fn a_stream_that_pauses_keeps_its_release_boundary() {
        for rate in [16000u32, 44100, 48000] {
            let block = |ms: usize| vec![0.0f32; ms * rate as usize / 1000];
            // A long pause (1000..1400 ms) is cut out; the clip still ends at release + 200 ms.
            let mut long = FakeStream::new(true, rate);
            long.timeline.push(long.epoch, &block(1000));
            long.timeline
                .push(long.epoch + Duration::from_millis(1400), &block(600));
            long.now = long.epoch + Duration::from_millis(2000);
            assert!(long.stop(1700));
            let expected = (450 + 500) * rate as usize / 1000;
            assert!(long.timeline.audio.len().abs_diff(expected) <= 1);
            // A short pause (1000..1100 ms) is filled with silence.
            let mut short = FakeStream::new(true, rate);
            short.timeline.push(short.epoch, &block(1000));
            short
                .timeline
                .push(short.epoch + Duration::from_millis(1100), &block(900));
            short.now = short.epoch + Duration::from_millis(2000);
            assert!(short.stop(1700));
            let expected = (1900 - 550) * rate as usize / 1000;
            assert!(short.timeline.audio.len().abs_diff(expected) <= 1);
        }
    }
    #[test]
    fn a_paused_stream_restarts_its_timestamps_where_audio_arrived() {
        let chained = Instant::now();
        assert_eq!(
            block_start(chained, chained + Duration::from_millis(20)),
            chained
        );
        let arrived = chained + Duration::from_millis(400);
        assert_eq!(block_start(chained, arrived), arrived);
    }
    #[test]
    fn the_timeline_never_reallocates_recorded_audio() {
        let mut stream = FakeStream::new(true, 16000);
        let reserved = stream.timeline.audio.capacity();
        stream.deliver(5000, None);
        assert_eq!(stream.timeline.audio.capacity(), reserved);
        assert!(reserved >= 16000 * MAX_SECONDS);
    }
    #[test]
    fn a_release_published_after_the_snapshot_is_still_seen() {
        let times = Mutex::new((Instant::now(), None));
        assert!(!released_since(&times));
        times.lock().unwrap().1 = Some(Instant::now());
        assert!(released_since(&times));
    }
    #[test]
    fn cold_speech_without_a_quiet_baseline_still_gets_the_full_tail() {
        let rate = 16000;
        let samples: Vec<f32> = (0..rate * 2)
            .map(|i| (i as f32 * 0.17).sin() * 0.1)
            .collect();
        assert_eq!(
            release_tail(&samples, rate as u32, rate),
            (Duration::from_millis(600), false)
        );
    }

    #[test]
    fn short_key_click_does_not_grow_tail() {
        let mut stream = FakeStream::new(true, 16000);
        stream.deliver(1800, None);
        let from = (1680 - 550) * 16;
        for (i, v) in stream.timeline.audio[from..from + 320]
            .iter_mut()
            .enumerate()
        {
            *v = (i as f32 * 0.17).sin() * 0.5;
        }
        assert!(stream.stop(1500));
        assert_eq!(stream.timeline.audio.len(), 1150 * 16);
    }
}
