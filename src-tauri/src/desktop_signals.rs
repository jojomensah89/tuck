//! Desktop peeks and the source of the volume/brightness HUD. Only lock-key state,
//! volume-key state (when a level changes) and foreground input-language identity are
//! examined, plus the time of the last input; arbitrary key presses and typed text are
//! never collected. Volume and brightness are event-driven
//! (`IAudioEndpointVolumeCallback`, WMI `WmiMonitorBrightnessEvent`); neither is polled.
use serde::Serialize;
use std::sync::mpsc;
#[cfg(windows)]
use std::sync::Mutex;
use tauri::AppHandle;
#[cfg(windows)]
use tauri::Emitter;

#[cfg(windows)]
static SERVICE_TX: Mutex<Option<mpsc::Sender<native::Signal>>> = Mutex::new(None);

#[derive(Clone, Serialize)]
pub struct DesktopPeek {
    pub title: String,
    pub text: String,
}

/// Payload of the `system-level` event that drives the island's volume/brightness HUD.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SystemLevel {
    /// `volume` or `brightness`.
    pub kind: &'static str,
    /// 0-100.
    pub level: u32,
    /// Always false for brightness.
    pub muted: bool,
}

impl SystemLevel {
    pub const fn volume(level: u32, muted: bool) -> Self {
        Self {
            kind: "volume",
            level,
            muted,
        }
    }

    pub const fn brightness(level: u32) -> Self {
        Self {
            kind: "brightness",
            level,
            muted: false,
        }
    }
}

/// Volume change requested by the island (wheel or middle click).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeRequest {
    /// Move by this many percent, snapped to the step grid (see `step_volume`).
    Step(i32),
    ToggleMute,
}

/// Next volume after a wheel step. Values off the grid snap to it first, so 37% moves
/// to 38% or 36% with a step of 2, and to 40% or 30% with a step of 10.
pub fn step_volume(current: u32, delta: i32) -> u32 {
    let current = current.min(100) as i32;
    let step = delta.unsigned_abs().clamp(1, 100) as i32;
    let next = match delta.signum() {
        1 => (current / step + 1) * step,
        -1 if current % step == 0 => current - step,
        -1 => current / step * step,
        _ => current,
    };
    next.clamp(0, 100) as u32
}

/// Brightness HUDs need input this recent; see `brightness_from_user`.
pub(crate) const BRIGHTNESS_INPUT_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether a brightness change looks user-driven. Brightness keys and the Quick Settings
/// slider come with input; idle dimming happens while nobody is typing. Unknown input
/// age shows the HUD. Laptops whose brightness keys bypass Windows input entirely only
/// get a HUD while in use, unless the Windows flyout is hidden: the island is then the
/// only feedback, so a missed key is worse than a stray HUD.
pub(crate) fn brightness_shows_hud(
    input_age: Option<std::time::Duration>,
    flyout_hidden: bool,
) -> bool {
    flyout_hidden || input_age.is_none_or(|age| age <= BRIGHTNESS_INPUT_WINDOW)
}

/// Brightness steps this small can be a ramp (adaptive brightness, charger changes).
/// Brightness keys normally move further.
pub(crate) const RAMP_MAX_STEP: u32 = 3;
/// Small steps this close together form a ramp, however recent the last input was.
pub(crate) const RAMP_MAX_GAP: std::time::Duration = std::time::Duration::from_secs(1);

/// What to do with one brightness change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RampVerdict {
    /// A real step: show it now.
    Show,
    /// A small step that may start a ramp: `BrightnessRamp::due` releases it if none follows.
    Hold,
    /// Part of a ramp.
    Ignore,
}

/// Tells runs of small closely spaced brightness steps from single changes.
#[derive(Debug, Default)]
pub(crate) struct BrightnessRamp {
    last_change: Option<std::time::Instant>,
    held: Option<(u32, std::time::Instant)>,
}

