//! Best-effort WebView2 trimming, without suspending notification delivery.
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::WebviewWindow;
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(windows)]
fn memory_target(window: &WebviewWindow, low: bool, generation: u64) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    use webview_core::Interface;
    let _ = window.with_webview(move |platform| unsafe {
        if GENERATION.load(Ordering::Acquire) != generation {
            return;
        }
        let result = (|| {
            let view = platform.controller().CoreWebView2()?;
            let view: ICoreWebView2_19 = view.cast()?;
            view.SetMemoryUsageTargetLevel(if low {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
            } else {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
            })
        })();
        if let Err(error) = result {
            eprintln!("WebView memory target: {error}");
        }
    });
}
#[cfg(not(windows))]
fn memory_target(_window: &WebviewWindow, _low: bool, _generation: u64) {}

#[tauri::command]
pub fn island_activity(window: WebviewWindow, active: bool) {
    if window.label() != "main" {
        return;
    }
    let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    if active {
        memory_target(&window, false, generation);
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        if GENERATION.load(Ordering::Acquire) == generation {
            memory_target(&window, true, generation);
        }
    });
}
