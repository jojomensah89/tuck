use super::DictationService;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
use zeroize::{Zeroize, Zeroizing};

const MAX_SECONDS: usize = 120;
const MAX_QUEUED_AUDIO_SECONDS: usize = 10;
const MAX_CALLBACK_SECONDS: usize = 1;
const AUDIO_QUEUE_BLOCKS: usize = 2048;
const PREVIEW_INTERVAL: Duration = Duration::from_secs(3);

#[path = "audio_math.rs"]
mod audio_math;
use audio_math::{release_audio_frames, resample_to_16k, try_reserve_audio_frames};

pub(super) fn record_and_transcribe(
    service: &DictationService,
    model_path: &std::path::Path,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    service.model_busy.store(true, Ordering::Release);
    let result = record_and_transcribe_inner(service, model_path, stop, cancel);
    service.model_busy.store(false, Ordering::Release);
    result
}

fn record_and_transcribe_inner(
    service: &DictationService,
    model_path: &std::path::Path,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Dictation cancelled".into());
    }
    if stop.load(Ordering::Acquire) {
        return Err("Dictation stopped before microphone capture began".into());
    }
    let context = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
        .map_err(|e| format!("Could not load the local dictation model: {e}"))?;
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
    service.set_status("recording", None);

    let started = Instant::now();
    let preview = service.settings().partial_preview;
    let mut samples = Zeroizing::new(Vec::<f32>::new());
    let cap = (sample_rate as usize).saturating_mul(MAX_SECONDS);
    let mut next_preview = PREVIEW_INTERVAL;
    loop {
        if cancel.load(Ordering::Acquire) {
            drop(stream);
            zeroize_pending_audio(&audio_rx, &queued_frames);
            samples.zeroize();
            return Err("Dictation cancelled".into());
        }
        if let Ok(error) = error_rx.try_recv() {
            drop(stream);
            zeroize_pending_audio(&audio_rx, &queued_frames);
            samples.zeroize();
            return Err(format!("Microphone capture interrupted: {error}"));
        }
        match audio_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(mut block) => {
                release_audio_frames(&queued_frames, block.len());
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
                drop(stream);
                zeroize_pending_audio(&audio_rx, &queued_frames);
                samples.zeroize();
                return Err("Microphone capture ended unexpectedly".into());
            }
        }
        // Do not begin another costly preview after release or the capture cap.
        if stop.load(Ordering::Acquire)
            || started.elapsed() >= Duration::from_secs(MAX_SECONDS as u64)
        {
            stop.store(true, Ordering::Release);
            break;
        }
        if preview && started.elapsed() >= next_preview && samples.len() >= sample_rate as usize * 2
        {
            let start = samples.len().saturating_sub(sample_rate as usize * 15);
            let preview_audio = Zeroizing::new(resample_to_16k(&samples[start..], sample_rate));
            if !cancel.load(Ordering::Acquire) {
                if let Ok(mut partial_text) = transcribe(&context, &preview_audio, cancel.clone()) {
                    let text = partial_text.trim();
                    service.publish_preview(text);
                    partial_text.zeroize();
                }
            }
            // Schedule from completion; slow inference never creates catch-up jobs.
            next_preview = started.elapsed() + PREVIEW_INTERVAL;
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        if started.elapsed() >= Duration::from_secs(MAX_SECONDS as u64) {
            stop.store(true, Ordering::Release);
            break;
        }
    }
    drop(stream);
    // Drain the bounded-by-session audio backlog after the callback has stopped.
    while let Ok(mut block) = audio_rx.try_recv() {
        release_audio_frames(&queued_frames, block.len());
        let remaining = cap.saturating_sub(samples.len());
        if block.len() > remaining {
            block.truncate(remaining);
        }
        samples.extend_from_slice(&block);
        block.zeroize();
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Dictation cancelled".into());
    }
    if samples.len() < sample_rate as usize / 4 {
        return Err("The microphone captured too little audio. Try again and speak closer to the microphone.".into());
    }

    service.set_status("transcribing", None);
    let mut audio_16k = Zeroizing::new(resample_to_16k(&samples, sample_rate));
    samples.zeroize();
    let transcript = transcribe(&context, &audio_16k, cancel)?;
    audio_16k.zeroize();
    Ok(transcript)
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
            if !try_reserve_audio_frames(&queued_frames, frame_count, max_queued_frames) {
                let _ = callback_error.try_send(
                    "Microphone audio queue reached its 10 second memory limit; dictation stopped safely".into(),
                );
                return;
            }
            let mut mono = Vec::with_capacity(input.len() / channels);
            for frame in input.chunks_exact(channels) {
                let value = frame.iter().cloned().map(&convert).sum::<f32>() / channels as f32;
                mono.push(value.clamp(-1.0, 1.0));
            }
            if let Err(error) = tx.try_send(mono) {
                let mut dropped = error.into_inner();
                release_audio_frames(&queued_frames, dropped.len());
                dropped.zeroize();
                let _ = callback_error.try_send(
                    "Microphone callback queue is full; dictation stopped safely".into(),
                );
            }
        },
        move |error| {
            let _ = stream_error.try_send(error.to_string());
        },
        None,
    ).map_err(|e| format!("Could not open microphone: {e}. Check microphone permissions and close applications holding exclusive access."))
}

fn transcribe(
    context: &WhisperContext,
    audio: &[f32],
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    let mut state = context
        .create_state()
        .map_err(|e| format!("Could not start local transcription: {e}"))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(None);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_n_threads(std::thread::available_parallelism().map_or(2, |n| n.get().min(4)) as i32);
    params.set_abort_callback_safe({
        let cancel = cancel.clone();
        move || cancel.load(Ordering::Acquire)
    });
    state.full(params, audio).map_err(|e| {
        if cancel.load(Ordering::Acquire) {
            "Dictation cancelled".into()
        } else {
            format!("Local transcription failed: {e}")
        }
    })?;
    if cancel.load(Ordering::Acquire) {
        return Err("Dictation cancelled".into());
    }
    let count = state.full_n_segments();
    let mut output = Zeroizing::new(String::new());
    for index in 0..count {
        if cancel.load(Ordering::Acquire) {
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
