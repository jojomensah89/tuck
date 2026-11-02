//! CPAL's WASAPI backend enables loopback when an input stream is built on a render device.
use super::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Samples {
    values: VecDeque<f32>,
    last: Instant,
}

pub fn run_loopback_capture(app: AppHandle) {
    if let Err(error) = capture(&app) {
        ACTIVE.store(false, Ordering::Release);
        let _ = app.emit("visualizer-error", error);
        std::thread::sleep(Duration::from_secs(1));
    }
}
fn capture(app: &AppHandle) -> Result<(), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("No playback device is available")?;
    let supported = device
        .default_output_config()
        .map_err(|_| "Playback format unavailable")?;
    let channels = supported.channels() as usize;
    if channels == 0 || channels > 32 {
        return Err("Unsupported playback channel count".into());
    }
    let config: StreamConfig = supported.clone().into();
    let samples = Arc::new(Mutex::new(Samples {
        values: VecDeque::with_capacity(512),
        last: Instant::now(),
    }));
    let failed = Arc::new(AtomicBool::new(false));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => stream::<f32, _>(
            &device,
            &config,
            channels,
            samples.clone(),
            failed.clone(),
            |v| v,
        ),
        SampleFormat::I16 => stream::<i16, _>(
            &device,
            &config,
            channels,
            samples.clone(),
            failed.clone(),
            |v| v as f32 / 32768.0,
        ),
        SampleFormat::U16 => stream::<u16, _>(
            &device,
            &config,
            channels,
            samples.clone(),
            failed.clone(),
            |v| (v as f32 - 32768.0) / 32768.0,
        ),
        _ => return Err("Unsupported playback sample format".into()),
    }?;
    stream
        .play()
        .map_err(|_| "Could not start playback loopback analysis")?;
    ACTIVE.store(true, Ordering::Release);
    let mut previous = vec![0.0; NUM_BINS];
    let output_generation = OUTPUT_GENERATION.load(Ordering::Acquire);
    while ENABLED.load(Ordering::Acquire)
        && !failed.load(Ordering::Acquire)
        && OUTPUT_GENERATION.load(Ordering::Acquire) == output_generation
    {
        std::thread::sleep(Duration::from_millis(33));
        let audio = {
            let current = samples.lock().map_err(|_| "Audio analysis unavailable")?;
            if current.last.elapsed() > Duration::from_millis(150) || current.values.len() < 512 {
                vec![0.0; 512]
            } else {
                current.values.iter().copied().collect()
            }
        };
        let snapshot = analyze_samples(&audio, &previous, REDUCED_MOTION.load(Ordering::Relaxed));
        previous.clone_from(&snapshot.bins);
        if let Ok(mut saved) = SNAPSHOT.lock() {
            *saved = Some(snapshot.clone());
        }
        let _ = app.emit("visualizer", snapshot);
    }
    drop(stream);
    ACTIVE.store(false, Ordering::Release);
    if let Ok(mut saved) = SNAPSHOT.lock() {
        *saved = Some(VisualizerSnapshot::default());
    }
    let _ = app.emit("visualizer", VisualizerSnapshot::default());
    if failed.load(Ordering::Acquire) {
        return Err("Playback device changed or became unavailable".into());
    }
    Ok(())
}
fn stream<T, F>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    samples: Arc<Mutex<Samples>>,
    failed: Arc<AtomicBool>,
    convert: F,
) -> Result<Stream, String>
where
    T: cpal::SizedSample + Copy + Send + 'static,
    F: Fn(T) -> f32 + Send + 'static,
{
    device
        .build_input_stream(
            config,
            move |input: &[T], _| {
                if let Ok(mut data) = samples.try_lock() {
                    for frame in input.chunks_exact(channels) {
                        let value =
                            frame.iter().copied().map(&convert).sum::<f32>() / channels as f32;
                        if data.values.len() == 512 {
                            data.values.pop_front();
                        }
                        data.values.push_back(if value.is_finite() {
                            value.clamp(-1.0, 1.0)
                        } else {
                            0.0
                        });
                    }
                    data.last = Instant::now();
                }
            },
            move |_| {
                failed.store(true, Ordering::Release);
            },
            None,
        )
        .map_err(|_| "Playback loopback is unavailable for this device".into())
}
