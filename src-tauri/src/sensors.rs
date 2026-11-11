//! Local microphone control and capability-use indicators. No usage history is retained.
use serde::{Deserialize, Serialize};
use std::sync::{LazyLock, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    shortcut: String,
    indicator: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            shortcut: "Ctrl+Alt+M".into(),
            indicator: true,
        }
    }
}
#[derive(Clone, Default, Serialize, PartialEq)]
pub struct Snapshot {
    muted: bool,
    microphone: Vec<String>,
    camera: Vec<String>,
}
static SETTINGS: LazyLock<Mutex<Settings>> = LazyLock::new(|| Mutex::new(Settings::default()));
static WAKE: std::sync::Condvar = std::sync::Condvar::new();
static SNAPSHOT: LazyLock<Mutex<Snapshot>> = LazyLock::new(|| Mutex::new(Snapshot::default()));
static PAUSE_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[tauri::command]
pub fn sensor_shortcuts_pause(app: AppHandle, paused: bool) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let generation = PAUSE_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    PAUSED.store(paused, Ordering::Release);
    if let Ok(key) = Shortcut::try_from(sensor_settings().shortcut.as_str()) {
        if paused {
            app.global_shortcut()
                .unregister(key)
                .map_err(|e| e.to_string())?;
        } else if !app.global_shortcut().is_registered(key) {
            app.global_shortcut()
                .register(key)
                .map_err(|e| e.to_string())?;
        }
    }
    if paused {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(30));
            if PAUSED.load(Ordering::Acquire)
                && PAUSE_GENERATION.load(Ordering::Acquire) == generation
            {
                let _ = sensor_shortcuts_pause(app, false);
            }
        });
    }
    Ok(())
}
#[tauri::command]
pub fn sensor_settings() -> Settings {
    SETTINGS.lock().unwrap().clone()
}
#[tauri::command]
pub fn sensor_snapshot() -> Snapshot {
    SNAPSHOT.lock().unwrap().clone()
}
fn path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("sensors.json"))
}
#[tauri::command]
pub fn sensor_configure(app: AppHandle, settings: Settings) -> Result<(), String> {
    let mut prior = SETTINGS.lock().map_err(|e| e.to_string())?;
    let next = if settings.shortcut.is_empty() {
        None
    } else {
        Some(Shortcut::try_from(settings.shortcut.as_str()).map_err(|e| e.to_string())?)
    };
    if settings.shortcut != prior.shortcut {
        if let Some(key) = next {
            app.global_shortcut()
                .register(key)
                .map_err(|e| e.to_string())?;
        }
    }
    let save = (|| {
        let path = path(&app)?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, serde_json::to_vec(&settings).unwrap()).map_err(|e| e.to_string())?;
        std::fs::rename(temp, path).map_err(|e| e.to_string())
    })();
    if let Err(error) = save {
        if settings.shortcut != prior.shortcut {
            if let Some(key) = next {
                let _ = app.global_shortcut().unregister(key);
            }
        }
        return Err(error);
    }
    if settings.shortcut != prior.shortcut {
        if let Ok(key) = Shortcut::try_from(prior.shortcut.as_str()) {
            let _ = app.global_shortcut().unregister(key);
        }
    }
    *prior = settings;
    WAKE.notify_all();
    if !prior.indicator {
        native::clear_registry_watches();
    }
    if !prior.indicator {
        let mut state = SNAPSHOT.lock().unwrap();
        state.microphone.clear();
        state.camera.clear();
        let _ = app.emit("sensors", state.clone());
    }
    Ok(())
}
pub fn toggle_target(states: &[bool]) -> bool {
    !states.iter().all(|muted| *muted)
}
pub fn in_use(stop: Option<u64>, name: &str, own: &str) -> Option<String> {
    if stop != Some(0) {
        return None;
    }
    let path = name.replace('#', "\\");
    if path.eq_ignore_ascii_case(own) {
        return None;
    }
    Some(path.rsplit('\\').next().unwrap_or(&path).to_string())
}
pub fn handle_shortcut(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if PAUSED.load(std::sync::atomic::Ordering::Acquire)
        || event.state != ShortcutState::Pressed
        || Shortcut::try_from(sensor_settings().shortcut.as_str())
            .ok()
            .as_ref()
            != Some(shortcut)
    {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        static ACTION: Mutex<()> = Mutex::new(());
        let _action = ACTION.lock().unwrap();
        match native::mute(true) {
            Ok(muted) => {
                let _ = app.emit("microphone-toggle", muted);
            }
            Err(error) => {
                let _ = app.emit("microphone-error", error);
            }
        }
    });
}

