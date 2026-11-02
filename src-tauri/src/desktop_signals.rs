//! Read-only desktop peeks. Only lock-key state and foreground input-language identity
//! are examined; arbitrary key presses and typed text are never collected.
use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize)]
pub struct DesktopPeek {
    pub title: String,
    pub text: String,
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows::{
        core::{w, BSTR, VARIANT},
        Win32::{
            Media::Audio::{
                eConsole, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator,
                MMDeviceEnumerator,
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
                WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
            },
        },
    };
    fn volume() -> Option<(u32, bool, String)> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
            let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
            let id = device.GetId().ok()?;
            let name = id.to_string().ok();
            windows::Win32::System::Com::CoTaskMemFree(Some(id.0 as _));
            Some((
                (endpoint.GetMasterVolumeLevelScalar().ok()? * 100.0).round() as u32,
                endpoint.GetMute().ok()?.as_bool(),
                name?,
            ))
        }
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
    pub fn run(app: AppHandle) {
        unsafe {
            if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                return;
            }
        }
        let brightness_service = brightness_service();
        let mut previous_volume = volume();
        let mut previous_brightness = brightness_service.as_ref().and_then(brightness);
        let mut previous_wifi = wifi_connected();
        let mut caps = unsafe { GetKeyState(VK_CAPITAL.0 as i32) } & 1 != 0;
        let mut layout = None;
        let mut cycles = 0u32;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            if let Some(current) = volume() {
                if let Some(old) = &previous_volume {
                    if old.2 != current.2 {
                        crate::visualizer::notify_output_changed();
                        peek(&app, "Audio output", "Playback device changed");
                    } else if old.0 != current.0 || old.1 != current.1 {
                        peek(
                            &app,
                            "Volume",
                            &if current.1 {
                                "Muted".into()
                            } else {
                                format!("{}%", current.0)
                            },
                        );
                    }
                }
                previous_volume = Some(current);
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
        #[allow(unreachable_code)]
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
    std::thread::spawn(move || native::run(app));
}