impl BrightnessRamp {
    /// `previous` is the level before this change, when known.
    pub(crate) fn observe(
        &mut self,
        previous: Option<u32>,
        level: u32,
        now: std::time::Instant,
    ) -> RampVerdict {
        let close = self
            .last_change
            .and_then(|at| now.checked_duration_since(at))
            .is_some_and(|gap| gap <= RAMP_MAX_GAP);
        self.last_change = Some(now);
        let small = previous.is_some_and(|before| before.abs_diff(level) <= RAMP_MAX_STEP);
        if !small {
            self.held = None;
            RampVerdict::Show
        } else if close {
            self.held = None;
            RampVerdict::Ignore
        } else {
            self.held = Some((level, now + RAMP_MAX_GAP));
            RampVerdict::Hold
        }
    }

    /// A held step that no other step followed, once that is certain.
    pub(crate) fn due(&mut self, now: std::time::Instant) -> Option<u32> {
        let (level, at) = self.held?;
        if now < at {
            return None;
        }
        self.held = None;
        Some(level)
    }
}

/// Milliseconds between two `GetTickCount` readings, across the 49.7-day wrap.
pub(crate) fn input_age_from_ticks(last_input: u32, now: u32) -> std::time::Duration {
    std::time::Duration::from_millis(u64::from(now.wrapping_sub(last_input)))
}

/// Time since the last user input anywhere; `None` when it cannot be read.
pub(crate) fn user_input_age() -> Option<std::time::Duration> {
    #[cfg(windows)]
    return native::input_age();
    #[cfg(not(windows))]
    None
}

/// Whether a volume key is down or was just pressed. Only those three keys are read.
pub(crate) fn volume_key_active() -> bool {
    #[cfg(windows)]
    return native::volume_key_active();
    #[cfg(not(windows))]
    false
}

/// Whether a step left the level stuck although it could have moved: the device only
/// takes coarse steps, so `VolumeStepUp`/`VolumeStepDown` should finish the job.
pub(crate) fn needs_step_fallback(before: u32, after: u32, delta: i32) -> bool {
    match delta.signum() {
        1 => before < 100 && after <= before,
        -1 => before > 0 && after >= before,
        _ => false,
    }
}

