//! The Vulkan loader (vulkan-1.dll) that ggml's Vulkan backend calls on x64.
//! It comes with the graphics driver, not with Windows, and an old one lacks
//! Vulkan 1.1's functions: imported at load time, that stopped Hex before
//! `main` with a missing-entry-point dialog (#1240). The executable now
//! delay-loads it (build.rs), and `check` asks the loader for its version
//! before anything touches ggml. Without a Vulkan 1.2 loader, which ggml
//! needs, ggml never registers its Vulkan backend, so nothing calls into the
//! DLL and speech runs on the CPU. Tuck renders its UI with WebView2.

use std::sync::atomic::{AtomicBool, Ordering};

/// This launch found no Vulkan 1.2 loader.
static MISSING: AtomicBool = AtomicBool::new(false);

/// After `restrict_dll_search`, before any thread starts: keeps ggml off
/// Vulkan unless the loader is new enough to run it.
pub(crate) fn check() {
    static CHECK: std::sync::Once = std::sync::Once::new();
    CHECK.call_once(check_once);
}
fn check_once() {
    if loader_supported(loader_version()) {
        return;
    }
    MISSING.store(true, Ordering::Relaxed);
    unsafe extern "C" {
        fn _putenv_s(name: *const std::ffi::c_char, value: *const std::ffi::c_char) -> i32;
    }
    // ggml reads the C runtime's copy of the environment, which
    // `std::env::set_var` doesn't update. SAFETY: two NUL-terminated
    // strings, before any other thread starts.
    unsafe { _putenv_s(c"GGML_DISABLE_VULKAN".as_ptr(), c"1".as_ptr()) };
}

fn loader_supported(version: Option<(u32, u32)>) -> bool {
    version.is_some_and(|v| v >= (1, 2))
}
/// This launch transcribes on the CPU because the graphics driver's Vulkan
/// is missing or too old.
pub(crate) fn missing() -> bool {
    MISSING.load(Ordering::Relaxed)
}

/// The loader's Vulkan version, or `None` without vulkan-1.dll or with a
/// Vulkan 1.0 loader, which lacks `vkEnumerateInstanceVersion`.
fn loader_version() -> Option<(u32, u32)> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    let name: Vec<u16> = "vulkan-1.dll\0".encode_utf16().collect();
    // SAFETY: a NUL-terminated name. The library stays loaded for ggml.
    let module = unsafe { LoadLibraryW(windows::core::PCWSTR(name.as_ptr())) }.ok()?;
    if module.is_invalid() {
        return None;
    }
    // SAFETY: a loaded module and a NUL-terminated name.
    unsafe {
        GetProcAddress(
            module,
            windows::core::PCSTR(c"vkGetPhysicalDeviceFeatures2".as_ptr().cast()),
        )
    }?;
    // SAFETY: as above.
    let enumerate = unsafe {
        GetProcAddress(
            module,
            windows::core::PCSTR(c"vkEnumerateInstanceVersion".as_ptr().cast()),
        )
    }?;
    // SAFETY: Vulkan's `VkResult vkEnumerateInstanceVersion(uint32_t*)`.
    let enumerate: unsafe extern "system" fn(*mut u32) -> i32 =
        unsafe { std::mem::transmute(enumerate) };
    let mut version = 0u32;
    // SAFETY: writes one u32; 0 is VK_SUCCESS.
    if unsafe { enumerate(&mut version) } != 0 {
        return None;
    }
    Some(((version >> 22) & 0x7f, (version >> 12) & 0x3ff))
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_and_old_loaders_use_cpu() {
        assert!(!super::loader_supported(None));
        assert!(!super::loader_supported(Some((1, 0))));
        assert!(!super::loader_supported(Some((1, 1))));
        assert!(super::loader_supported(Some((1, 2))));
        assert!(super::loader_supported(Some((1, 4))));
    }
}
