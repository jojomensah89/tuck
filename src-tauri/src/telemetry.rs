use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;

#[cfg(windows)]
static ADAPTERS_DIRTY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
#[cfg(windows)]
pub(crate) fn invalidate_adapters() {
    ADAPTERS_DIRTY.store(true, std::sync::atomic::Ordering::Release);
}

const MAX_HISTORY_POINTS: usize = 60;
#[cfg(windows)]
#[path = "gpu_counters.rs"]
mod gpu_counters;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CpuStats {
    pub usage_percent: f32,
    pub frequency_mhz: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RamStats {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub usage_percent: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GpuStats {
    pub adapter_id: String,
    pub name: String,
    pub vendor: String,
    pub usage_percent: Option<f32>,
    pub memory_used_bytes: Option<u64>,
    pub memory_total_bytes: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryHistoryPoint {
    pub timestamp_ms: u64,
    pub cpu_percent: Option<f32>,
    pub ram_percent: Option<f32>,
    pub gpu_percent: Option<f32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TelemetrySnapshot {
    /// Status: "warming_up", "ready", "stale", or "unavailable"
    pub status: String,
    pub cpu: Option<CpuStats>,
    pub ram: Option<RamStats>,
    pub gpus: Vec<GpuStats>,
    pub history: Vec<TelemetryHistoryPoint>,
    pub sample_time_ms: u64,
    pub network_down_bps: Option<f64>,
    pub network_up_bps: Option<f64>,
}

impl Default for TelemetrySnapshot {
    fn default() -> Self {
        Self {
            status: "warming_up".into(),
            cpu: None,
            ram: None,
            gpus: Vec::new(),
            history: Vec::new(),
            sample_time_ms: now_ms(),
            network_down_bps: None,
            network_up_bps: None,
        }
    }
}

type Request = std::sync::mpsc::Sender<TelemetrySnapshot>;
static REQUESTS: Mutex<Option<std::sync::mpsc::Sender<Request>>> = Mutex::new(None);
static SNAPSHOT: Mutex<Option<TelemetrySnapshot>> = Mutex::new(None);
static HISTORY: Mutex<Option<VecDeque<TelemetryHistoryPoint>>> = Mutex::new(None);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn telemetry_now() -> Result<TelemetrySnapshot, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    REQUESTS
        .lock()
        .map_err(|_| "Telemetry unavailable")?
        .as_ref()
        .ok_or("Telemetry not ready")?
        .send(tx)
        .map_err(|_| "Telemetry stopped")?;
    rx.recv_timeout(std::time::Duration::from_secs(3))
        .map_err(|_| "Telemetry sample timed out".into())
}

#[tauri::command]
pub fn telemetry_history() -> Result<Vec<TelemetryHistoryPoint>, String> {
    let history_guard = HISTORY
        .lock()
        .map_err(|_| "Failed to lock telemetry history")?;
    Ok(history_guard
        .as_ref()
        .map(|deque| deque.iter().cloned().collect())
        .unwrap_or_default())
}

#[cfg(windows)]
#[allow(non_snake_case, dead_code, clippy::upper_case_acronyms)]
mod native_windows {
    use super::*;
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Copy, Clone, Default)]
    pub struct FILETIME {
        pub dwLowDateTime: u32,
        pub dwHighDateTime: u32,
    }

    impl FILETIME {
        pub fn as_u64(&self) -> u64 {
            ((self.dwHighDateTime as u64) << 32) | (self.dwLowDateTime as u64)
        }
    }

    #[repr(C)]
    pub struct MEMORYSTATUSEX {
        pub dwLength: u32,
        pub dwMemoryLoad: u32,
        pub ullTotalPhys: u64,
        pub ullAvailPhys: u64,
        pub ullTotalPageFile: u64,
        pub ullAvailPageFile: u64,
        pub ullTotalVirtual: u64,
        pub ullAvailVirtual: u64,
        pub ullAvailExtendedVirtual: u64,
    }

    #[repr(C)]
    struct GUID {
        Data1: u32,
        Data2: u16,
        Data3: u16,
        Data4: [u8; 8],
    }

    const IID_IDXGI_FACTORY: GUID = GUID {
        Data1: 0x7b7166ec,
        Data2: 0x21c7,
        Data3: 0x44ae,
        Data4: [0xb2, 0x1a, 0xc9, 0xae, 0x32, 0x1a, 0xe3, 0x69],
    };

    #[repr(C)]
    struct DXGI_ADAPTER_DESC {
        Description: [u16; 128],
        VendorId: u32,
        DeviceId: u32,
        SubSysId: u32,
        Revision: u32,
        DedicatedVideoMemory: usize,
        DedicatedSystemMemory: usize,
        SharedSystemMemory: usize,
        AdapterLuid: i64,
    }

    type HRESULT = i32;

    #[repr(C)]
    struct IDXGIFactoryVtbl {
        QueryInterface: usize,
        AddRef: unsafe extern "system" fn(this: *mut c_void) -> u32,
        Release: unsafe extern "system" fn(this: *mut c_void) -> u32,
        SetPrivateData: usize,
        SetPrivateDataInterface: usize,
        GetPrivateData: usize,
        GetParent: usize,
        EnumAdapters: unsafe extern "system" fn(
            this: *mut c_void,
            Adapter: u32,
            ppAdapter: *mut *mut c_void,
        ) -> HRESULT,
    }

    #[repr(C)]
    struct IDXGIAdapterVtbl {
        QueryInterface: usize,
        AddRef: unsafe extern "system" fn(this: *mut c_void) -> u32,
        Release: unsafe extern "system" fn(this: *mut c_void) -> u32,
        SetPrivateData: usize,
        SetPrivateDataInterface: usize,
        GetPrivateData: usize,
        GetParent: usize,
        EnumOutputs: usize,
        GetDesc:
            unsafe extern "system" fn(this: *mut c_void, pDesc: *mut DXGI_ADAPTER_DESC) -> HRESULT,
    }

    extern "system" {
        fn GetSystemTimes(
            lpIdleTime: *mut FILETIME,
            lpKernelTime: *mut FILETIME,
            lpUserTime: *mut FILETIME,
        ) -> i32;

        fn GlobalMemoryStatusEx(lpBuffer: *mut MEMORYSTATUSEX) -> i32;
        fn LoadLibraryW(lpLibFileName: *const u16) -> *mut c_void;
        fn GetProcAddress(hModule: *mut c_void, lpProcName: *const u8) -> *mut c_void;
        fn FreeLibrary(hLibModule: *mut c_void) -> i32;
    }

    pub fn sample_cpu(prev_times: &mut Option<(u64, u64, u64)>) -> Option<CpuStats> {
        let mut idle = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();

        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return None;
        }

        let idle_u = idle.as_u64();
        let kernel_u = kernel.as_u64();
        let user_u = user.as_u64();

        let stats = if let Some((prev_idle, prev_kernel, prev_user)) = *prev_times {
            let kernel_diff = kernel_u.saturating_sub(prev_kernel);
            let user_diff = user_u.saturating_sub(prev_user);
            let idle_diff = idle_u.saturating_sub(prev_idle);
            let total_diff = kernel_diff.saturating_add(user_diff);

            if total_diff > 0 {
                let busy_diff = total_diff.saturating_sub(idle_diff);
                let pct = (busy_diff as f64 / total_diff as f64 * 100.0) as f32;
                Some(CpuStats {
                    usage_percent: pct.clamp(0.0, 100.0),
                    frequency_mhz: None,
                })
            } else {
                None
            }
        } else {
            // Warm-up on first cycle: previous sample does not exist yet.
            None
        };

        *prev_times = Some((idle_u, kernel_u, user_u));
        stats
    }

