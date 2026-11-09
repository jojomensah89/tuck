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

#[cfg(windows)]
pub fn startup_state() -> Result<windows::ApplicationModel::StartupTaskState, String> {
    windows::ApplicationModel::StartupTask::GetAsync(&windows::core::HSTRING::from("TuckStartup"))
        .and_then(|op| op.get())
        .and_then(|task| task.State())
        .map_err(|e| e.to_string())
}
#[cfg(windows)]
pub fn set_startup(enable: bool) -> Result<(), String> {
    use windows::ApplicationModel::{StartupTask, StartupTaskState};
    let task = StartupTask::GetAsync(&windows::core::HSTRING::from("TuckStartup"))
        .and_then(|op| op.get())
        .map_err(|e| e.to_string())?;
    let state = if enable {
        task.RequestEnableAsync().and_then(|op| op.get())
    } else {
        task.Disable().and_then(|_| task.State())
    }
    .map_err(|e| e.to_string())?;
    if enable && state != StartupTaskState::Enabled && state != StartupTaskState::EnabledByPolicy {
        return Err("Turned off in Task Manager > Startup apps".into());
    }
    Ok(())
}
#[tauri::command]
pub fn autostart_status() -> Result<String, String> {
    #[cfg(windows)]
    if has_package_identity() {
        use windows::ApplicationModel::StartupTaskState;
        return Ok(match startup_state()? {
            StartupTaskState::DisabledByUser => "Turned off in Task Manager > Startup apps",
            StartupTaskState::DisabledByPolicy => {
                "Turned off by your organization (Startup apps policy)"
            }
            _ => "",
        }
        .into());
    }
    Ok(String::new())
}
