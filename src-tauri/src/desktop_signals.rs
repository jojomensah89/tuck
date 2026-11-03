//! Read-only desktop peeks. Only lock-key state and foreground input-language identity
//! are examined; arbitrary key presses and typed text are never collected.
use serde::Serialize;
#[cfg(windows)]
use std::sync::{mpsc, Mutex};
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
static SERVICE_TX: Mutex<Option<mpsc::Sender<native::Signal>>> = Mutex::new(None);

#[derive(Clone, Serialize)]
pub struct DesktopPeek {
    pub title: String,
    pub text: String,
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows::{
        core::{implement, w, BSTR, PCWSTR, VARIANT},
        Win32::{
            Media::Audio::{
                eConsole, eRender, EDataFlow, ERole,
                Endpoints::{
                    IAudioEndpointVolume, IAudioEndpointVolumeCallback,
                    IAudioEndpointVolumeCallback_Impl,
                },
                IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
                MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE,
            },
            System::{
                Com::{
                    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL,
                    COINIT_MULTITHREADED,
                },
                Wmi::{
                    IWbemLocator, IWbemServices, WbemLocator, WBEM_FLAG_FORWARD_ONLY,
                    WBEM_FLAG_RETURN_IMMEDIATELY,
                },
            },
            UI::{
                Input::KeyboardAndMouse::{GetKeyState, GetKeyboardLayout, VK_CAPITAL},
                Shell::PropertiesSystem::PROPERTYKEY,
                WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
            },
        },
    };

    #[derive(Clone, Copy)]
    pub enum Signal {
        Volume(u32, bool),
        Rebind,
        Suspend,
        Stop,
    }