    pub fn sample_ram() -> Option<RamStats> {
        let mut mem = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            dwMemoryLoad: 0,
            ullTotalPhys: 0,
            ullAvailPhys: 0,
            ullTotalPageFile: 0,
            ullAvailPageFile: 0,
            ullTotalVirtual: 0,
            ullAvailVirtual: 0,
            ullAvailExtendedVirtual: 0,
        };

        if unsafe { GlobalMemoryStatusEx(&mut mem) } == 0 {
            return None;
        }

        let total = mem.ullTotalPhys;
        let free = mem.ullAvailPhys;
        let used = total.saturating_sub(free);
        let pct = if total > 0 {
            (used as f64 / total as f64 * 100.0) as f32
        } else {
            0.0
        };

        Some(RamStats {
            total_bytes: total,
            used_bytes: used,
            free_bytes: free,
            usage_percent: pct.clamp(0.0, 100.0),
        })
    }

    pub fn sample_gpus() -> Vec<GpuStats> {
        let mut list = Vec::new();

        // Dynamically load dxgi.dll to enumerate display adapters cleanly
        let dll_name: Vec<u16> = "dxgi.dll\0".encode_utf16().collect();
        let h_module = unsafe { LoadLibraryW(dll_name.as_ptr()) };
        if h_module.is_null() {
            return list;
        }

        let proc_name = b"CreateDXGIFactory\0";
        let create_factory_ptr = unsafe { GetProcAddress(h_module, proc_name.as_ptr()) };
        if !create_factory_ptr.is_null() {
            type CreateDXGIFactoryFn = unsafe extern "system" fn(
                riid: *const GUID,
                ppFactory: *mut *mut c_void,
            ) -> HRESULT;
            let create_factory: CreateDXGIFactoryFn =
                unsafe { std::mem::transmute(create_factory_ptr) };

            let mut p_factory: *mut c_void = std::ptr::null_mut();
            let hr = unsafe { create_factory(&IID_IDXGI_FACTORY, &mut p_factory) };
            if hr == 0 && !p_factory.is_null() {
                let factory_vtbl = unsafe { *(p_factory as *mut *const IDXGIFactoryVtbl) };
                let mut adapter_index = 0u32;

                loop {
                    let mut p_adapter: *mut c_void = std::ptr::null_mut();
                    let enum_hr = unsafe {
                        ((*factory_vtbl).EnumAdapters)(p_factory, adapter_index, &mut p_adapter)
                    };
                    if enum_hr != 0 || p_adapter.is_null() {
                        break;
                    }

                    let adapter_vtbl = unsafe { *(p_adapter as *mut *const IDXGIAdapterVtbl) };
                    let mut desc: DXGI_ADAPTER_DESC = unsafe { std::mem::zeroed() };
                    let desc_hr = unsafe { ((*adapter_vtbl).GetDesc)(p_adapter, &mut desc) };

                    if desc_hr == 0 {
                        // Extract wide string Description
                        let len = desc
                            .Description
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(desc.Description.len());
                        let name = String::from_utf16_lossy(&desc.Description[..len])
                            .trim()
                            .to_string();

                        // Filter out Microsoft Basic Render Driver software fallback if physical GPUs exist
                        let vendor = match desc.VendorId {
                            0x10DE => "NVIDIA",
                            0x1002 => "AMD",
                            0x8086 => "Intel",
                            0x1414 => "Microsoft",
                            _ => "Unknown",
                        };

                        let vram = desc.DedicatedVideoMemory as u64;

                        // GPU engine aggregation note:
                        // GPUs provide multiple separate hardware engines (3D, Video Decode, Copy, Compute).
                        // Arbitrarily summing per-engine percentages is mathematically invalid and causes false >100% spikes.
                        // When PDH utilization counters are not active, we provide None rather than a fabricated zero.
                        list.push(GpuStats {
                            adapter_id: format!(
                                "luid_0x{:08x}_0x{:08x}",
                                (desc.AdapterLuid as u64 >> 32) as u32,
                                desc.AdapterLuid as u32
                            ),
                            name,
                            vendor: vendor.to_string(),
                            usage_percent: None,
                            memory_used_bytes: None,
                            memory_total_bytes: if vram > 0 { Some(vram) } else { None },
                        });
                    }

                    unsafe {
                        ((*adapter_vtbl).Release)(p_adapter);
                    }
                    adapter_index += 1;
                }

                unsafe {
                    ((*factory_vtbl).Release)(p_factory);
                }
            }
        }

        unsafe {
            FreeLibrary(h_module);
        }

        if list.iter().any(|gpu| gpu.vendor != "Microsoft") {
            list.retain(|gpu| gpu.vendor != "Microsoft");
        }
        list
    }
}