pub fn ensure_unmuted() -> Result<(), String> {
    capture_allowed(native::mute(false)?)
}
fn capture_allowed(muted: bool) -> Result<(), String> {
    if muted {
        Err("Microphone is muted".into())
    } else {
        Ok(())
    }
}
pub fn start(app: AppHandle) {
    if let Ok(path) = path(&app) {
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(settings) = serde_json::from_slice(&bytes) {
                *SETTINGS.lock().unwrap() = settings;
            }
        }
    }
    if let Ok(shortcut) = Shortcut::try_from(sensor_settings().shortcut.as_str()) {
        if let Err(error) = app.global_shortcut().register(shortcut) {
            eprintln!("Microphone shortcut: {error}");
        }
    }
    native::watch(app.clone());
    std::thread::spawn(move || loop {
        let mut next = Snapshot {
            muted: SNAPSHOT.lock().unwrap().muted,
            ..Snapshot::default()
        };
        // Hold the settings lock during reads so disabling waits for any in-flight read.
        let settings = WAKE
            .wait_while(SETTINGS.lock().unwrap(), |settings| !settings.indicator)
            .unwrap();
        {
            if settings.indicator {
                let cached = SNAPSHOT.lock().unwrap().clone();
                next.microphone = native::changed_apps("microphone").unwrap_or(cached.microphone);
                next.camera = native::changed_apps("webcam").unwrap_or(cached.camera);
            }
        }
        let mut state = SNAPSHOT.lock().unwrap();
        next.muted = state.muted;
        if *state != next {
            *state = next;
            let _ = app.emit("sensors", state.clone());
        }
        drop(state);
        let _wait = WAKE
            .wait_timeout(settings, std::time::Duration::from_secs(2))
            .unwrap();
    });
}
#[cfg(windows)]
mod native {
    use super::*;
    use windows::Win32::{
        Media::Audio::{
            eCapture, eCommunications, eConsole, Endpoints::IAudioEndpointVolume,
            IMMDeviceEnumerator, MMDeviceEnumerator,
        },
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
        },
    };
    use windows::{
        core::{implement, PCWSTR},
        Win32::{
            Media::Audio::{
                EDataFlow, ERole,
                Endpoints::{IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl},
                IMMNotificationClient, IMMNotificationClient_Impl, AUDIO_VOLUME_NOTIFICATION_DATA,
                DEVICE_STATE,
            },
            UI::Shell::PropertiesSystem::PROPERTYKEY,
        },
    };
    #[implement(IAudioEndpointVolumeCallback)]
    struct VolumeCallback(std::sync::mpsc::Sender<()>);
    #[allow(non_snake_case)]
    impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
        fn OnNotify(&self, _: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
            let _ = self.0.send(());
            Ok(())
        }
    }
    #[implement(IMMNotificationClient)]
    struct DeviceCallback(std::sync::mpsc::Sender<()>);
    #[allow(non_snake_case)]
    impl IMMNotificationClient_Impl for DeviceCallback_Impl {
        fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnDefaultDeviceChanged(
            &self,
            flow: EDataFlow,
            _: ERole,
            _: &PCWSTR,
        ) -> windows::core::Result<()> {
            if flow == eCapture {
                let _ = self.0.send(());
            }
            Ok(())
        }
    }
    pub fn watch(app: AppHandle) {
        std::thread::spawn(move || unsafe {
            if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                return;
            }
            let Ok(enumerator): Result<IMMDeviceEnumerator, _> =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            else {
                CoUninitialize();
                return;
            };
            let (tx, rx) = std::sync::mpsc::channel();
            let devices: IMMNotificationClient = DeviceCallback(tx.clone()).into();
            let callback: IAudioEndpointVolumeCallback = VolumeCallback(tx).into();
            if enumerator
                .RegisterEndpointNotificationCallback(&devices)
                .is_err()
            {
                CoUninitialize();
                return;
            }
            let mut endpoints: Vec<IAudioEndpointVolume> = Vec::new();
            loop {
                for endpoint in endpoints.drain(..) {
                    let _ = endpoint.UnregisterControlChangeNotify(&callback);
                }
                for role in [eConsole, eCommunications] {
                    if let Ok(device) = enumerator.GetDefaultAudioEndpoint(eCapture, role) {
                        if let Ok(endpoint) =
                            device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                        {
                            let _ = endpoint.RegisterControlChangeNotify(&callback);
                            endpoints.push(endpoint);
                        }
                    }
                }
                let muted = endpoints
                    .iter()
                    .any(|endpoint| endpoint.GetMute().is_ok_and(|value| value.as_bool()));
                {
                    let mut state = SNAPSHOT.lock().unwrap();
                    if state.muted != muted {
                        state.muted = muted;
                        let _ = app.emit("sensors", state.clone());
                    }
                }
                if rx.recv().is_err() {
                    break;
                }
                while rx.try_recv().is_ok() {}
            }
            for endpoint in endpoints {
                let _ = endpoint.UnregisterControlChangeNotify(&callback);
            }
            let _ = enumerator.UnregisterEndpointNotificationCallback(&devices);
            CoUninitialize();
        });
    }
    pub fn mute(toggle: bool) -> Result<bool, String> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|e| e.to_string())?;
            struct Com;
            impl Drop for Com {
                fn drop(&mut self) {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
            let _com = Com;
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| e.to_string())?;
            let mut endpoints: Vec<IAudioEndpointVolume> = Vec::new();
            for role in [eConsole, eCommunications] {
                let device = enumerator
                    .GetDefaultAudioEndpoint(eCapture, role)
                    .map_err(|e| e.to_string())?;
                endpoints.push(
                    device
                        .Activate(CLSCTX_ALL, None)
                        .map_err(|e| e.to_string())?,
                );
            }
            let states = endpoints
                .iter()
                .map(|v| v.GetMute().map(|v| v.as_bool()).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            if toggle {
                let target = toggle_target(&states);
                for endpoint in endpoints {
                    endpoint
                        .SetMute(target, std::ptr::null())
                        .map_err(|e| e.to_string())?;
                }
                Ok(target)
            } else {
                Ok(states.iter().any(|v| *v))
            }
        }
    }
    // A notification is rearmed before reading, so changes during enumeration are retained.
    struct RegistryWatch {
        key: Option<winreg::RegKey>,
        event: isize,
    }
    impl Drop for RegistryWatch {
        fn drop(&mut self) {
            drop(self.key.take());
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(
                    windows::Win32::Foundation::HANDLE(self.event as _),
                );
            }
        }
    }
    static WATCHES: LazyLock<Mutex<std::collections::HashMap<String, RegistryWatch>>> =
        LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));
    pub fn clear_registry_watches() {
        WATCHES.lock().unwrap().clear();
    }
    pub fn changed_apps(capability: &str) -> Option<Vec<String>> {
        use windows::Win32::{
            Foundation::{HANDLE, WAIT_OBJECT_0},
            System::{
                Registry::{
                    RegNotifyChangeKeyValue, HKEY, REG_NOTIFY_CHANGE_LAST_SET,
                    REG_NOTIFY_CHANGE_NAME,
                },
                Threading::{CreateEventW, WaitForSingleObject},
            },
        };
        use winreg::{
            enums::{HKEY_CURRENT_USER, KEY_READ},
            RegKey,
        };
        let mut watches = WATCHES.lock().unwrap();
        let initial = !watches.contains_key(capability);
        if initial {
            let path = format!("Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\{capability}");
            let key = RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey_with_flags(path, KEY_READ)
                .ok()?;
            let event = unsafe { CreateEventW(None, false, false, None).ok()? };
            watches.insert(
                capability.into(),
                RegistryWatch {
                    key: Some(key),
                    event: event.0 as isize,
                },
            );
        }
        let watch = watches.get(capability)?;
        if !initial && unsafe { WaitForSingleObject(HANDLE(watch.event as _), 0) } != WAIT_OBJECT_0
        {
            return None;
        }
        let result = unsafe {
            RegNotifyChangeKeyValue(
                HKEY(watch.key.as_ref()?.raw_handle() as _),
                true,
                REG_NOTIFY_CHANGE_LAST_SET | REG_NOTIFY_CHANGE_NAME,
                HANDLE(watch.event as _),
                true,
            )
        };
        if result.is_err() {
            watches.remove(capability);
            return Some(Vec::new());
        }
        Some(apps(capability))
    }
    fn package_name(family: &str) -> Option<String> {
        use windows::{
            core::HSTRING, ApplicationModel::Package, Management::Deployment::PackageManager,
        };
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
        }
        let name = (|| {
            if Package::Current()
                .and_then(|p| p.Id())
                .and_then(|id| id.FamilyName())
                .is_ok_and(|own| own.to_string().eq_ignore_ascii_case(family))
            {
                return None;
            }
            let packages = PackageManager::new()
                .ok()?
                .FindPackagesByUserSecurityIdPackageFamilyName(
                    &HSTRING::new(),
                    &HSTRING::from(family),
                )
                .ok()?;
            for package in packages {
                if let Ok(name) = package.DisplayName() {
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
            Some(family.to_string())
        })();
        unsafe {
            CoUninitialize();
        }
        name
    }
    fn apps(capability: &str) -> Vec<String> {
        use winreg::{
            enums::{HKEY_CURRENT_USER, KEY_READ},
            RegKey,
        };
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let path = format!("Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\{capability}");
        let Ok(key) = root.open_subkey_with_flags(path, KEY_READ) else {
            return Vec::new();
        };
        let own = std::env::current_exe()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut result = Vec::new();
        for parent in [Some(&key), None] {
            let nested;
            let parent = match parent {
                Some(key) => key,
                None => {
                    let Ok(key) = key.open_subkey_with_flags("NonPackaged", KEY_READ) else {
                        continue;
                    };
                    nested = key;
                    &nested
                }
            };
            for name in parent.enum_keys().flatten() {
                if name == "NonPackaged" {
                    continue;
                }
                if let Ok(app) = parent.open_subkey_with_flags(&name, KEY_READ) {
                    if let Some(name) = in_use(
                        app.get_value::<u64, _>("LastUsedTimeStop").ok(),
                        &name,
                        &own,
                    ) {
                        if name.ends_with(".exe") {
                            result.push(name);
                        } else if let Some(display) = package_name(&name) {
                            result.push(display);
                        }
                    }
                }
            }
        }
        result.sort();
        result.dedup();
        result
    }
}
#[cfg(not(windows))]
mod native {
    pub fn watch(_: super::AppHandle) {}
    pub fn mute(_: bool) -> Result<bool, String> {
        Err("Microphone control requires Windows".into())
    }
    pub fn clear_registry_watches() {}
    pub fn changed_apps(_: &str) -> Option<Vec<String>> {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn muted_capture_fails_with_actionable_error() {
        assert_eq!(capture_allowed(true), Err("Microphone is muted".into()));
        assert_eq!(capture_allowed(false), Ok(()));
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(settings.indicator);
        assert_eq!(settings.shortcut, "Ctrl+Alt+M");
    }
    #[test]
    fn mute_decision() {
        assert!(toggle_target(&[false, true]));
        assert!(!toggle_target(&[true, true]));
    }
    #[test]
    fn registry_interpretation() {
        assert_eq!(
            in_use(Some(0), "C:#Apps#teams.exe", "C:\\Tuck.exe"),
            Some("teams.exe".into())
        );
        assert_eq!(in_use(None, "app", "tuck"), None);
        assert_eq!(in_use(Some(3), "app", "tuck"), None);
        assert_eq!(in_use(Some(0), "C:#Tuck.exe", "c:\\tuck.exe"), None);
    }
}
