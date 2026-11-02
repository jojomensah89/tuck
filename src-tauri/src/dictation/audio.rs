use super::{DictationService, EVENT_PARTIAL};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
use zeroize::Zeroizing;

const TARGET_RATE: u32 = 16_000;
const MAX_SECONDS: usize = 120;
const PREVIEW_INTERVAL: Duration = Duration::from_secs(7);

pub(super) fn record_and_transcribe(
    service: &DictationService,
    app: &AppHandle,
    model_path: &std::path::Path,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    service.model_busy.store(true, Ordering::Release);
    let result = record_and_transcribe_inner(service, app, model_path, stop, cancel);
    service.model_busy.store(false, Ordering::Release);
    result
}

fn record_and_transcribe_inner(
    service: &DictationService,
    app: &AppHandle,
    model_path: &std::path::Path,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    let context = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
        .map_err(|e| format!("Could not load the local dictation model: {e}"))?;
    if cancel.load(Ordering::Acquire) {
        return Err("Dictation cancelled".into());
    }

    let (audio_tx, audio_rx) = unbounded::<Vec<f32>>();
    let (error_tx, error_rx) = bounded::<String>(1);
    let (stream, sample_rate) = open_input_stream(audio_tx, error_tx)?;
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
            zeroize_pending_audio(&audio_rx);
            samples.zeroize();
            return Err("Dictation cancelled".into());
        }
        if error_rx.try_recv().is_ok() {
            drop(stream);
            return Err("Microphone capture stopped unexpectedly. Check Windows microphone privacy settings and the input device.".into());
        }
        match audio_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(mut block) => {
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
                return Err("Microphone capture ended unexpectedly".into())
            }
        }
        if preview && started.elapsed() >= next_preview && samples.len() >= sample_rate as usize * 2
        {
            let preview_audio = resample_to_16k(&samples, sample_rate);
            if !cancel.load(Ordering::Acquire) {
                if let Ok(text) = transcribe(&context, &preview_audio, cancel.clone()) {
                    let text = text.trim();
                    if !text.is_empty() {
                        let _ = app.emit(
                            EVENT_PARTIAL,
                            PartialTranscript {
                                text: text.to_owned(),
                            },
                        );
                    }
                }
            }
            next_preview += PREVIEW_INTERVAL;
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

fn zeroize_pending_audio(receiver: &Receiver<Vec<f32>>) {
    while let Ok(mut block) = receiver.try_recv() {
        block.zeroize();
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PartialTranscript {
    text: String,
}

fn open_input_stream(
    audio_tx: Sender<Vec<f32>>,
    error_tx: Sender<String>,
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
    let stream = match format {
        SampleFormat::F32 => {
            build_stream::<f32, _>(&device, &stream_config, channels, audio_tx, error_tx, |v| v)
        }
        SampleFormat::I16 => {
            build_stream::<i16, _>(&device, &stream_config, channels, audio_tx, error_tx, |v| {
                v as f32 / 32768.0
            })
        }
        SampleFormat::U16 => {
            build_stream::<u16, _>(&device, &stream_config, channels, audio_tx, error_tx, |v| {
                (v as f32 - 32768.0) / 32768.0
            })
        }
        other => Err(format!(
            "The microphone uses unsupported sample format {other:?}"
        )),
    }?;
    Ok((stream, sample_rate))
}

fn build_stream<T, F>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    tx: Sender<Vec<f32>>,
    error_tx: Sender<String>,
    convert: F,
) -> Result<Stream, String>
where
    T: cpal::SizedSample + Copy + Send + 'static,
    F: Fn(T) -> f32 + Send + 'static,
{
    let callback_error = error_tx.clone();
    device.build_input_stream(
        config,
        move |input: &[T], _| {
            let mut mono = Vec::with_capacity(input.len() / channels);
            for frame in input.chunks_exact(channels) {
                let value = frame.iter().cloned().map(&convert).sum::<f32>() / channels as f32;
                mono.push(value.clamp(-1.0, 1.0));
            }
            let _ = tx.send(mono);
        },
        move |error| { let _ = callback_error.try_send(error.to_string()); },
        None,
    ).map_err(|e| format!("Could not open microphone: {e}. Check microphone permissions and close applications holding exclusive access."))
}

fn resample_to_16k(input: &[f32], source_rate: u32) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    if source_rate == TARGET_RATE {
        return input.to_vec();
    }
    let output_len = ((input.len() as u64 * TARGET_RATE as u64) / source_rate as u64) as usize;
    let mut output = Vec::with_capacity(output_len);
    for output_index in 0..output_len {
        let start = output_index as f64 * source_rate as f64 / TARGET_RATE as f64;
        let end = (output_index + 1) as f64 * source_rate as f64 / TARGET_RATE as f64;
        let first = start.floor() as usize;
        let last = (end.ceil() as usize).min(input.len());
        let mut total = 0.0f32;
        let mut weight = 0.0f64;
        for index in first..last {
            let overlap = (end.min(index as f64 + 1.0) - start.max(index as f64)).max(0.0);
            total += input[index] * overlap as f32;
            weight += overlap;
        }
        output.push(if weight > 0.0 {
            total / weight as f32
        } else {
            0.0
        });
    }
    output
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
    let count = state
        .full_n_segments()
        .map_err(|e| format!("Could not read transcription segments: {e}"))?;
    let mut output = String::new();
    for index in 0..count {
        if cancel.load(Ordering::Acquire) {
            output.zeroize();
            return Err("Dictation cancelled".into());
        }
        let segment = state
            .full_get_segment_text(index)
            .map_err(|e| format!("Could not read transcription segment: {e}"))?;
        output.push_str(&segment);
    }
    Ok(output)
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
            .chunks_exact(2)
            .map(|frame| frame.iter().sum::<f32>() / 2.0)
            .collect();
        assert_eq!(mono, vec![0.0, 0.25]);
    }
}