/// Wheel steps the `volume_step` command accepts: non-zero, at most 100 either way.
pub(crate) fn valid_volume_step(delta: i32) -> bool {
    delta != 0 && delta.unsigned_abs() <= 100
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows::{
        core::{implement, w, Interface, BSTR, GUID, PCWSTR, VARIANT},
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
                    CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CoUninitialize,
                    CLSCTX_ALL, CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, EOAC_NONE,
                    RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
                },
                Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE},
                Wmi::{
                    IUnsecuredApartment, IWbemClassObject, IWbemLocator, IWbemObjectSink,
                    IWbemObjectSink_Impl, IWbemServices, WbemLocator, WBEM_FLAG_FORWARD_ONLY,
                    WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE,
                },
            },
            UI::{
                Input::KeyboardAndMouse::{
                    GetAsyncKeyState, GetKeyState, GetKeyboardLayout, GetLastInputInfo,
                    LASTINPUTINFO, VK_CAPITAL, VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
                },
                Shell::PropertiesSystem::PROPERTYKEY,
                WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
            },
        },
    };

    pub enum Signal {
        Volume(u32, bool),
        Brightness(u32),
        /// Default render device changed: rebuild the volume subscription.
        Rebind,
        /// Back from sleep: rebuild everything released on suspend.
        Resume,
        Suspend,
        Adjust(VolumeRequest, mpsc::Sender<Option<SystemLevel>>),
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

    /// Receives `WmiMonitorBrightnessEvent` instances. WMI calls it on its own RPC
    /// threads; it only forwards the new level to the signals thread.
    #[implement(IWbemObjectSink)]
    struct BrightnessSink {
        sender: mpsc::Sender<Signal>,
    }

    fn event_brightness(object: &IWbemClassObject) -> Option<u32> {
        let mut value = VARIANT::default();
        unsafe {
            object
                .Get(w!("Brightness"), 0, &mut value, None, None)
                .ok()?;
        }
        // The property is a uint8; the conversion widens it (VariantToUInt32).
        u32::try_from(&value).ok().filter(|level| *level <= 100)
    }

    #[allow(non_snake_case)]
    impl IWbemObjectSink_Impl for BrightnessSink_Impl {
        fn Indicate(
            &self,
            count: i32,
            objects: *const Option<IWbemClassObject>,
        ) -> windows::core::Result<()> {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if objects.is_null() || count <= 0 {
                    return;
                }
                let objects = unsafe { std::slice::from_raw_parts(objects, count as usize) };
                // A burst carries one instance per step; only the latest level matters.
                if let Some(level) = objects.iter().rev().flatten().find_map(event_brightness) {
                    callback_send(&self.sender, Signal::Brightness(level));
                }
            }));
            if result.is_err() {
                eprintln!("Brightness event sink panicked");
            }
            Ok(())
        }

        fn SetStatus(
            &self,
            _flags: i32,
            _result: windows::core::HRESULT,
            _param: &BSTR,
            _object: Option<&IWbemClassObject>,
        ) -> windows::core::Result<()> {
            Ok(())
        }
    }

    /// WMI brightness event subscription for internal panels. External monitors
    /// (DDC/CI) are out of scope. Cancelled on drop (suspend, shutdown).
    struct BrightnessWatch {
        service: IWbemServices,
        sink: IWbemObjectSink,
    }

    /// CLSID_UnsecuredApartment (wbemcli.h); the bindings only carry the interface.
    const UNSECURED_APARTMENT: GUID = GUID::from_u128(0x49bd2028_1523_11d1_ad79_00c04fd8fdff);

    impl BrightnessWatch {
        fn new(service: IWbemServices, sender: &mpsc::Sender<Signal>) -> Option<Self> {
            let direct: IWbemObjectSink = BrightnessSink {
                sender: sender.clone(),
            }
            .into();
            // Microsoft's recommended route for async WMI sinks: unsecapp.exe receives the
            // calls from the WMI service, so its access check against this process's COM
            // security cannot silently drop events. Fall back to the direct sink.
            let stub = unsafe {
                CoCreateInstance::<_, IUnsecuredApartment>(
                    &UNSECURED_APARTMENT,
                    None,
                    CLSCTX_LOCAL_SERVER,
                )
                .and_then(|apartment| apartment.CreateObjectStub(&direct))
                .and_then(|stub| stub.cast::<IWbemObjectSink>())
            };
            let sink = match stub {
                Ok(stub) => stub,
                Err(error) => {
                    eprintln!("Unsecured apartment unavailable, using a direct sink: {error}");
                    direct
                }
            };
            unsafe {
                service.ExecNotificationQueryAsync(
                    &BSTR::from("WQL"),
                    &BSTR::from("SELECT * FROM WmiMonitorBrightnessEvent"),
                    WBEM_GENERIC_FLAG_TYPE(0),
                    None,
                    &sink,
                )
            }
            .map_err(|error| eprintln!("Brightness events: {error}"))
            .ok()?;
            Some(Self { service, sink })
        }
    }

    impl Drop for BrightnessWatch {
        fn drop(&mut self) {
            unsafe {
                if let Err(error) = self.service.CancelAsyncCall(&self.sink) {
                    eprintln!("Brightness events cleanup: {error}");
                }
            }
        }
    }

    /// Subscribes whenever the `ROOT\WMI` service connects. The first read only seeds
    /// the previous level: the provider can be slow to answer right after sign-in or
    /// resume, and a missed first read must not cost the HUD for the whole session.
    /// Machines with only external monitors simply never receive an event.
    fn watch_brightness(sender: &mpsc::Sender<Signal>) -> (Option<BrightnessWatch>, Option<u32>) {
        let Some(service) = brightness_service() else {
            return (None, None);
        };
        let level = brightness(&service);
        (BrightnessWatch::new(service, sender), level)
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
                    (self.endpoint.GetMasterVolumeLevelScalar().ok()? * 100.0)
                        .round()
                        .clamp(0.0, 100.0) as u32,
                    self.endpoint.GetMute().ok()?.as_bool(),
                ))
            }
        }

        /// Applies a wheel step or mute toggle and returns the resulting state. The
        /// endpoint callback still fires, so the HUD follows changes from any source.
        fn adjust(&self, request: VolumeRequest) -> Option<(u32, bool)> {
            let (level, muted) = self.volume()?;
            unsafe {
                match request {
                    VolumeRequest::Step(delta) => {
                        let next = step_volume(level, delta);
                        self.endpoint
                            .SetMasterVolumeLevelScalar(next as f32 / 100.0, std::ptr::null())
                            .ok()?;
                        // Coarse-step devices round a small change back to where it was;
                        // their own step always moves.
                        let after = self.volume().map_or(level, |(after, _)| after);
                        if needs_step_fallback(level, after, delta) {
                            let _ = if delta > 0 {
                                self.endpoint.VolumeStepUp(std::ptr::null())
                            } else {
                                self.endpoint.VolumeStepDown(std::ptr::null())
                            };
                        }
                        // Raising the volume unmutes, like the hardware keys.
                        if muted && delta > 0 {
                            self.endpoint.SetMute(false, std::ptr::null()).ok()?;
                        }
                    }
                    VolumeRequest::ToggleMute => {
                        self.endpoint.SetMute(!muted, std::ptr::null()).ok()?;
                    }
                }
            }
            self.volume()
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
            let service = locator
                .ConnectServer(
                    &BSTR::from("ROOT\\WMI"),
                    &BSTR::new(),
                    &BSTR::new(),
                    &BSTR::new(),
                    0,
                    &BSTR::new(),
                    None,
                )
                .ok()?;
            // Without impersonation the brightness provider answers WBEM_E_ACCESS_DENIED
            // (the process default is "identify"), for queries and event subscriptions.
            CoSetProxyBlanket(
                &service,
                RPC_C_AUTHN_WINNT,
                RPC_C_AUTHZ_NONE,
                PCWSTR::null(),
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )
            .map_err(|error| eprintln!("Brightness service security: {error}"))
            .ok()?;
            Some(service)
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
            // Runs only at startup and on resume. The provider often needs more than 100ms on
            // its first call, which comes back as WBEM_S_TIMEDOUT with no objects.
            if query.Next(2000, &mut objects, &mut count).is_err() || count == 0 {
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
        let (mut brightness_watch, mut previous_brightness) = watch_brightness(&sender);
        let mut brightness_ramp = BrightnessRamp::default();
        let mut watcher = DeviceWatcher::new(&sender)
            .map_err(|error| eprintln!("Audio device watcher: {error}"))
            .ok();
        let mut audio = subscribe_endpoint(watcher.as_ref(), &sender);
        let mut previous_volume = audio.as_ref().and_then(EndpointSubscription::volume);
        let mut previous_device = audio.as_ref().map(|value| value.device_id.clone());
        let mut previous_wifi = wifi_connected();
        let mut caps = unsafe { GetKeyState(VK_CAPITAL.0 as i32) } & 1 != 0;
        let mut layout = None;
        let mut cycles = 0u32;
        loop {
            // A flyout lookup scheduled by a level change runs once Explorer settled.
            crate::osd_flyout::tick();
            match receiver.recv_timeout(std::time::Duration::from_millis(250)) {
                Ok(Signal::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(Signal::Suspend) => {
                    audio = None;
                    brightness_watch = None;
                    brightness_ramp = BrightnessRamp::default();
                }
                Ok(Signal::Resume) => {
                    if brightness_watch.is_none() {
                        (brightness_watch, previous_brightness) = watch_brightness(&sender);
                    }
                    let _ = sender.send(Signal::Rebind);
                }
                Ok(Signal::Adjust(request, reply)) => {
                    // The resulting level change is Tuck's, not a flyout key press.
                    crate::osd_flyout::note_self_change();
                    let result = audio
                        .as_ref()
                        .and_then(|audio| audio.adjust(request))
                        .map(|(level, muted)| SystemLevel::volume(level, muted));
                    let _ = reply.send(result);
                }
                Ok(Signal::Brightness(level)) => {
                    if previous_brightness != Some(level) {
                        let verdict = brightness_ramp.observe(
                            previous_brightness,
                            level,
                            std::time::Instant::now(),
                        );
                        if verdict == RampVerdict::Show {
                            emit_brightness(&app, level);
                        }
                        crate::osd_flyout::on_level_change(
                            crate::osd_flyout::LevelChange::Brightness,
                        );
                    }
                    previous_brightness = Some(level);
                }
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
                        emit_level(&app, SystemLevel::volume(level, muted));
                        crate::osd_flyout::on_level_change(crate::osd_flyout::LevelChange::Volume);
                    }
                    previous_volume = Some(current);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(level) = brightness_ramp.due(std::time::Instant::now()) {
                emit_brightness(&app, level);
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
            }
        }
        drop(brightness_watch);
        drop(audio);
        drop(watcher);
        unsafe {
            CoUninitialize();
        }
    }
    /// Whether a volume key is down or was pressed since the last check.
    pub(super) fn volume_key_active() -> bool {
        [VK_VOLUME_MUTE, VK_VOLUME_DOWN, VK_VOLUME_UP]
            .iter()
            .any(|key| unsafe { GetAsyncKeyState(i32::from(key.0)) } != 0)
    }

    /// Time since the last keyboard, mouse or touch input in any app.
    pub(super) fn input_age() -> Option<std::time::Duration> {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
            return None;
        }
        let now = unsafe { windows::Win32::System::SystemInformation::GetTickCount() };
        Some(input_age_from_ticks(info.dwTime, now))
    }

    fn emit_level(app: &AppHandle, level: SystemLevel) {
        let _ = app.emit("system-level", level);
    }

    /// Idle dimming arrives with nobody at the keyboard; see `brightness_shows_hud`.
    fn emit_brightness(app: &AppHandle, level: u32) {
        if brightness_shows_hud(input_age(), crate::osd_flyout::flyout_hidden()) {
            emit_level(app, SystemLevel::brightness(level));
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

    /// Live probe: subscribes to brightness events and waits for one. Change the
    /// brightness while it runs: `cargo test probe_brightness -- --ignored --nocapture`.
    #[cfg(test)]
    #[test]
    #[ignore = "needs an internal panel and a brightness change during the run"]
    fn probe_brightness_events() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let (sender, receiver) = mpsc::channel();
        let (watch, level) = watch_brightness(&sender);
        println!(
            "initial brightness: {level:?}, subscribed: {}",
            watch.is_some()
        );
        assert!(watch.is_some());
        let event = receiver.recv_timeout(std::time::Duration::from_secs(30));
        let Ok(Signal::Brightness(level)) = event else {
            panic!("no brightness event within 30s");
        };
        println!("brightness event: {level}");
        drop(watch);
    }

    /// Live probe: steps the default output up and down, checks the callback, then
    /// restores the exact level. `cargo test probe_volume -- --ignored --nocapture`.
    #[cfg(test)]
    #[test]
    #[ignore = "changes the system volume briefly"]
    fn probe_volume_steps() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let (sender, receiver) = mpsc::channel();
        let watcher = DeviceWatcher::new(&sender).expect("device enumerator");
        let audio = subscribe_endpoint(Some(&watcher), &sender).expect("default output");
        let original = unsafe { audio.endpoint.GetMasterVolumeLevelScalar() }.unwrap();
        let before = audio.volume().unwrap();
        let up = audio.adjust(VolumeRequest::Step(2)).unwrap();
        let down = audio.adjust(VolumeRequest::Step(-2)).unwrap();
        let notified = matches!(
            receiver.recv_timeout(std::time::Duration::from_secs(2)),
            Ok(Signal::Volume(..))
        );
        unsafe {
            let _ = audio
                .endpoint
                .SetMasterVolumeLevelScalar(original, std::ptr::null());
        }
        println!("before {before:?}, up {up:?}, down {down:?}, callback {notified}");
        assert_eq!(up.0, step_volume(before.0, 2));
        assert_eq!(down.0, step_volume(up.0, -2));
        assert!(notified);
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
    send(native::Signal::Resume);
}

pub fn stop() {
    #[cfg(windows)]
    send(native::Signal::Stop);
}

/// Runs a volume request on the signals thread, which owns the endpoint. Waits at most
/// `timeout`; `None` without an output device or before the thread is up.
fn request_volume(request: VolumeRequest, timeout: std::time::Duration) -> Option<SystemLevel> {
    let (reply, response) = mpsc::channel();
    #[cfg(windows)]
    send(native::Signal::Adjust(request, reply));
    #[cfg(not(windows))]
    drop((request, reply));
    response.recv_timeout(timeout).ok().flatten()
}

async fn run_volume_request(request: VolumeRequest) -> Result<SystemLevel, String> {
    tauri::async_runtime::spawn_blocking(move || {
        request_volume(request, std::time::Duration::from_millis(800))
    })
    .await
    .map_err(|_| "Volume request failed".to_string())?
    .ok_or_else(|| "No audio output device".to_string())
}

/// Island wheel: step the system volume by a signed percentage (2, or 10 with Shift).
#[tauri::command]
pub async fn volume_step(delta: i32) -> Result<SystemLevel, String> {
    if !valid_volume_step(delta) {
        return Err("Volume step out of range".into());
    }
    run_volume_request(VolumeRequest::Step(delta)).await
}

/// Island middle click: toggle mute.
#[tauri::command]
pub async fn volume_toggle_mute() -> Result<SystemLevel, String> {
    run_volume_request(VolumeRequest::ToggleMute).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_snap_to_the_grid() {
        assert_eq!(step_volume(36, 2), 38);
        assert_eq!(step_volume(37, 2), 38);
        assert_eq!(step_volume(37, -2), 36);
        assert_eq!(step_volume(36, -2), 34);
        assert_eq!(step_volume(37, 10), 40);
        assert_eq!(step_volume(37, -10), 30);
        assert_eq!(step_volume(40, -10), 30);
    }

    #[test]
    fn steps_clamp_at_the_ends() {
        assert_eq!(step_volume(99, 2), 100);
        assert_eq!(step_volume(100, 2), 100);
        assert_eq!(step_volume(1, -2), 0);
        assert_eq!(step_volume(0, -10), 0);
        assert_eq!(step_volume(250, -2), 98);
        assert_eq!(step_volume(50, 0), 50);
    }

    #[test]
    fn step_validation_survives_extreme_deltas() {
        assert!(!valid_volume_step(0));
        assert!(!valid_volume_step(i32::MIN));
        assert!(!valid_volume_step(i32::MAX));
        assert!(!valid_volume_step(101));
        assert!(!valid_volume_step(-101));
        assert!(valid_volume_step(100));
        assert!(valid_volume_step(-100));
        assert!(valid_volume_step(2));
        // step_volume itself never overflows either
        assert_eq!(step_volume(50, i32::MIN), 0);
        assert_eq!(step_volume(50, i32::MAX), 100);
    }

    #[test]
    fn stuck_steps_fall_back_to_the_device_step() {
        // A coarse device rounded 50 -> 52 back to 50
        assert!(needs_step_fallback(50, 50, 2));
        assert!(needs_step_fallback(50, 50, -2));
        // Rounded the wrong way
        assert!(needs_step_fallback(50, 49, 2));
        assert!(needs_step_fallback(50, 51, -2));
        // Moved as asked, or nowhere to go
        assert!(!needs_step_fallback(50, 52, 2));
        assert!(!needs_step_fallback(50, 48, -2));
        assert!(!needs_step_fallback(100, 100, 2));
        assert!(!needs_step_fallback(0, 0, -2));
        assert!(!needs_step_fallback(50, 50, 0));
    }

    #[test]
    fn brightness_hud_needs_recent_input() {
        use std::time::Duration;
        assert!(brightness_shows_hud(
            Some(Duration::from_millis(200)),
            false
        ));
        assert!(brightness_shows_hud(Some(BRIGHTNESS_INPUT_WINDOW), false));
        assert!(!brightness_shows_hud(Some(Duration::from_secs(120)), false));
        assert!(brightness_shows_hud(None, false));
        // Firmware keys leave no input behind; with the flyout hidden they must still show
        assert!(brightness_shows_hud(Some(Duration::from_secs(120)), true));
        assert_eq!(
            input_age_from_ticks(1_000, 1_500),
            Duration::from_millis(500)
        );
        // GetTickCount wraps after 49.7 days
        assert_eq!(
            input_age_from_ticks(u32::MAX - 99, 100),
            Duration::from_millis(200)
        );
    }

    #[test]
    fn a_run_of_small_close_steps_is_a_ramp() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut ramp = BrightnessRamp::default();
        // The first small step waits: it may be the start of a ramp
        assert_eq!(ramp.observe(Some(50), 49, at(0)), RampVerdict::Hold);
        assert_eq!(ramp.due(at(100)), None);
        // It was: nothing of it ever shows, however long the run lasts
        assert_eq!(ramp.observe(Some(49), 47, at(200)), RampVerdict::Ignore);
        assert_eq!(ramp.observe(Some(47), 46, at(1_100)), RampVerdict::Ignore);
        assert_eq!(ramp.observe(Some(46), 44, at(2_100)), RampVerdict::Ignore);
        assert_eq!(ramp.due(at(10_000)), None);
    }

    #[test]
    fn a_lone_small_step_shows_late_and_a_real_step_shows_at_once() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut ramp = BrightnessRamp::default();
        assert_eq!(ramp.observe(Some(50), 53, at(0)), RampVerdict::Hold);
        assert_eq!(ramp.due(at(999)), None);
        assert_eq!(ramp.due(at(1_000)), Some(53));
        assert_eq!(ramp.due(at(1_001)), None, "Released once");
        // Brightness keys move further, even in quick succession
        assert_eq!(ramp.observe(Some(53), 63, at(5_000)), RampVerdict::Show);
        assert_eq!(ramp.observe(Some(63), 73, at(5_100)), RampVerdict::Show);
        // A key press during a held step replaces it
        assert_eq!(ramp.observe(Some(73), 72, at(9_000)), RampVerdict::Hold);
        assert_eq!(ramp.observe(Some(72), 62, at(9_100)), RampVerdict::Show);
        assert_eq!(ramp.due(at(20_000)), None);
        // Small steps apart from each other are separate changes
        assert_eq!(ramp.observe(Some(62), 61, at(30_000)), RampVerdict::Hold);
        assert_eq!(ramp.due(at(31_000)), Some(61));
        assert_eq!(ramp.observe(Some(61), 60, at(32_001)), RampVerdict::Hold);
        // No known previous level: show it
        let mut fresh = BrightnessRamp::default();
        assert_eq!(fresh.observe(None, 60, at(0)), RampVerdict::Show);
    }

    #[test]
    fn level_payload_shape() {
        let json = serde_json::to_value(SystemLevel::volume(40, true)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "volume", "level": 40, "muted": true})
        );
        assert!(!SystemLevel::brightness(70).muted);
    }
}