    fn callback_send(sender: &mpsc::Sender<Signal>, signal: Signal) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = sender.send(signal);
        }));
        if result.is_err() {
            eprintln!("Audio callback panicked");
        }
    }

    #[implement(IAudioEndpointVolumeCallback)]
    struct VolumeCallback {
        sender: mpsc::Sender<Signal>,
    }

    #[allow(non_snake_case)]
    impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
        fn OnNotify(
            &self,
            notification: *mut AUDIO_VOLUME_NOTIFICATION_DATA,
        ) -> windows::core::Result<()> {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if let Some(data) = unsafe { notification.as_ref() } {
                    callback_send(
                        &self.sender,
                        Signal::Volume(
                            (data.fMasterVolume * 100.0).round().clamp(0.0, 100.0) as u32,
                            data.bMuted.as_bool(),
                        ),
                    );
                }
            }));
            if result.is_err() {
                eprintln!("IAudioEndpointVolume OnNotify panicked");
            }
            Ok(())
        }
    }

    #[implement(IMMNotificationClient)]
    struct DeviceCallback {
        sender: mpsc::Sender<Signal>,
    }

    #[allow(non_snake_case)]
    impl IMMNotificationClient_Impl for DeviceCallback_Impl {
        fn OnDeviceStateChanged(
            &self,
            _device_id: &PCWSTR,
            _state: DEVICE_STATE,
        ) -> windows::core::Result<()> {
            Ok(())
        }

        fn OnDeviceAdded(&self, _device_id: &PCWSTR) -> windows::core::Result<()> {
            Ok(())
        }

        fn OnDeviceRemoved(&self, _device_id: &PCWSTR) -> windows::core::Result<()> {
            Ok(())
        }

        fn OnDefaultDeviceChanged(
            &self,
            flow: EDataFlow,
            role: ERole,
            _device_id: &PCWSTR,
        ) -> windows::core::Result<()> {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if flow == eRender && role == eConsole {
                    callback_send(&self.sender, Signal::Rebind);
                }
            }));
            if result.is_err() {
                eprintln!("IMMNotificationClient callback panicked");
            }
            Ok(())
        }

        fn OnPropertyValueChanged(
            &self,
            _device_id: &PCWSTR,
            _key: &PROPERTYKEY,
        ) -> windows::core::Result<()> {
            Ok(())
        }
    }

    /// Device enumerator and default-device callback, registered once for the thread's
    /// lifetime so device notifications survive having no output device at all.
    struct DeviceWatcher {
        enumerator: IMMDeviceEnumerator,
        device_callback: IMMNotificationClient,
    }

    impl DeviceWatcher {
        fn new(sender: &mpsc::Sender<Signal>) -> windows::core::Result<Self> {
            unsafe {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                let device_callback: IMMNotificationClient = DeviceCallback {
                    sender: sender.clone(),
                }
                .into();
                enumerator.RegisterEndpointNotificationCallback(&device_callback)?;
                Ok(Self {
                    enumerator,
                    device_callback,
                })
            }
        }
    }

    impl Drop for DeviceWatcher {
        fn drop(&mut self) {
            unsafe {
                if let Err(error) = self
                    .enumerator
                    .UnregisterEndpointNotificationCallback(&self.device_callback)
                {
                    eprintln!("Audio device callback cleanup: {error}");
                }
            }
        }
    }

    /// Volume subscription on the current default render endpoint; rebuilt on rebind.
    struct EndpointSubscription {
        endpoint: IAudioEndpointVolume,
        volume_callback: IAudioEndpointVolumeCallback,
        device_id: String,
    }

    impl EndpointSubscription {
        fn new(
            enumerator: &IMMDeviceEnumerator,
            sender: &mpsc::Sender<Signal>,
        ) -> windows::core::Result<Self> {
            unsafe {
                let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
                let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
                let id = device.GetId()?;
                let device_id = id.to_string();
                windows::Win32::System::Com::CoTaskMemFree(Some(id.0 as _));
                let device_id = device_id?;
                let volume_callback: IAudioEndpointVolumeCallback = VolumeCallback {
                    sender: sender.clone(),
                }
                .into();
                endpoint.RegisterControlChangeNotify(&volume_callback)?;
                Ok(Self {
                    endpoint,
                    volume_callback,
                    device_id,
                })
            }
        }

        fn volume(&self) -> Option<(u32, bool)> {
            unsafe {
                Some((
                    (self.endpoint.GetMasterVolumeLevelScalar().ok()? * 100.0).round() as u32,
                    self.endpoint.GetMute().ok()?.as_bool(),
                ))
            }
        }
    }

    impl Drop for EndpointSubscription {
        fn drop(&mut self) {
            unsafe {
                if let Err(error) = self
                    .endpoint
                    .UnregisterControlChangeNotify(&self.volume_callback)
                {
                    eprintln!("Volume callback cleanup: {error}");
                }
            }
        }
    }

    fn subscribe_endpoint(
        watcher: Option<&DeviceWatcher>,
        sender: &mpsc::Sender<Signal>,
    ) -> Option<EndpointSubscription> {
        // No default render device is a normal state, not an error worth logging.
        watcher.and_then(|watcher| EndpointSubscription::new(&watcher.enumerator, sender).ok())
    }

    fn brightness_service() -> Option<IWbemServices> {
        unsafe {
            let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_ALL).ok()?;
            locator
                .ConnectServer(
                    &BSTR::from("ROOT\\WMI"),
                    &BSTR::new(),
                    &BSTR::new(),
                    &BSTR::new(),
                    0,
                    &BSTR::new(),
                    None,
                )
                .ok()
        }
    }
    fn wifi_connected() -> Option<bool> {
        use windows::Win32::{
            Foundation::HANDLE,
            NetworkManagement::WiFi::{
                wlan_interface_state_connected, WlanCloseHandle, WlanEnumInterfaces,
                WlanFreeMemory, WlanOpenHandle,
            },
        };
        unsafe {
            let mut handle = HANDLE::default();
            let mut version = 0;
            if WlanOpenHandle(2, None, &mut version, &mut handle) != 0 {
                return None;
            }
            let mut list = std::ptr::null_mut();
            let result = if WlanEnumInterfaces(handle, None, &mut list) == 0 && !list.is_null() {
                let count = (*list).dwNumberOfItems as usize;
                let value = if count > 0 && count <= 32 {
                    Some(
                        std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), count)
                            .iter()
                            .any(|interface| interface.isState == wlan_interface_state_connected),
                    )
                } else {
                    None
                };
                WlanFreeMemory(list as _);
                value
            } else {
                None
            };
            let _ = WlanCloseHandle(handle, None);
            result
        }
    }
    fn brightness(service: &IWbemServices) -> Option<u32> {
        unsafe {
            let query = service
                .ExecQuery(
                    &BSTR::from("WQL"),
                    &BSTR::from(
                        "SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active=TRUE",
                    ),
                    WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                    None,
                )
                .ok()?;
            let mut objects = [None];
            let mut count = 0;
            if query.Next(100, &mut objects, &mut count).is_err() || count == 0 {
                return None;
            }
            let mut value = VARIANT::default();
            objects[0]
                .as_ref()?
                .Get(w!("CurrentBrightness"), 0, &mut value, None, None)
                .ok()?;
            u32::try_from(&value).ok().filter(|value| *value <= 100)
        }
    }
    pub fn run(app: AppHandle, receiver: mpsc::Receiver<Signal>, sender: mpsc::Sender<Signal>) {
        unsafe {
            if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                return;
            }
        }
        let brightness_service = brightness_service();
        let mut watcher = DeviceWatcher::new(&sender)
            .map_err(|error| eprintln!("Audio device watcher: {error}"))
            .ok();
        let mut audio = subscribe_endpoint(watcher.as_ref(), &sender);
        let mut previous_volume = audio.as_ref().and_then(EndpointSubscription::volume);
        let mut previous_device = audio.as_ref().map(|value| value.device_id.clone());
        let mut previous_brightness = brightness_service.as_ref().and_then(brightness);
        let mut previous_wifi = wifi_connected();
        let mut caps = unsafe { GetKeyState(VK_CAPITAL.0 as i32) } & 1 != 0;
        let mut layout = None;
        let mut cycles = 0u32;
        loop {
            match receiver.recv_timeout(std::time::Duration::from_millis(250)) {
                Ok(Signal::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(Signal::Suspend) => audio = None,
                Ok(Signal::Rebind) => {
                    drop(audio.take());
                    if watcher.is_none() {
                        watcher = DeviceWatcher::new(&sender)
                            .map_err(|error| eprintln!("Audio device watcher: {error}"))
                            .ok();
                    }
                    let next = subscribe_endpoint(watcher.as_ref(), &sender);
                    let next_device = next.as_ref().map(|value| value.device_id.clone());
                    if previous_device.is_some() && next_device != previous_device {
                        crate::visualizer::notify_output_changed();
                        peek(&app, "Audio output", "Playback device changed");
                    }
                    previous_device = next_device;
                    previous_volume = next.as_ref().and_then(EndpointSubscription::volume);
                    audio = next;
                }
                Ok(Signal::Volume(level, muted)) => {
                    let current = (level, muted);
                    if previous_volume.is_some_and(|old| old != current) {
                        peek(
                            &app,
                            "Volume",
                            &if muted {
                                "Muted".into()
                            } else {
                                format!("{level}%")
                            },
                        );
                    }
                    previous_volume = Some(current);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }

            let next_caps = unsafe { GetKeyState(VK_CAPITAL.0 as i32) } & 1 != 0;
            if caps != next_caps {
                caps = next_caps;
                peek(&app, "Caps Lock", if caps { "On" } else { "Off" });
            }
            let thread = unsafe { GetWindowThreadProcessId(GetForegroundWindow(), None) };
            let next_layout = unsafe { GetKeyboardLayout(thread) }.0 as usize & 0xffff;
            if let Some(old) = layout {
                if old != next_layout {
                    peek(&app, "Keyboard language", "Input language changed");
                }
            }
            layout = Some(next_layout);
            cycles += 1;
            if cycles.is_multiple_of(8) {
                if let Some(current) = wifi_connected() {
                    if previous_wifi.is_some_and(|previous| previous != current) {
                        peek(
                            &app,
                            "Wi-Fi",
                            if current { "Connected" } else { "Disconnected" },
                        );
                    }
                    previous_wifi = Some(current);
                }
                if let Some(current) = brightness_service.as_ref().and_then(brightness) {
                    if previous_brightness.is_some_and(|previous| previous != current) {
                        peek(&app, "Brightness", &format!("{current}%"));
                    }
                    previous_brightness = Some(current);
                }
            }
        }
        drop(audio);
        drop(watcher);
        unsafe {
            CoUninitialize();
        }
    }
    fn peek(app: &AppHandle, title: &str, text: &str) {
        let _ = app.emit(
            "desktop-peek",
            DesktopPeek {
                title: title.into(),
                text: text.into(),
            },
        );
    }
}
pub fn start(app: AppHandle) {
    #[cfg(windows)]
    {
        let (sender, receiver) = mpsc::channel();
        if let Ok(mut guard) = SERVICE_TX.lock() {
            *guard = Some(sender.clone());
        }
        if let Err(error) = std::thread::Builder::new()
            .name("tuck-desktop-signals".into())
            .spawn(move || native::run(app, receiver, sender))
        {
            eprintln!("Could not start desktop signals: {error}");
        }
    }
}

#[cfg(windows)]
fn send(signal: native::Signal) {
    if let Ok(guard) = SERVICE_TX.lock() {
        if let Some(sender) = guard.as_ref() {
            let _ = sender.send(signal);
        }
    }
}

pub fn suspend() {
    #[cfg(windows)]
    send(native::Signal::Suspend);
}

pub fn resume() {
    #[cfg(windows)]
    send(native::Signal::Rebind);
}

pub fn stop() {
    #[cfg(windows)]
    send(native::Signal::Stop);
}
