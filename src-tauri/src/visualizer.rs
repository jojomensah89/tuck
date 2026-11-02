use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

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

static ENABLED: AtomicBool = AtomicBool::new(false);
static REDUCED_MOTION: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static SNAPSHOT: Mutex<Option<VisualizerSnapshot>> = Mutex::new(None);

fn check_authorized_window(window: Option<&tauri::Window>) -> Result<(), String> {
    if let Some(w) = window {
        let label = w.label();
        if label != "main" && label != "settings" {
            return Err("Access denied: only main and settings windows may perform this action".into());
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
pub fn visualizer_set_enabled(
    window: Option<tauri::Window>,
    enabled: bool,
) -> Result<(), String> {
    check_authorized_window(window.as_ref())?;
    ENABLED.store(enabled, Ordering::SeqCst);
    if !enabled {
        ACTIVE.store(false, Ordering::SeqCst);
        if let Ok(mut snap) = SNAPSHOT.lock() {
            *snap = Some(VisualizerSnapshot::default());
        }
    }
    Ok(())
}

#[tauri::command]
pub fn visualizer_set_reduced_motion(
    window: Option<tauri::Window>,
    enabled: bool,
) -> Result<(), String> {
    check_authorized_window(window.as_ref())?;
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
    for b in 0..NUM_BINS {
        let start_frac = (b as f32 / NUM_BINS as f32).powf(2.0);
        let end_frac = ((b + 1) as f32 / NUM_BINS as f32).powf(2.0);

        let start_idx = (start_frac * (half_n as f32)).floor() as usize;
        let end_idx = ((end_frac * (half_n as f32)).ceil() as usize).max(start_idx + 1).min(half_n);

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

        current_bins[b] = smoothed.clamp(0.0, 1.0);
    }

    let energy = (rms * 5.0).clamp(0.0, 1.0);

    VisualizerSnapshot {
        bins: current_bins,
        energy,
        is_silent: false,
    }
}

#[cfg(windows)]
#[allow(non_snake_case, dead_code)]
mod native_wasapi {
    use super::*;
    use std::ffi::c_void;

    type HRESULT = i32;

    #[repr(C)]
    struct GUID {
        Data1: u32,
        Data2: u16,
        Data3: u16,
        Data4: [u8; 8],
    }

    const CLSID_MM_DEVICE_ENUMERATOR: GUID = GUID {
        Data1: 0xbcde0395,
        Data2: 0xe52f,
        Data3: 0x467c,
        Data4: [0x8e, 0x3d, 0xc4, 0x57, 0x92, 0x91, 0x69, 0x2e],
    };

    const IID_IMM_DEVICE_ENUMERATOR: GUID = GUID {
        Data1: 0xa95664d2,
        Data2: 0x9614,
        Data3: 0x4f35,
        Data4: [0xa7, 0x46, 0xde, 0x8d, 0xb6, 0x36, 0x17, 0xe6],
    };

    const IID_IAUDIO_CLIENT: GUID = GUID {
        Data1: 0x1cb9ad4c,
        Data2: 0xdbfa,
        Data3: 0x4c32,
        Data4: [0xb1, 0x78, 0xc2, 0xf5, 0x68, 0xa7, 0x03, 0xb2],
    };

    const IID_IAUDIO_CAPTURE_CLIENT: GUID = GUID {
        Data1: 0xc8adbd64,
        Data2: 0xe71e,
        Data3: 0x48a0,
        Data4: [0xa4, 0xde, 0x18, 0x5c, 0x39, 0x5c, 0xd3, 0x17],
    };

    const AUDCLNT_STREAMFLAGS_LOOPBACK: u32 = 0x00020000;
    const AUDCLNT_SHAREMODE_SHARED: u32 = 0;
    const CLSCTX_ALL: u32 = 23;
    const E_RENDER: u32 = 0;
    const E_CONSOLE: u32 = 0;

    #[repr(C)]
    struct WAVEFORMATEX {
        wFormatTag: u16,
        nChannels: u16,
        nSamplesPerSec: u32,
        nAvgBytesPerSec: u32,
        nBlockAlign: u16,
        wBitsPerSample: u16,
        cbSize: u16,
    }

    #[repr(C)]
    struct IMMDeviceEnumeratorVtbl {
        QueryInterface: usize,
        AddRef: unsafe extern "system" fn(this: *mut c_void) -> u32,
        Release: unsafe extern "system" fn(this: *mut c_void) -> u32,
        EnumAudioEndpoints: usize,
        GetDefaultAudioEndpoint: unsafe extern "system" fn(
            this: *mut c_void,
            dataFlow: u32,
            role: u32,
            ppEndpoint: *mut *mut c_void,
        ) -> HRESULT,
    }

    #[repr(C)]
    struct IMMDeviceVtbl {
        QueryInterface: usize,
        AddRef: unsafe extern "system" fn(this: *mut c_void) -> u32,
        Release: unsafe extern "system" fn(this: *mut c_void) -> u32,
        Activate: unsafe extern "system" fn(
            this: *mut c_void,
            iid: *const GUID,
            dwClsCtx: u32,
            pActivationParams: *mut c_void,
            ppInterface: *mut *mut c_void,
        ) -> HRESULT,
    }

    #[repr(C)]
    struct IAudioClientVtbl {
        QueryInterface: usize,
        AddRef: unsafe extern "system" fn(this: *mut c_void) -> u32,
        Release: unsafe extern "system" fn(this: *mut c_void) -> u32,
        Initialize: unsafe extern "system" fn(
            this: *mut c_void,
            ShareMode: u32,
            StreamFlags: u32,
            hnsBufferDuration: i64,
            hnsPeriodicity: i64,
            pFormat: *const WAVEFORMATEX,
            AudioSessionGuid: *const GUID,
        ) -> HRESULT,
        GetBufferSize: unsafe extern "system" fn(this: *mut c_void, pNumBufferFrames: *mut u32) -> HRESULT,
        GetStreamLatency: usize,
        GetCurrentPadding: usize,
        IsFormatSupported: usize,
        GetMixFormat: unsafe extern "system" fn(this: *mut c_void, ppDeviceFormat: *mut *mut WAVEFORMATEX) -> HRESULT,
        GetDevicePeriod: usize,
        Start: unsafe extern "system" fn(this: *mut c_void) -> HRESULT,
        Stop: unsafe extern "system" fn(this: *mut c_void) -> HRESULT,
        Reset: usize,
        SetEventHandle: usize,
        GetService: unsafe extern "system" fn(
            this: *mut c_void,
            riid: *const GUID,
            ppv: *mut *mut c_void,
        ) -> HRESULT,
    }

    #[repr(C)]
    struct IAudioCaptureClientVtbl {
        QueryInterface: usize,
        AddRef: unsafe extern "system" fn(this: *mut c_void) -> u32,
        Release: unsafe extern "system" fn(this: *mut c_void) -> u32,
        GetBuffer: unsafe extern "system" fn(
            this: *mut c_void,
            ppData: *mut *mut u8,
            pNumFramesToRead: *mut u32,
            pdwFlags: *mut u32,
            pu64DevicePosition: *mut u64,
            pu64QPCPosition: *mut u64,
        ) -> HRESULT,
        ReleaseBuffer: unsafe extern "system" fn(this: *mut c_void, NumFramesRead: u32) -> HRESULT,
        GetNextPacketSize: unsafe extern "system" fn(this: *mut c_void, pNumFramesInNextPacket: *mut u32) -> HRESULT,
    }

    extern "system" {
        fn CoInitializeEx(pvReserved: *mut c_void, dwCoInit: u32) -> HRESULT;
        fn CoUninitialize();
        fn CoCreateInstance(
            rclsid: *const GUID,
            pUnkOuter: *mut c_void,
            dwClsContext: u32,
            riid: *const GUID,
            ppv: *mut *mut c_void,
        ) -> HRESULT;
        fn CoTaskMemFree(pv: *mut c_void);
    }

    pub fn run_loopback_capture(app: AppHandle) {
        unsafe {
            // COINIT_MULTITHREADED = 0x0
            let _ = CoInitializeEx(std::ptr::null_mut(), 0);
        }

        let mut p_enumerator: *mut c_void = std::ptr::null_mut();
        let hr = unsafe {
            CoCreateInstance(
                &CLSID_MM_DEVICE_ENUMERATOR,
                std::ptr::null_mut(),
                CLSCTX_ALL,
                &IID_IMM_DEVICE_ENUMERATOR,
                &mut p_enumerator,
            )
        };

        if hr != 0 || p_enumerator.is_null() {
            unsafe { CoUninitialize() };
            return;
        }

        let enumerator_vtbl = unsafe { *(p_enumerator as *mut *const IMMDeviceEnumeratorVtbl) };
        let mut p_device: *mut c_void = std::ptr::null_mut();

        // Loopback strictly on eRender endpoint (playback audio, NEVER microphone capture)
        let hr = unsafe {
            ((*enumerator_vtbl).GetDefaultAudioEndpoint)(
                p_enumerator,
                E_RENDER,
                E_CONSOLE,
                &mut p_device,
            )
        };

        if hr != 0 || p_device.is_null() {
            unsafe {
                ((*enumerator_vtbl).Release)(p_enumerator);
                CoUninitialize();
            }
            return;
        }

        let device_vtbl = unsafe { *(p_device as *mut *const IMMDeviceVtbl) };
        let mut p_audio_client: *mut c_void = std::ptr::null_mut();

        let hr = unsafe {
            ((*device_vtbl).Activate)(
                p_device,
                &IID_IAUDIO_CLIENT,
                CLSCTX_ALL,
                std::ptr::null_mut(),
                &mut p_audio_client,
            )
        };

        if hr != 0 || p_audio_client.is_null() {
            unsafe {
                ((*device_vtbl).Release)(p_device);
                ((*enumerator_vtbl).Release)(p_enumerator);
                CoUninitialize();
            }
            return;
        }

        let audio_client_vtbl = unsafe { *(p_audio_client as *mut *const IAudioClientVtbl) };
        let mut p_format: *mut WAVEFORMATEX = std::ptr::null_mut();
        let hr = unsafe { ((*audio_client_vtbl).GetMixFormat)(p_audio_client, &mut p_format) };

        if hr != 0 || p_format.is_null() {
            unsafe {
                ((*audio_client_vtbl).Release)(p_audio_client);
                ((*device_vtbl).Release)(p_device);
                ((*enumerator_vtbl).Release)(p_enumerator);
                CoUninitialize();
            }
            return;
        }

        let channels = unsafe { (*p_format).nChannels as usize };
        let is_float = unsafe { (*p_format).wBitsPerSample == 32 };

        // 100ms buffer duration (1_000_000 in 100ns units)
        let hr = unsafe {
            ((*audio_client_vtbl).Initialize)(
                p_audio_client,
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                1_000_000,
                0,
                p_format,
                std::ptr::null(),
            )
        };

        unsafe { CoTaskMemFree(p_format as *mut c_void) };

        if hr != 0 {
            unsafe {
                ((*audio_client_vtbl).Release)(p_audio_client);
                ((*device_vtbl).Release)(p_device);
                ((*enumerator_vtbl).Release)(p_enumerator);
                CoUninitialize();
            }
            return;
        }

        let mut p_capture: *mut c_void = std::ptr::null_mut();
        let hr = unsafe {
            ((*audio_client_vtbl).GetService)(
                p_audio_client,
                &IID_IAUDIO_CAPTURE_CLIENT,
                &mut p_capture,
            )
        };

        if hr != 0 || p_capture.is_null() {
            unsafe {
                ((*audio_client_vtbl).Release)(p_audio_client);
                ((*device_vtbl).Release)(p_device);
                ((*enumerator_vtbl).Release)(p_enumerator);
                CoUninitialize();
            }
            return;
        }

        let capture_vtbl = unsafe { *(p_capture as *mut *const IAudioCaptureClientVtbl) };
        unsafe { ((*audio_client_vtbl).Start)(p_audio_client) };
        ACTIVE.store(true, Ordering::SeqCst);

        let mut sample_ring_buffer = vec![0.0f32; 512];
        let mut prev_bins = vec![0.0f32; NUM_BINS];

        while ENABLED.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(33)); // ~30 fps cadence

            let mut packet_size = 0u32;
            let _ = unsafe { ((*capture_vtbl).GetNextPacketSize)(p_capture, &mut packet_size) };

            while packet_size > 0 {
                let mut p_data: *mut u8 = std::ptr::null_mut();
                let mut num_frames = 0u32;
                let mut flags = 0u32;

                let hr = unsafe {
                    ((*capture_vtbl).GetBuffer)(
                        p_capture,
                        &mut p_data,
                        &mut num_frames,
                        &mut flags,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                };

                if hr == 0 && !p_data.is_null() && num_frames > 0 {
                    // AUDCLNT_BUFFERFLAGS_SILENT = 0x2
                    let is_silent_flag = (flags & 0x2) != 0;
                    if !is_silent_flag && is_float {
                        let float_slice = unsafe {
                            std::slice::from_raw_parts(p_data as *const f32, (num_frames as usize) * channels)
                        };
                        for frame in float_slice.chunks(channels) {
                            let mono = if channels > 1 {
                                (frame[0] + frame[1]) * 0.5
                            } else {
                                frame[0]
                            };
                            sample_ring_buffer.remove(0);
                            sample_ring_buffer.push(mono);
                        }
                    } else if is_silent_flag {
                        for s in sample_ring_buffer.iter_mut() {
                            *s = 0.0;
                        }
                    }
                    unsafe { ((*capture_vtbl).ReleaseBuffer)(p_capture, num_frames) };
                }

                let _ = unsafe { ((*capture_vtbl).GetNextPacketSize)(p_capture, &mut packet_size) };
            }

            let reduced = REDUCED_MOTION.load(Ordering::Relaxed);
            let snapshot = analyze_samples(&sample_ring_buffer, &prev_bins, reduced);
            prev_bins = snapshot.bins.clone();

            if let Ok(mut snap_guard) = SNAPSHOT.lock() {
                *snap_guard = Some(snapshot.clone());
            }

            let _ = app.emit("visualizer", snapshot);
        }

        // Clean COM teardown on stop
        ACTIVE.store(false, Ordering::SeqCst);
        unsafe {
            let _ = ((*audio_client_vtbl).Stop)(p_audio_client);
            let _ = ((*capture_vtbl).Release)(p_capture);
            let _ = ((*audio_client_vtbl).Release)(p_audio_client);
            let _ = ((*device_vtbl).Release)(p_device);
            let _ = ((*enumerator_vtbl).Release)(p_enumerator);
            CoUninitialize();
        }
    }
}

/// Starts the visualizer monitor loop.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if ENABLED.load(Ordering::Relaxed) {
                #[cfg(windows)]
                native_wasapi::run_loopback_capture(app.clone());

                #[cfg(not(windows))]
                {
                    ACTIVE.store(true, Ordering::SeqCst);
                    while ENABLED.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        let snap = VisualizerSnapshot::default();
                        let _ = app.emit("visualizer", snap);
                    }
                    ACTIVE.store(false, Ordering::SeqCst);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