/// Starts the dedicated 1000ms background telemetry poller.
pub fn start(_app: AppHandle) {
    let (tx, rx) = std::sync::mpsc::channel::<Request>();
    *REQUESTS.lock().unwrap_or_else(|p| p.into_inner()) = Some(tx);
    std::thread::spawn(move || {
        let mut prev_times: Option<(u64, u64, u64)> = None;
        let mut first_cycle = true;
        let mut network = crate::network_rates::Sampler::default();
        let mut last_sample = std::time::Instant::now();
        #[cfg(windows)]
        let mut gpu_counters = gpu_counters::GpuCounters::new();

        #[cfg(windows)]
        let mut adapters = Vec::new();
        while let Ok(reply) = rx.recv() {
            // CPU deltas after a long idle would describe the whole gap, not current load.
            if last_sample.elapsed() > std::time::Duration::from_secs(2) {
                prev_times = None;
                first_cycle = true;
            }
            last_sample = std::time::Instant::now();
            #[cfg(windows)]
            let (cpu, ram, gpus) = {
                let cpu = native_windows::sample_cpu(&mut prev_times);
                let ram = native_windows::sample_ram();
                if ADAPTERS_DIRTY.swap(false, std::sync::atomic::Ordering::AcqRel) {
                    adapters = native_windows::sample_gpus();
                }
                let mut gpus = adapters.clone();
                if let Some(counters) = gpu_counters.as_mut() {
                    counters.sample(&mut gpus);
                }
                (cpu, ram, gpus)
            };

            #[cfg(not(windows))]
            let (cpu, ram, gpus): (Option<CpuStats>, Option<RamStats>, Vec<GpuStats>) =
                (None, None, Vec::new());

            let status = if first_cycle {
                first_cycle = false;
                "warming_up".to_string()
            } else if cpu.is_some() || ram.is_some() {
                "ready".to_string()
            } else {
                "unavailable".to_string()
            };

            let timestamp = now_ms();
            let point = TelemetryHistoryPoint {
                timestamp_ms: timestamp,
                cpu_percent: cpu.as_ref().map(|c| c.usage_percent),
                ram_percent: ram.as_ref().map(|r| r.usage_percent),
                gpu_percent: gpus
                    .iter()
                    .filter_map(|g| g.usage_percent)
                    .max_by(|a, b| a.total_cmp(b)),
            };

            // Update bounded circular history buffer
            if let Ok(mut hist_guard) = HISTORY.lock() {
                let deque = hist_guard.get_or_insert_with(VecDeque::new);
                if deque.len() >= MAX_HISTORY_POINTS {
                    deque.pop_front();
                }
                deque.push_back(point);
            }

            let history_copy: Vec<TelemetryHistoryPoint> = HISTORY
                .lock()
                .ok()
                .and_then(|h| h.as_ref().map(|d| d.iter().cloned().collect()))
                .unwrap_or_default();

            let rates = network.sample();
            let snapshot = TelemetrySnapshot {
                status,
                cpu,
                ram,
                gpus,
                history: history_copy,
                sample_time_ms: timestamp,
                network_down_bps: rates.map(|r| r.0),
                network_up_bps: rates.map(|r| r.1),
            };

            if let Ok(mut snap_guard) = SNAPSHOT.lock() {
                *snap_guard = Some(snapshot.clone());
            }

            let _ = reply.send(snapshot);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    #[ignore = "Read-only live Windows hardware diagnostic"]
    fn live_windows_telemetry() {
        let mut previous = None;
        let _ = native_windows::sample_cpu(&mut previous);
        let mut counters = gpu_counters::GpuCounters::new();
        std::thread::sleep(std::time::Duration::from_secs(1));
        let cpu = native_windows::sample_cpu(&mut previous).expect("CPU sample");
        let ram = native_windows::sample_ram().expect("RAM sample");
        let mut gpus = native_windows::sample_gpus();
        if let Some(counters) = counters.as_mut() {
            counters.sample(&mut gpus);
        }
        assert!((0.0..=100.0).contains(&cpu.usage_percent));
        assert!(ram.total_bytes > 0 && ram.used_bytes <= ram.total_bytes);
        println!(
            "LIVE_TELEMETRY {}",
            serde_json::json!({"cpu":cpu,"ram":ram,"gpus":gpus})
        );
    }

    #[test]
    fn test_telemetry_snapshot_default() {
        let snap = TelemetrySnapshot::default();
        assert_eq!(snap.status, "warming_up");
        assert!(snap.cpu.is_none());
        assert!(snap.ram.is_none());
        assert!(snap.gpus.is_empty());
    }

    #[test]
    fn test_history_circular_limit() {
        let mut deque = VecDeque::new();
        for i in 0..100 {
            if deque.len() >= MAX_HISTORY_POINTS {
                deque.pop_front();
            }
            deque.push_back(TelemetryHistoryPoint {
                timestamp_ms: i,
                cpu_percent: Some(i as f32),
                ram_percent: Some(50.0),
                gpu_percent: None,
            });
        }
        assert_eq!(deque.len(), MAX_HISTORY_POINTS);
        assert_eq!(deque.front().unwrap().timestamp_ms, 40);
        assert_eq!(deque.back().unwrap().timestamp_ms, 99);
    }

    #[test]
    fn test_telemetry_serialization() {
        let snap = TelemetrySnapshot {
            status: "ready".into(),
            cpu: Some(CpuStats {
                usage_percent: 18.5,
                frequency_mhz: Some(3600),
            }),
            ram: Some(RamStats {
                total_bytes: 16_000_000_000,
                used_bytes: 8_000_000_000,
                free_bytes: 8_000_000_000,
                usage_percent: 50.0,
            }),
            gpus: vec![GpuStats {
                adapter_id: "fixture".into(),
                name: "RTX 4070".into(),
                vendor: "NVIDIA".into(),
                usage_percent: Some(34.2),
                memory_used_bytes: Some(4_000_000_000),
                memory_total_bytes: Some(12_000_000_000),
            }],
            history: vec![],
            sample_time_ms: 123456789,
            network_down_bps: None,
            network_up_bps: None,
        };

        let json = serde_json::to_string(&snap).expect("serialize");
        assert!(json.contains("\"usagePercent\":18.5"));
        assert!(json.contains("\"totalBytes\":16000000000"));
        assert!(json.contains("\"sampleTimeMs\":123456789"));
    }
}
