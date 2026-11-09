//! Runtime package identity, independent of build configuration.
#[cfg(windows)]
pub fn has_package_identity() -> bool {
    use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
    let mut length = 0;
    let result = unsafe { GetCurrentPackageFullName(&mut length, windows::core::PWSTR::null()) };
    identity_from_status(result.0)
}
#[cfg(not(windows))]
pub fn has_package_identity() -> bool {
    false
}

#[cfg(any(windows, test))]
fn identity_from_status(status: u32) -> bool {
    // ERROR_INSUFFICIENT_BUFFER means a package name is available.
    status == 122 || status == 0
}

#[tauri::command]
pub fn is_store_build() -> bool {
    has_package_identity()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_probe_statuses() {
        assert!(identity_from_status(122));
        assert!(identity_from_status(0));
        assert!(!identity_from_status(15700)); // APPMODEL_ERROR_NO_PACKAGE
        assert!(!identity_from_status(5));
    }
}
