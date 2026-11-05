use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use tauri::{AppHandle, Emitter, Manager};

pub const NUM_BINS: usize = 16;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VisualizerStatus {
    pub enabled: bool,
    pub active: bool,
    pub reduced_motion: bool,
    pub bin_count: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VisualizerSnapshot {
    pub bins: Vec<f32>,
    pub energy: f32,
    pub is_silent: bool,
}

impl Default for VisualizerSnapshot {
    fn default() -> Self {
        Self {
            bins: vec![0.0; NUM_BINS],
            energy: 0.0,
            is_silent: true,
        }
    }
}

static PLAYING: Mutex<bool> = Mutex::new(false);
static WAKE: Condvar = Condvar::new();

fn playback_active(active: bool, playing: bool) -> bool {
    active && playing
}
pub fn playback_changed(active: bool, playing: bool) {
    let mut current = PLAYING.lock().unwrap_or_else(|p| p.into_inner());
    *current = playback_active(active, playing);
    WAKE.notify_all();
}

fn capture_requested() -> bool {
    ENABLED.load(Ordering::Acquire) && *PLAYING.lock().unwrap_or_else(|p| p.into_inner())
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static REDUCED_MOTION: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static OUTPUT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn notify_output_changed() {
    OUTPUT_GENERATION.fetch_add(1, Ordering::Release);
}
static SNAPSHOT: Mutex<Option<VisualizerSnapshot>> = Mutex::new(None);

fn check_authorized_window(window: Option<&tauri::Window>) -> Result<(), String> {
    if let Some(w) = window {
        let label = w.label();
        if label != "main" && label != "settings" {
            return Err(
                "Access denied: only main and settings windows may perform this action".into(),
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub fn visualizer_status() -> VisualizerStatus {
    VisualizerStatus {
        enabled: ENABLED.load(Ordering::Relaxed),
        active: ACTIVE.load(Ordering::Relaxed),
        reduced_motion: REDUCED_MOTION.load(Ordering::Relaxed),
        bin_count: NUM_BINS,
    }
}

#[tauri::command]
pub fn visualizer_set_enabled(window: tauri::Window, enabled: bool) -> Result<(), String> {
    check_authorized_window(Some(&window))?;
    let _preference =
        crate::service_preferences::write(window.app_handle(), "visualizer", enabled)?;
    ENABLED.store(enabled, Ordering::SeqCst);
    {
        let _guard = PLAYING.lock().unwrap_or_else(|p| p.into_inner());
        WAKE.notify_all();
    }
    if !enabled {
        ACTIVE.store(false, Ordering::SeqCst);
        if let Ok(mut snap) = SNAPSHOT.lock() {
            *snap = Some(VisualizerSnapshot::default());
        }
    }
    Ok(())
}

#[tauri::command]
pub fn visualizer_set_reduced_motion(window: tauri::Window, enabled: bool) -> Result<(), String> {
    check_authorized_window(Some(&window))?;
    REDUCED_MOTION.store(enabled, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub fn visualizer_spectrum() -> Result<VisualizerSnapshot, String> {
    SNAPSHOT
        .lock()
        .map_err(|_| "Failed to lock visualizer snapshot")?
        .clone()
        .ok_or_else(|| "Visualizer inactive".to_string())
}

/// Computes Cooley-Tukey Radix-2 FFT on complex input in-place.
/// Length of input must be a power of 2.
pub fn fft_radix2(real: &mut [f32], imag: &mut [f32]) {
    let n = real.len();
    if n <= 1 {
        return;
    }

    // Bit-reversal permutation
    let mut j = 0;
    for i in 0..n {
        if i < j {
            real.swap(i, j);
            imag.swap(i, j);
        }
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
    }

    // Cooley-Tukey iterative computation
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let angle = -2.0 * std::f32::consts::PI / (len as f32);
        let w_step_r = angle.cos();
        let w_step_i = angle.sin();

        let mut i = 0;
        while i < n {
            let mut w_r = 1.0f32;
            let mut w_i = 0.0f32;

            for k in 0..half {
                let u_r = real[i + k];
                let u_i = imag[i + k];

                let v_r = real[i + k + half] * w_r - imag[i + k + half] * w_i;
                let v_i = real[i + k + half] * w_i + imag[i + k + half] * w_r;

                real[i + k] = u_r + v_r;
                imag[i + k] = u_i + v_i;

                real[i + k + half] = u_r - v_r;
                imag[i + k + half] = u_i - v_i;

                let next_w_r = w_r * w_step_r - w_i * w_step_i;
                let next_w_i = w_r * w_step_i + w_i * w_step_r;
                w_r = next_w_r;
                w_i = next_w_i;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// Applies a Hann window to reduce spectral leakage.
pub fn apply_hann_window(samples: &[f32], out: &mut [f32]) {
    let n = samples.len();
    if n == 0 {
        return;
    }
    let denom = (n - 1) as f32;
    for (i, (&s, o)) in samples.iter().zip(out.iter_mut()).enumerate() {
        let multiplier = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * (i as f32) / denom).cos());
        *o = s * multiplier;
    }
}

/// Analyzes windowed PCM float samples into normalized logarithmically grouped frequency bins.
pub fn analyze_samples(
    samples: &[f32],
    prev_bins: &[f32],
    reduced_motion: bool,
) -> VisualizerSnapshot {
    let n = samples.len();
    if n < 64 || (n & (n - 1)) != 0 {
        return VisualizerSnapshot::default();
    }

    // Compute audio energy (RMS)
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    let rms = (sum_sq / (n as f32)).sqrt();
    let is_silent = rms < 0.001;

    if is_silent {
        // Smoothly decay to zero when silent
        let decay = if reduced_motion { 0.80 } else { 0.88 };
        let bins: Vec<f32> = prev_bins
            .iter()
            .map(|&b| {
                let v = b * decay;
                if v < 0.005 {
                    0.0
                } else {
                    v
                }
            })
            .collect();
        return VisualizerSnapshot {
            bins,
            energy: 0.0,
            is_silent: true,
        };
    }

    let mut windowed = vec![0.0f32; n];
    apply_hann_window(samples, &mut windowed);

    let mut imag = vec![0.0f32; n];
    fft_radix2(&mut windowed, &mut imag);

    // Compute magnitudes for positive half of spectrum
    let half_n = n / 2;
    let mut magnitudes = vec![0.0f32; half_n];
    for i in 0..half_n {
        let mag = (windowed[i] * windowed[i] + imag[i] * imag[i]).sqrt() / (n as f32);
        magnitudes[i] = mag;
    }

    // Group into NUM_BINS logarithmic frequency bands
    let mut current_bins = vec![0.0f32; NUM_BINS];
    for (b, bin) in current_bins.iter_mut().enumerate() {
        let start_frac = (b as f32 / NUM_BINS as f32).powf(2.0);
        let end_frac = ((b + 1) as f32 / NUM_BINS as f32).powf(2.0);

        let start_idx = (start_frac * (half_n as f32)).floor() as usize;
        let end_idx = ((end_frac * (half_n as f32)).ceil() as usize)
            .max(start_idx + 1)
            .min(half_n);

        let slice = &magnitudes[start_idx..end_idx];
        let max_val = slice.iter().copied().fold(0.0f32, f32::max);

        // Normalize with logarithmic perception curve
        let normalized = (max_val * 12.0).clamp(0.0, 1.0);

        // Temporal smoothing
        let prev = prev_bins.get(b).copied().unwrap_or(0.0);
        let smoothed = if normalized > prev {
            // Fast attack
            let attack = if reduced_motion { 0.35 } else { 0.65 };
            prev + (normalized - prev) * attack
        } else {
            // Smooth decay
            let decay = if reduced_motion { 0.75 } else { 0.85 };
            prev * decay
        };

        *bin = smoothed.clamp(0.0, 1.0);
    }

    let energy = (rms * 5.0).clamp(0.0, 1.0);

    VisualizerSnapshot {
        bins: current_bins,
        energy,
        is_silent: false,
    }
}

#[cfg(windows)]
#[path = "loopback.rs"]
mod native_wasapi;

/// Starts the visualizer monitor loop.
pub fn start(app: AppHandle) {
    ENABLED.store(
        crate::service_preferences::read(&app, "visualizer"),
        Ordering::Release,
    );
    std::thread::spawn(move || loop {
        let playing = PLAYING.lock().unwrap_or_else(|p| p.into_inner());
        let playing = WAKE
            .wait_while(playing, |playing| {
                !ENABLED.load(Ordering::Acquire) || !*playing
            })
            .unwrap_or_else(|p| p.into_inner());
        drop(playing);
        if capture_requested() {
            #[cfg(windows)]
            native_wasapi::run_loopback_capture(app.clone());

            #[cfg(not(windows))]
            {
                ACTIVE.store(true, Ordering::SeqCst);
                while capture_requested() {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    let snap = VisualizerSnapshot::default();
                    let _ = app.emit("visualizer", snap);
                }
                ACTIVE.store(false, Ordering::SeqCst);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_requires_active_playback() {
        assert!(playback_active(true, true));
        assert!(!playback_active(true, false));
        assert!(!playback_active(false, true));
        assert!(!playback_active(false, false));
    }

    #[test]
    fn test_fft_radix2_impulse() {
        let mut real = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let mut imag = vec![0.0; 8];
        fft_radix2(&mut real, &mut imag);

        // FFT of unit impulse has flat magnitude of 1.0 at all bins
        for &r in &real {
            assert!((r - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn test_analyze_samples_silence() {
        let samples = vec![0.0f32; 128];
        let prev_bins = vec![0.5f32; NUM_BINS];
        let snapshot = analyze_samples(&samples, &prev_bins, false);
        assert!(snapshot.is_silent);
        assert_eq!(snapshot.energy, 0.0);
        // Ensure smooth settling towards zero
        for &b in &snapshot.bins {
            assert!(b < 0.5);
        }
    }

    #[test]
    fn test_analyze_samples_sine_wave() {
        let mut samples = vec![0.0f32; 128];
        for (i, s) in samples.iter_mut().enumerate() {
            *s = (2.0 * std::f32::consts::PI * 4.0 * (i as f32) / 128.0).sin() * 0.8;
        }
        let prev_bins = vec![0.0f32; NUM_BINS];
        let snapshot = analyze_samples(&samples, &prev_bins, false);
        assert!(!snapshot.is_silent);
        assert!(snapshot.energy > 0.0);
        assert!(snapshot.bins.iter().any(|&b| b > 0.0));
    }

    #[test]
    fn test_visualizer_status_default() {
        let status = visualizer_status();
        assert_eq!(status.bin_count, NUM_BINS);
    }
}
