# How notch / dynamic-island apps work, and what Tuck should adopt

Research date: 2026-11-03. Method: shallow clones, static reading only. Nothing was built, run or installed.

Evidence tags used throughout:

- **[seen]** I read it in the cited file and line (paths are relative to `C:\Users\Jojo\Desktop\Projects\tuck-re\`).
- **[inferred]** Reasoned from what I saw, or from general Win32/WinRT knowledge. Not confirmed in a source file.
- **[web]** Confirmed by a web search during this session. Those results were thin, so each is flagged where it matters.
- **[unknown]** Not found. This does not mean it is absent.

## 0. What was inspected

| Dir under `tuck-re/` | Upstream | Commit | Language | License | Notes |
|---|---|---|---|---|---|
| `DynamicWin/` and `DynamicWin-V2/` | FlorianButz/DynamicWin | b8a874b | none | CC BY-SA 4.0 | **No source.** `main` and `V2` hold only README, LICENSE and a gif. The README says "This repository currently only exists to host the releases." |
| `DynamicWin-Legacy/` | 59xa/DynamicWin-Legacy (branch `release`) | a2622d5 | C#, WPF, SkiaSharp | CC BY-SA 4.0 | The real Windows source. Predecessor of DynamicWin V2. About 98 `.cs` files. |
| `boring.notch/` | TheBoredTeam/boring.notch | b7611bb | Swift, SwiftUI | **GPL-3.0** | macOS. Most polished reference. |
| `NotchDrop/` | Lakr233/NotchDrop | e70b3d7 | Swift | MIT | macOS. About 2k lines. The cleanest hover and drop state machine. |
| `Atoll/` | Ebullioscopic/Atoll | 0edae1b | Swift | **GPL-3.0** | macOS. Biggest feature surface: 74 manager files, timer, clipboard, Bluetooth, Focus, privacy dots, LLM-usage quotas, extension RPC. It credits boring.notch as its base. |

**Licensing warning.** Port ideas and APIs, not code. Copying GPL-3.0 or CC BY-SA code into Tuck would impose copyleft on Tuck.

**Closed-source Windows islands: not inspected.** The brief allowed it but I did not do it. DynamicWin V2 ships binaries only, and I did not download any. The Legacy source answers the same questions. The `nusa.exe` overlay that `docs/feature-matrix.md` says is already running on this machine is a local candidate. Inspecting it would need a separate go-ahead.

**Limits of this report.** The Swift apps use macOS-only APIs such as `NSPanel`, `MediaRemote` and `EventKit`. For each one I name the Windows equivalent and the pitfalls. The Windows equivalents are mostly [inferred] from API knowledge, apart from the few [web] checks. Each is something to prove in Tuck, not a verified fact.

---

## 1. What Tuck already does

Read first, so the port plan only proposes deltas. All [seen] in `src-tauri/src/`:

- **Window.**
  - A fixed 520×230 logical transparent canvas, with the island drawn in the DOM.
  - `WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE` (`system.rs:213-219`).
  - Toggles focusability for keyboard entry (`system.rs:223-233`).
- **Hit testing and click-through.** A 40 ms thread polls `GetCursorPos`, runs a DPI-aware hit test and calls `set_ignore_cursor_events(!hovered)` (`system.rs:389-456`).
- **Fullscreen.** `GetForegroundWindow`, an exclusion list (Progman, WorkerW, Shell_TrayWnd), same-monitor check, `SHQueryUserNotificationState` (D3D_FULL_SCREEN or BUSY), then a window-rect vs monitor-rect fallback (`system.rs:271-338`).
- **Layout and power polling.** A 2 s thread re-centres on the primary monitor and reads `GetSystemPowerStatus` (`system.rs:459-470`).
- **Media.**
  - SMTC is polled every 500 ms (`media.rs:418-432`).
  - Artwork is cached per track and retried.
  - Session picking is supported (`media_sessions`, `media_select_session`).
- **Desktop peeks.**
  - Volume via `IAudioEndpointVolume`, polled every 250 ms (`desktop_signals.rs:19-52,144`).
  - Brightness via WMI, Wi-Fi state and Caps Lock via `GetKeyState`.
- **Audio visualizer.** WASAPI loopback (`loopback.rs`, `visualizer.rs`).
- **Clipboard history.** Uses `GetClipboardSequenceNumber`. Honours `ExcludeClipboardContentFromMonitorProcessing` and `CanIncludeInClipboardHistory` (`clipboard_history.rs:527-576`).
- **Extensions** (`extensions.rs`):
  - Radios via `Windows.Devices.Radios` (`:504-612`).
  - `UserNotificationListener` (`:614-690`).
  - Local ICS calendar (`:178`).
  - Downloads scan.
  - Shelf stored as references, capped at 100 entries and 50 per drop (`:369-397`).
  - Drag-out via the `drag` crate (`:452-480`).
- **Also present.** Telemetry (`GetSystemTimes`, `GlobalMemoryStatusEx`, PDH GPU), lyrics, dictation, Claude and Codex usage.

Biggest structural gaps against the references, detailed in §4:

1. Media, volume, power and clipboard are polled rather than event-driven.
2. There is no multi-monitor selection.
3. There is no lock, sleep or display-change handling.
4. There is no TOPMOST re-assertion.
5. There is no auto-open shelf on a system-wide drag.
6. There is no screen-capture exclusion.
7. There is no single-instance guard.
8. Bluetooth device connect and battery events are missing.
9. Mic and camera "in use" indicators are missing.
10. Focus/DND state is missing.
11. A webcam mirror is missing.
12. Per-device notification mirroring needs identity work.

---

## 2. How each app works, by topic

### 2.1 Window setup

**DynamicWin-Legacy (WPF) [seen]**
- Style (`DynamicWin/Main/MainForm.xaml.cs:103-115`):
  - `WindowStyle.None` and `AllowsTransparency = true` (a layered window).
  - `Topmost = true` and `ShowInTaskbar = false`.
  - `ShowActivated=false` in `App.xaml.cs:104-111`.
  - Background is `Transparent` (`MainForm.xaml:8`).
- Alt-Tab exclusion: on `Loaded` it sets `WS_EX_TOOLWINDOW` and clears `WS_EX_APPWINDOW` with `SetWindowLong` (`MainForm.xaml.cs:119-127`).
- The window is **not small**. It is 800 wide and, when "AlwaysTopmost" is on, the full monitor height (`Utils/WindowPositionHelper.cs:53`). It is placed at the monitor's top-centre in DPI-corrected coordinates (`:38-51`).
- Skia paints the island inside it.
- There is **no `WS_EX_TRANSPARENT` and no `WM_NCHITTEST`** anywhere in the repo (grep returned nothing). Click-through of empty areas therefore relies on layered-window per-pixel alpha hit-testing [inferred]. That works for a WPF `AllowsTransparency` window and does **not** carry over to WebView2.
- Topmost is kept by brute force: a 500 ms `System.Timers.Timer` calls `SetWindowPos(HWND_TOPMOST, SWP_NOMOVE|SWP_NOSIZE|SWP_SHOWWINDOW)` (`Main/App.xaml.cs:125-134`, `:190-198`). A later variant also re-centres on every tick (`:285-310`).
- Optional AppBar: it can reserve 40 px of the work area with `SHAppBarMessage(ABM_NEW, ABM_QUERYPOS, ABM_SETPOS)`, edge TOP (`Utils/AppBarHelper.cs:62-88`; toggle at `MainForm.xaml.cs:215-219`).
- Single instance: a named mutex `59xa.DynamicWin` (`App.xaml.cs:65`).
- Tray: a WinForms `NotifyIcon`, with the context menu re-positioned by hand (`MainForm.xaml.cs:477-509`). It drops `Topmost` while the menu is open so the menu is not hidden underneath (`:143-151`).

**boring.notch [seen]**
- `NSPanel` subclass: `isFloatingPanel`, `isOpaque=false`, `backgroundColor=.clear`, `hasShadow=false`, `level = .mainMenu + 3`.
- `collectionBehavior = [.fullScreenAuxiliary, .stationary, .canJoinAllSpaces, .ignoresCycle]`.
- `canBecomeKey` and `canBecomeMain` are both false, so it never takes focus (`boringNotch/components/Notch/BoringNotchWindow.swift:10-49`).
- The SkyLight variant adds lock-screen presence:
  - It calls private `SLSRemoveWindowsFromSpaces` through `dlsym` (`BoringNotchSkyLightWindow.swift:13-32`).
  - `sharingType = .none` hides the window from screen recording (`:88-94`).
- The window is fixed: 640×(190+20 shadow) (`sizing/matters.swift:15-17`). It is positioned at the screen's top-centre with `setFrameOrigin` (`boringNotchApp.swift:268-280`). SwiftUI animates inside the fixed window.
- One window per screen is keyed by a display UUID. It switches screens on `didChangeScreenParametersNotification` (`boringNotchApp.swift:457-545`).
- On lock the windows are torn down, or promoted to the SkyLight space if "show on lock screen" is on (`:92-145`).

**NotchDrop [seen]**
- `level = .statusBar + 8` ("kills ibar lol"). The window **can** become key (`NotchWindow.swift:36,40-46`).
- The window is the screen's full width and 200 px tall, anchored at the top (`NotchWindowController.swift:10,59-66`).
- A 1 s timer runs a pid-file single-instance check and `makeKeyAndOrderFront` while open (`AppDelegate.swift:31-37,66-84`).
- The window is rebuilt on any screen-parameter change (`:20-25,53-64`).

**Atoll [seen]**
- Same as boring: `NSPanel`, `.mainMenu + 3`, the same collection behaviour (`DynamicIsland/components/Notch/DynamicIslandWindow.swift:25-61`).
- It additionally registers with a capture-visibility manager (`:59`).
- It re-asserts presence on `activeSpaceDidChangeNotification` by resetting `collectionBehavior` and calling `orderFrontRegardless` (`DynamicIslandApp.swift:395-408,972-981`).
- It applies a per-display "top bleed" so the shape overlaps the bezel (`:455-470`).

**Windows equivalents and pitfalls** [inferred unless noted]
- **Always on top without focus.** `WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`. Tuck has the last two.
- **TOPMOST gets knocked off.** The taskbar, a fullscreen exclusive app, "always on top" tools and Win+Tab can push the window below other topmost windows. DynamicWin's 500 ms timer is wasteful. Prefer re-asserting `SetWindowPos(HWND_TOPMOST, SWP_NOACTIVATE|SWP_NOMOVE|SWP_NOSIZE)` on events:
  - `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)`.
  - `WM_DISPLAYCHANGE`.
  - `WM_SETTINGCHANGE`.
  - Session unlock.
  - Resume from sleep.
- **Click-through.**
  - `WS_EX_TRANSPARENT | WS_EX_LAYERED` makes the whole window pass clicks.
  - Tauri's `set_ignore_cursor_events` does this. It is what Tuck does, polled at 40 ms.
  - The cost is that a click-through window gets no OLE drag-enter and no wheel events. See §2.8.
- **Hide from screen capture** (boring's `sharingType = .none`). `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)`, Windows 10 2004+.
- **Multi-monitor.** `EnumDisplayMonitors` plus `GetMonitorInfoW`. Keep a stable monitor id, not an index. Use `QueryDisplayConfig` or `EnumDisplayDevices` DeviceID so a hot-plug does not reshuffle the choice. boring keys by UUID; DynamicWin keys by index, so removing a monitor shifts it (`MainForm.xaml.cs:192-207`, clamped).
- **DPI.** Use physical coordinates (Tuck already does). DynamicWin divides by the primary DPI and WPF DIPs (`WindowPositionHelper.cs:38-42`). That breaks on mixed-DPI setups. Use `GetDpiForMonitor` or per-monitor v2 awareness.
- **Single instance.** A named mutex (DynamicWin), a pid file (NotchDrop), or `tauri-plugin-single-instance`. This matters on this machine, since the feature matrix records a conflicting `nusa.exe` overlay.
- **Tray.** Keep the native shell tray (Tuck has this). Watch for the tray menu rendering under a TOPMOST window.

### 2.2 Hover and expand state machine

**NotchDrop (three states) [seen]**
- `closed`, `popping`, `opened` (`NotchViewModel.swift:29-33`).
- Input is **global** mouse monitors (`NSEvent` global monitors), not window events (`EventMonitors.swift:25-52`):
  - Mouse move in the device notch rect, while `closed`: `notchPop()` (`NotchViewModel+Events.swift:56-64`).
  - Leaving that rect, while `popping`: `notchClose()`.
  - Left-down inside the notch: `notchOpen(.click)`.
  - Left-down outside the opened rect: `notchClose()` (`:14-45`).
- Haptic tick on pop, throttled to 0.5 s and skipped if a button is down (`:75-93`).
- `notchVisible` is cleared 0.5 s after `closed`, so the shape fades out (`:95-104`).
- Open uses `NSApp.activate(ignoringOtherApps: true)` (`NotchViewModel.swift:85-90`), so the window steals focus.

**boring.notch [seen]**
- Hover via SwiftUI `.onHover` (`ContentView.swift:131-133`) into `handleHover` (`:513-558`):
  - Hover starts: set `isHovering` with the spring, then a `Task` sleeps `minimumHoverDuration` (default **0.3 s**, `models/Constants.swift`) before `vm.open()`. It is cancelled if the pointer leaves.
  - Hover ends: a **100 ms** grace sleep, then clear `isHovering` and `vm.close()`. Close is vetoed while a popover is active (`isBatteryPopoverActive`) or `SharingStateManager.shared.preventNotchClose` is set (`:150-180,552`).
- Two-finger drag down opens it and drag up closes it, with live "rubber-band" scaling (`:562-600`, gesture progress ×0.01 scale at `:85-87`).
- A drag over the notch region flips to the shelf. A 500 ms debounce closes it again if no drop arrives (`:217-239`).
- Global keyboard shortcut toggles open and auto-closes after 3 s (`boringNotchApp.swift:368-411`).

**DynamicWin-Legacy [seen]**
- The Skia UI polls `IsHovering`. Size targets switch between `ActiveMenu.IslandSize()` and `IslandSizeBig()` (`UI/UIElements/IslandObject.cs:115-126`).
- `expandInteractionRect = 20` pads the hit rect (`:60`).
- Hover is Skia hit-testing against the layered window.
- The window never changes size, because only the Skia island does.

**Atoll [seen]**
- The same `handleHover` pattern as boring (`ContentView.swift:2303-2342`) with `hoverTask`, plus `openNotchOnHover` and gesture options.

**Port notes**
- Tuck's 40 ms `GetCursorPos` poll is the right Windows analogue of NotchDrop's global monitors. Click-through windows get no hover events, so the poll is mandatory. Consider adaptive polling (§2.4).
- Adopt these from boring: the 100 ms leave grace, a "prevent close" token set (popovers, drags, text fields, dictation), and the 0.3 s open delay as a setting.
- Hit testing at the pop to open boundary needs hysteresis. Use a smaller rect to open and a larger one to stay open. NotchDrop does this implicitly with `inset`.

### 2.3 Animation

**DynamicWin-Legacy [seen].** Second-order dynamics ("Giant's t3ssel8r" model), not CSS or Storyboards.
- `Utils/SecondOrder.cs:13-48`: `k1 = z/(πf)`, `k2 = 1/(2πf)²`, `k3 = r·z/(2πf)`. Stabilised with `k2_stable = max(k2, T²/2+T·k1/2, T·k1)`, so it is unconditionally stable at any frame time.
- Parameters in `IslandObject.cs:32-33`:
  - expand `[f=2.3, z=0.6, r=0.15]`
  - contract `[f=2.8, z=0.8, r=0.1]`
  - So: bouncy open, tighter close.
- Size is fed through it each frame: `Size = scaleSecondOrder.Update(deltaTime, currSize)` (`:126`).
- The morph from island to notch is a 0 to 1 `_morphT` lerped at `6·dt`. Top corners shrink to 0 over 0 to 0.85, then "flares" appear from 0.85 to 1 (`:99,201-260`). Corner squircle control points use κ=0.5523 scaled by a squash factor (`:229-237`).
- Hover shadow strength and size are lerped (`:155-156`).
- The render loop is throttled with `CompositionTarget.Rendering`:
  - Full display refresh while input is active.
  - 60 Hz for "continuous" content.
  - **8 Hz** for static content.
  - Burst mode for 350 ms after events (`MainForm.xaml.cs:40-56,232-319`).
- Rendering is paused on suspend and resumed on resume (`App.xaml.cs:235-283`).

**boring.notch [seen]**
- SwiftUI springs:
  - Movement and resize: `interactiveSpring(response: 0.38, dampingFraction: 0.8)` (`ContentView.swift:41`).
  - Open: `spring(response: 0.42, dampingFraction: 0.8)`.
  - Close: `spring(response: 0.45, dampingFraction: 1.0)`, critically damped, so it never rebounds (`:123-127`).
- The shape is a custom `Shape` with animatable top and bottom corner radii. Closed is 6/14, opened is 19/24 (`NotchShape.swift:12-32`, `sizing/matters.swift:18`).

**NotchDrop [seen].** `interactiveSpring(duration: 0.5, extraBounce: 0.25, blendDuration: 0.125)` (`NotchViewModel.swift:21-25`).

**Port notes for Tuck** (CSS and TS in WebView2):
- Keep the window fixed, like boring and Tuck. A transparent WebView2 window that resizes visibly tears. Tuck already follows this.
- Closed-form spring in JS. Run an rAF loop with DynamicWin's stable second-order integrator for width, height and corner radii. It gives the retargetable motion a CSS transition cannot do, since a new target mid-flight keeps its velocity.
- Or generate a CSS `linear()` easing from a spring sampler. Use `--spring` only for non-interruptible moves.
- Reuse the numbers above as starting points. Open is about f 2.3 / z 0.6; close is about f 2.8 / z 0.8, or critically damped as in boring.
- Idle throttling: stop the rAF loop at rest, as DynamicWin's 8 Hz idle does. A transparent always-on-top WebView2 that keeps repainting costs GPU.

### 2.4 Media now-playing and artwork colour

**DynamicWin-Legacy [seen]**
- `Utils/MediaController.cs`, class `MediaInfo`: `GlobalSystemMediaTransportControlsSessionManager.RequestAsync()` once (`:116`).
- Events (`:119,152-165`):
  - `CurrentSessionChanged` on the manager.
  - `MediaPropertiesChanged`, `PlaybackInfoChanged` and `TimelinePropertiesChanged` on the current session. Old handlers are unsubscribed to avoid leaks.
- The first `MediaPropertiesChanged` is handled immediately; the rest are coalesced with a **120 ms** debounce (`:95,192-232`).
- Stale responses are dropped with version counters (`_mediaPropertiesRefreshVersion`, `_thumbnailFetchVersion`) and a check that the session is still current (`:96-97,253-260`).
- Artwork:
  - `MediaThumbnailService.cs` decodes bytes with Skia (`:501-505`).
  - It keeps a fingerprint (`BitmapUtils.GetBitmapFingerprint`) to skip no-op updates (`:492-521`).
  - **I found no dominant-colour extraction** in the thumbnail, widget or player code (grep for palette, dominant, average, accent returned nothing relevant). Colour is [unknown].
- The play-pause flag is read in the thumbnail widget (`UI/Widgets/Small/MediaThumbnailWidget.cs:145,447,501`).
- Hardware media keys: the low-level keyboard hook handles next and previous (`Main/RendererMain.cs:255-262`).

**boring.notch [seen]**
- Backends:
  - A "Now Playing" controller using private `MediaRemote`.
  - Apple Music, Spotify and YouTube Music controllers (`MediaControllers/`).
- The macOS controller shells out to `/usr/bin/perl mediaremote-adapter.pl <framework> stream` and parses JSON lines (`NowPlayingController.swift:191-215`). Direct private-framework calls stopped working on newer macOS releases, so they proxy through a Perl process [inferred from the design, since the adapter is a workaround].
- State merge in `MusicManager.swift`:
  - Detect title, artist, album, artwork and bundle changes (`:197-223`).
  - Position is **extrapolated** locally: `elapsed + (now − timestamp) × playbackRate`, clamped to the duration (`:567-572`).
- Colour: `NSImage.averageColor` (`extensions/NSImage+Extensions.swift:19-90`):
  - Draws into an RGBA context at full size.
  - Sums R, G, B over every pixel and divides.
  - If near black (<0.03 on all channels), returns grey 0.5.
  - Otherwise converts to HSB and raises brightness to a **minimum of 0.5**, reducing saturation. It runs on a background queue.
  - Triggered from `calculateAverageColor()` (`MusicManager.swift:575-580`) and published as `avgColor`.

**Atoll [seen].** Same `mediaremote-adapter` approach (`mediaremote-adapter/` dir). It adds animated artwork, lyrics (LRC, NetEase), a Spotify auth manager and a like button.

**Windows API and pitfalls**
- **API.** `GlobalSystemMediaTransportControlsSessionManager` (`windows::Media::Control`).
  - Events: `SessionsChanged`, `CurrentSessionChanged`, and per session `MediaPropertiesChanged`, `PlaybackInfoChanged`, `TimelinePropertiesChanged`.
  - `TryGetMediaPropertiesAsync()` returns `Thumbnail` as a `IRandomAccessStreamReference`.
  - `TryTogglePlayPauseAsync`, `TrySkipNextAsync` and `TryChangePlaybackPositionAsync` (100 ns ticks) for control.
- **Pitfalls** [inferred, from API behaviour and DynamicWin's own workarounds]:
  - Metadata often arrives **before** artwork. `Thumbnail` can be null for several hundred ms. Retry with back-off, as Tuck's `art_attempts` does.
  - Events fire in bursts, so debounce.
  - Timeline position is **not live**. SMTC updates it rarely and sets `LastUpdatedTime`. Extrapolate with `LastUpdatedTime` and the playback rate, as boring does.
  - Browser sessions report `SourceAppUserModelId` such as `chrome.exe`. Some players expose no timeline.
  - Key everything by `SourceAppUserModelId` plus title, so a stale async result cannot overwrite a newer track (DynamicWin's version counters).
- **Colour on Windows.**
  - boring's mean is cheap but turns muddy for busy artwork. Downscale to about 32×32 first (boring does not, and loops over every pixel).
  - Prefer a saturation-weighted mean or a small k-means. Keep boring's brightness floor and its near-black rule, which are the useful parts. These improvements are [inferred]; no source here does them.
  - Tuck already has `src/accent.ts`. I did not inspect its algorithm.

### 2.5 Volume and brightness HUD replacement

**DynamicWin-Legacy [seen]**
- It does **not suppress** the Windows flyout. It watches and shows its own on top.
- Volume trigger:
  - Install a global `WH_KEYBOARD_LL` hook (`Utils/KeyHandler.cs:26-34`) that tracks held keys, lets everything through and returns `CallNextHookEx` (`:60`).
  - On `VK_VOLUME_UP/DOWN/MUTE`, open the volume overlay for 2.75 s (`Main/RendererMain.cs:243-253`).
  - The value is read from NAudio `MMDevice.AudioEndpointVolume.MasterVolumeLevelScalar` and `.Mute` (`UI/Menu/Menus/VolumeAdjustMenu.cs:25-32`).
  - The default endpoint is resolved once at startup via `MMDeviceEnumerator.GetDefaultAudioEndpoint` (`App.xaml.cs:76-78`). **It is not refreshed when the default device changes** [seen: no handler found].
- Brightness: WMI `root\wmi` `WmiMonitorBrightness.CurrentBrightness` and `WmiMonitorBrightnessMethods.WmiSetBrightness` (`BrightnessAdjustMenu.cs:95-141`). A `notSupported` flag latches on the first exception (`:97,116-119`).
  - Triggered by **polling** (`RendererMain.cs:283-301`).
  - It works only on internal panels. External monitors need DDC/CI [inferred].
- Skeuomorphism: shake and spring on the volume key, muted state in red (`VolumeAdjustMenu.cs:99-161`).

**boring.notch [seen]**
- Intercepts hardware media keys with a `CGEvent` tap (`.cghidEventTap`, system-defined events, subtype 8) and returns `nil` for handled events, so the macOS HUD is never shown (`observers/MediaKeyInterceptor.swift:68-86,103-139`).
- Needs Accessibility permission, requested through an XPC helper (`:37-60`).
- Option+key opens System Settings or shows the HUD (`:141-154`).
- It plays the stock `volume.aiff` itself to keep the feedback sound (`:156-175`).
- Volume: CoreAudio `AudioObjectGetPropertyData` with `kAudioDevicePropertyVolumeScalar` across channel elements (main, 1 to 4). Listeners are installed for default-device change, volume and mute (`managers/VolumeManager.swift:104-215`).
- Brightness and keyboard backlight: private `DisplayServices` loaded by `dlsym` inside an XPC helper (`BoringNotchXPCHelper/BoringNotchXPCHelper.swift:143-158,179-182`).
- Result goes to a coordinator "sneak peek" with a duration (default 1.5 s, `BoringViewCoordinator.swift:208-257,233`).

**Atoll [seen]**
- Replaces the native HUD by **freezing the system HUD process**:
  - `SIGSTOP` on `OSDUIHelper`, re-applied every ~150 ms because launchd respawns it as a new pid (`managers/SystemOSDManager.swift:27-45,228-310`).
  - Re-enable: `killall -9 OSDUIHelper`, then `launchctl kickstart gui/<uid>/com.apple.OSDUIHelper` (`:107-125`).
  - It sends `SIGCONT` on quit so a crash does not leave the OS HUD frozen (`:183-207`).
  - The suspend watcher stops during sleep and resets on wake (`:49-86`).
- Several HUD styles: circular, vertical, custom OSD, inline.
- A `SystemChangesObserver` decides who draws while the screen is locked (`managers/SystemChangesObserver.swift:25-60`).

**Windows APIs and pitfalls**
- **Reading volume.**
  - `IAudioEndpointVolume::RegisterControlChangeNotify` with an `IAudioEndpointVolumeCallback` gives an event with the new level and mute. `windows-rs` supports `#[implement]` for this [inferred].
  - Tuck polls `IAudioEndpointVolume` every 250 ms (`desktop_signals.rs:144`). Switching to the callback removes polling and the 250 ms lag.
  - Also register `IMMNotificationClient::OnDefaultDeviceChanged` and re-bind. DynamicWin gets this wrong (never refreshed). boring gets it right (listener on `kAudioHardwarePropertyDefaultOutputDevice`, `VolumeManager.swift:176-186`).
- **Suppressing the native flyout.**
  - There is **no documented API**. A web search found no reliable method. All it found were third-party tools such as HideVolumeOSD, which it said may not work on every build, and ModernFlyouts, which replaces the flyout [web].
  - Options:
    - (a) Do nothing and overlay, like DynamicWin. Lowest risk.
    - (b) Swallow `VK_VOLUME_*` in a `WH_KEYBOARD_LL` hook, return 1 and set the volume yourself via `IAudioEndpointVolume::SetMasterVolumeLevelScalar`. This is the analogue of boring's event tap [inferred]. Cost: the hook has a timeout (`LowLevelHooksTimeout`), antivirus often flags global hooks, and keys that do not arrive as `VK_VOLUME_*` (some OEM Fn keys) still show the OS flyout.
    - (c) Find and `ShowWindow(SW_HIDE)` the shell's flyout window. This is what those third-party tools reportedly do, but the window class differs between Windows 10 and 11 [unknown, needs a runtime check with Spy++ or `EnumWindows`].
  - Atoll's `SIGSTOP` trick has **no Windows analogue**. Suspending `explorer.exe` or `ShellExperienceHost.exe` would freeze the taskbar and Start menu. Do not do this.
  - Tuck's stated privacy line (`desktop_signals.rs:1-2`: "arbitrary key presses and typed text are never collected") conflicts with option (b). If used, filter inside the callback to `VK_VOLUME_*` and `VK_MEDIA_*` only, record nothing, and make it opt-in.
- **Brightness.**
  - Event: WMI `WmiMonitorBrightnessEvent` in `root\wmi` [inferred]. Tuck currently polls via `IWbemServices` (`desktop_signals.rs:54-130`).
  - External monitors: DDC/CI through `dxva2` `GetMonitorBrightness` or `SetVCPFeature` [inferred]. Slow, and many monitors do not support it.
  - Tuck also needs the "unsupported" latch (DynamicWin `BrightnessAdjustMenu.cs:97`).

### 2.6 Fullscreen detection and auto-hide

- **DynamicWin-Legacy:** none [seen: grep for fullscreen found only the unrelated `AlwaysTopmost` setting]. The overlay stays over fullscreen apps.
- **boring.notch** [seen]:
  - `FullscreenMediaDetector` uses the third-party MacroVisionKit `FullScreenMonitor.spaceChanges()` stream (`observers/FullscreenMediaDetection.swift:29-54`).
  - `hideNotchOption` is `always`, `nowPlayingOnly` or `never`. In `nowPlayingOnly`, it hides only if the fullscreen space contains the current media app's bundle id (`:44-47`).
  - Status is per screen UUID.
- **Atoll** [seen]: `hideOnClosed` reacts to fullscreen in `ContentView.swift:1877,1966`; Space membership is kept separate from fullscreen visibility (`DynamicIslandApp.swift:375-383,932`).
- **NotchDrop:** none [seen].

**Windows.** Tuck's implementation (§1) is already what the community does. Deltas:
- **Event-driven wake-up.** Use `SetWinEventHook` with `EVENT_SYSTEM_FOREGROUND`, `EVENT_OBJECT_LOCATIONCHANGE` and `EVENT_SYSTEM_MINIMIZEEND` to trigger `check_fullscreen`. Keep the 40 ms poll only for hit-testing, and run the fullscreen check on events or a slow tick.
- **`SHQueryUserNotificationState`.** It reports D3D exclusive fullscreen and "presentation mode", and `QUNS_BUSY` covers an F11 browser. But it reflects the user's *session*, not the monitor, so Tuck's rect check on the island's monitor is the right second signal.
- **Borderless windowed games.** These match the rect test but not `SHQueryUserNotificationState`. Keep the rect test (Tuck does).
- **Per-app policy.** Port boring's `nowPlayingOnly`: hide only if the foreground window's process matches the SMTC `SourceAppUserModelId`. Map the exe to an AUMID with `GetApplicationUserModelId` on the process handle [inferred].
- **Pitfall.** The desktop and shell windows must be excluded or "show desktop" counts as fullscreen. Tuck's class list covers `Progman`, `WorkerW` and the taskbars.

### 2.7 Battery

- **DynamicWin-Legacy** [seen]: `GetSystemPowerStatus` P/Invoke (`Utils/PowerStatusChecker.cs:70-83`), AC line, battery flag (charging bit 8), percent (255 = unknown) and life-time (`uint.MaxValue` = unknown). Polled.
- **boring.notch** [seen]:
  - `IOPSNotificationCreateRunLoopSource` gives power-source change events (`managers/BatteryActivityManager.swift:69-80`).
  - It diffs against the previous snapshot and enqueues typed events one at a time so animations do not overlap: `isChargingChanged`, `powerSourceChanged`, `batteryLevelChanged`, `lowPowerModeChanged`, `timeToFullChargeChanged`, `maxCapacityChanged` (`:21-30,102-190`).
  - Low Power Mode comes from `NSProcessInfoPowerStateDidChange` (`:54-65`).
- **Atoll** [seen]: `MacBatteryManager.swift`, `BatteryPowerReading.swift`. I did not read them in depth.

**Windows**
- **Events.** `RegisterPowerSettingNotification` on a message-only window for `GUID_ACDC_POWER_SOURCE`, `GUID_BATTERY_PERCENTAGE_REMAINING`, `GUID_POWER_SAVING_STATUS` (battery saver) and `GUID_CONSOLE_DISPLAY_STATE`. Handle `WM_POWERBROADCAST` including `PBT_APMRESUMEAUTOMATIC` and `PBT_APMPOWERSTATUSCHANGE` [inferred].
- **Richer data.** `Windows.Devices.Power.Battery.AggregateBattery.GetReport()` and `ReportUpdated` give charge rate, design and full capacity, hence battery health and time to full [inferred].
- **Pitfalls.** `GetSystemPowerStatus.BatteryLifeTime` is only a rough estimate and is frequently unknown. Desktops report "no system battery" (flag 128), so hide the module there. Tuck already treats unknown as unknown (`system.rs:53`).
- **Event queue.** Copy boring's idea of serialising change events through a queue, so a plug-in plus level change does not fire two overlapping animations.

### 2.8 File shelf and drag-and-drop

**NotchDrop (the pure shelf app) [seen]**
- Drop target: SwiftUI `.onDrop(of: [.data])` on the tray view (`TrayDrop+View.swift:36-41`), then `tvm.load(providers)` on a background queue.
- Storage: a **copy**. `FileManager.copyItem` into `~/Documents/CopiedItems/<uuid>/<name>`, plus a PNG preview (`TrayDrop+DropItem.swift:24-39,67-75`).
- Expiry: a `keepInterval` setting (1 h, 1 d, 2 d, 3 d, 1 w, forever, custom), checked in `shouldClean`. It refuses to delete when the interval is ≤ 0 "to avoid deleting the user's files" (`TrayDrop.swift:11,81-88,122-155`; `TrayDrop+DropItem.swift:81-87`).
- Drag out: `Transferable` / `FileRepresentation` copies the file to a fresh temp dir per drag, so the receiver cannot mutate the stored file (`TrayDrop+DropItem.swift:43-64`).
- Delete walks up and removes empty parent dirs (`TrayDrop.swift:95-114`).
- AirDrop share in `Share.swift` and `Share+View.swift` (macOS only).

**boring.notch [seen]**
- Stores **security-scoped bookmarks**, i.e. references, not copies. Data-only drops such as text or images get a temp file (`isTemporary: true`) (`components/Shelf/Services/ShelfDropService.swift:13-66`).
- Persisted as `items.json` (`ShelfPersistenceService.swift:18-73`).
- Multi-select zip via a spawned `zip` process (`TemporaryFileStorageService.swift:129-217`). Temp files are only removed if inside the temp dir (`:31-47`).
- Drag out via `NSDraggingSession` with `NSPasteboardItem` (`Views/ShelfItemView.swift:226-327`), with a custom drag preview.
- Auto-open on a *system-wide* drag, which is the interesting trick (`observers/DragDetector.swift:51-100`):
  - Global monitors for `leftMouseDown`, `leftMouseDragged` and `leftMouseUp`.
  - On mouse-down, remember `NSPasteboard(name: .drag).changeCount`.
  - During the drag, a changed `changeCount` plus a valid type (file URL, URL, string) means a content drag.
  - Entering the notch rect fires `onDragEntersNotchRegion`, which opens the shelf (`boringNotchApp.swift:195-232`).
  - Drag-over-notch detectors are created per screen.

**DynamicWin-Legacy [seen]**
- The window itself is an OLE drop target (`MainForm.AllowDrop = true`, `MainForm.xaml.cs:135`). `DragEnter` opens a "drop file" menu (`:372-382`), `DragLeave` returns home (`:384-390`), `Drop` calls `DropFileMenu.Drop(e)` (`:453-470`).
- This only works because its window is large and not click-through.
- Storage: **copy** to `SavePath\TrayFiles` and skip if the name already exists (`UI/UIElements/Custom/Tray.cs:383-402`). That is a silent name collision.
- Drag out: `DragDrop.DoDragDrop` with `DataFormats.FileDrop`, then the **whole Skia renderer is destroyed and recreated** after the drag (`MainForm.xaml.cs:394-422`), a workaround for the modal drag loop breaking the render surface [inferred].
- "Shake to drop" was cut for performance (README).

**Windows APIs and pitfalls**
- **Drop target.** OLE `IDropTarget`. In Tauri/WebView2, use the window's drag-drop event, which yields `CF_HDROP` paths. Tuck already has `shelf_add`.
- **Drop only works if the window accepts mouse input.** A `WS_EX_TRANSPARENT` or ignore-cursor-events window gets no `DragEnter` [inferred]. Hence the problem: how does Tuck know a drag is approaching when it is click-through?
  - boring's trick transposes to Windows [inferred, unproven]: watch `GetAsyncKeyState(VK_LBUTTON)` plus cursor position on the existing 40 ms poll. If the button is down and the cursor enters the notch zone, switch the window to interactive (`set_ignore_cursor_events(false)`). OLE will then call `DragEnter` and the shelf opens.
  - You cannot read the drag payload before `DragEnter`. Windows has no `.drag` pasteboard to peek at, so you cannot tell a file drag from a window move or text selection. That causes false opens. Mitigate with a dwell time and by checking the foreground window is not the Tuck's own.
- **Virtual files.** Drags from Outlook, browsers and some archive tools carry `CFSTR_FILEDESCRIPTORW` and `CFSTR_FILECONTENTS`, not `CF_HDROP`. boring handles non-file providers by writing a temp file (`ShelfDropService.swift:40-55`). Tuck's path-only shelf will drop these silently [unknown how Tuck's drag-drop event reports them].
- **Cloud placeholders** (OneDrive "online only"). Reading them triggers a download. Copying is slow, and holding a reference is cheap but may go stale.
- **Reference vs copy.**
  - Tuck stores references (`extensions.rs:380-387`) and never removes originals. That matches boring and avoids NotchDrop's disk growth.
  - The cost is that the shelf item dies when the source is moved or deleted. `shelf_path` checks for this.
  - Copy-on-drop with a TTL (NotchDrop) is only worth it for transient sources such as browser-dragged images.
- **Drag out.**
  - Tuck uses the `drag` crate (wraps `DoDragDrop`) (`extensions.rs:452-480`), the right approach.
  - DynamicWin's finding is relevant: `DoDragDrop` is modal and can wedge a custom render loop. Tuck runs it on the main thread via `run_on_main_thread`. Watch for WebView2 hover state sticking after the drag.
  - Offer a temp copy for drag-out like NotchDrop so the target app cannot move the original. This is a choice, not a bug.
- **Share.** DynamicWin advertises the Windows Share dialog. API: `IDataTransferManagerInterop::ShowShareUIForWindow` [inferred].

### 2.9 Calendar

- **boring.notch** [seen]: EventKit.
  - `EKEventStore.authorizationStatus`, `.EKEventStoreChanged` observer (`managers/CalendarManager.swift:45-66`).
  - Calendar and reminder selection, `setReminderCompleted` (`:196`).
  - Calendar auth is requested separately from reminders (`:66,103`).
- **Atoll** [seen]: same, plus lock-screen calendar rows and reminder live activities (`managers/ReminderLiveActivityManager.swift`, `LockScreenReminderWidgetManager.swift`). It has an Apple Notes sync manager.
- **DynamicWin V2:** the README claims Google Calendar integration, but there is no source [unknown]. **DynamicWin-Legacy** has no calendar; it has a weather widget instead (`Utils/WeatherAPI.cs`, `OpenMeteoResponse.cs`).
- **Windows.** There is no EventKit equivalent, so Tuck's local ICS import (done) is the sane base. Add-ons, ranked by cost:
  1. Subscribe to ICS URLs (Google "secret address", Outlook.com publish). Re-fetch every 15 to 30 min. Handle `RRULE`, `EXDATE` and `VTIMEZONE` correctly. Tuck's parser at `extensions.rs:178` should be checked for recurrence [unknown].
  2. `Windows.ApplicationModel.Appointments.AppointmentStore` (`AppointmentManager.RequestStoreAsync`) for the Windows Calendar and Outlook account data [inferred]. Needs a runtime test for unpackaged apps; this was not confirmed.
  3. OAuth against Google Calendar or Microsoft Graph with PKCE and a loopback redirect. Highest effort. The feature matrix already lists OAuth as future.
- **Pitfall.** Treat "join meeting" URLs as untrusted. Tuck's `validate_http_url` (`extensions.rs:123`) already restricts them.

### 2.10 Bluetooth and device notifications

**Atoll [seen]** (`managers/BluetoothAudioManager.swift`, 2949 lines):
- Connect and disconnect come from `DistributedNotificationCenter` `IOBluetoothDeviceConnectedNotification` and `...Disconnected...` (`:167-182`). Power state is checked first (`:264`, `:306`, `:418`).
- Audio devices are classified by SDP service UUID: Audio Sink 0x110B, Headset 0x1108, Handsfree 0x111E (`:543-547`).
- Battery has several fallbacks (`:583-597`): the IOKit registry by address then by name, `UserDefaults`, a BLE battery reader (`BluetoothLEBatteryReader`, `:44`) and a cached `system_profiler` reading shared across callers, refreshed every 20 s (`:72-93`). That subprocess is slow, so it is shared across callers.
- This is the only app here that does Bluetooth battery.

**boring.notch:** grep found no Bluetooth code. **DynamicWin-Legacy:** the "Used Devices" widget is mic and camera, not Bluetooth (`UI/Widgets/Small/UsedDevicesWidget.cs`). The V2 README says "Bluetooth view which shows the connected device and battery" but there is no source [unknown].

**Windows** [inferred unless tagged]
- **Connect and disconnect events.**
  - `DeviceWatcher` from `DeviceInformation.CreateWatcher(BluetoothDevice.GetDeviceSelectorFromConnectionStatus(Connected))`.
  - `BluetoothDevice.ConnectionStatusChanged` and `BluetoothLEDevice.ConnectionStatusChanged` per device.
  - Classic and LE devices need separate watchers.
- **Battery.**
  - BLE: GATT Battery Service (`GattServiceUuids.Battery`) and Battery Level characteristic (`GattCharacteristicUuids.BatteryLevel`), with value-changed notifications [web].
  - Classic headsets (HFP) are **not** exposed by a WinRT call. Community projects query over RFCOMM using the Apple-style `IPHONEACCEV` command. Support varies by vendor [web].
  - Windows Settings shows battery for many devices. It reads a PnP device property; I believe it is `DEVPKEY_Bluetooth_Battery`, `{104EA319-6EE2-4701-BD47-8DDBF425BBE5}` pid 2, via `CM_Get_DevNode_PropertyW` [inferred; a web search did not confirm the GUID]. Verify with a real headset before relying on it.
- **Radios.** Tuck already controls radios (`Radio::GetRadiosAsync`).
- **Pitfalls.**
  - Device IDs are unstable across pairings.
  - Reading GATT keeps a connection open and costs battery on the peripheral.
  - Bluetooth battery over-polling is slow, hence Atoll's 20 s interval and shared cache.
  - Show "battery unknown" honestly, as Tuck's matrix already requires.

### 2.11 Notification mirroring

- **None of the four macOS apps mirror notifications.** Grep for `usernoted`, `UNUserNotificationCenter` and notification-centre DB paths found only a few unrelated hits in Atoll [seen]. macOS has no public API for it.
- The nearest thing is Atoll's Focus/DND state: it reads `~/Library/DoNotDisturb/DB/Assertions.json` and distributed notifications `focusModeEnabled` and `focusModeDisabled`. While Focus is believed active, a periodic task re-verifies it (interval not read) (`managers/DoNotDisturbManager.swift:41-47,94-108,280-290`).
- **Windows.**
  - `UserNotificationListener` (`Windows.UI.Notifications.Management`) is the official API. Tuck already uses it (`extensions.rs:614-690`).
  - It needs a user grant via `RequestAccessAsync`, called from a UI thread, and the `userNotificationListener` manifest capability [web: Microsoft docs describe the packaged-app setup]. I did **not** find a statement on whether an unpackaged desktop app can use it [unknown]. `docs/feature-matrix.md:36` already lists "Windows notification permission/package capability" as an unverified manual item.
  - If it fails unpackaged, the fix to evaluate is a **sparse MSIX package** that gives the exe package identity while staying a Win32 app [inferred]. This also affects toast activation and `UserNotificationListener`.
  - Fallback some tools use is reading the `wpndatabase.db` SQLite file under `%LOCALAPPDATA%\Microsoft\Windows\Notifications` [inferred]. That is undocumented, may expose private content and conflicts with Tuck's privacy stance. Do not do it.
- **Focus Assist / DND state.**
  - No documented API. Community code reads the undocumented WNF state through `NtQueryWnfStateData` (state name `0x0D83063EA3BF1C75`, observed on one Windows 11 build, values 0 to 2). It treats a failed query as "cannot verify" [web, one source, not verified on other builds].
  - Treat as best-effort and test per build. Tuck currently only offers a Focus Settings shortcut.

### 2.12 Timers, Pomodoro and Caffeinate

- **Atoll** [seen]:
  - `TimerManager.swift`: a 1 s `Timer.scheduledTimer` with a `lastUpdated = Date()` stamp (`:191-211,296-316`). It can also adopt an external timer (`adoptExternalTimer`, `updateExternalTimer`, `:323-381`) and plays a sound on completion (`:486-519`).
  - A rolling 1 s tick drifts. If sleep or a heavy frame skews it, remaining time is wrong.
  - Separate lock-screen timer widget.
  - Caffeinate: `IOPMAssertion` with a "keep display awake" option (`managers/CaffeinateManager.swift:49,83`).
- **DynamicWin-Legacy:** timer widget and "timer over" menu (`UI/Widgets/Big/TimerWidget.cs`, `UI/Menu/Menus/TimerOverMenu.cs`) [seen files; internals not read].
- **Windows.**
  - Store an **absolute end time** (monotonic `Instant` or UTC) and recompute remaining time from the clock, never by decrementing. Recompute on `PBT_APMRESUMEAUTOMATIC`. Tuck's `timers` feature should be checked for this [unknown].
  - Completion: a toast or an in-island expand. A sound through `PlaySoundW` or the webview.
  - Caffeinate equivalent: `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED)`, or `PowerCreateRequest` / `PowerSetRequest` for a named reason [inferred]. It is released automatically on exit, unlike a leaked IOPMAssertion.

### 2.13 Clipboard

- **Atoll** [seen]: a 0.5 s timer compares `NSPasteboard.general.changeCount` (`managers/ClipboardManager.swift:259,378,563`). Items are capped by `maxHistoryItems` and old payload files are deleted when trimmed (`:537-545`). The UI has its own panel window and drag support (`components/Clipboard/`).
- **boring.notch** has only rich-text helpers (`helpers/Clipboard+Content.swift`). No history.
- **Windows.**
  - Tuck already does the hard part: sequence number plus the `ExcludeClipboardContentFromMonitorProcessing` and `CanIncludeInClipboardHistory` markers (`clipboard_history.rs:527-576`). This matches Atoll's gap: it has no equivalent "concealed type" handling [seen: grep for `ConcealedType` and `org.nspasteboard` found nothing in the clipboard manager].
  - Delta: replace polling with `AddClipboardFormatListener` on a message-only window (`WM_CLIPBOARDUPDATE`). This is event-driven. I could not tell from `clipboard_history.rs:496` whether Tuck already does this (a 350 ms sleep is present, possibly a retry delay) [unknown].
  - Pitfall: `OpenClipboard` races with the source app. Retry a few times with short sleeps. Also skip when the clipboard owner is Tuck itself.

### 2.14 Webcam mirror

- **boring.notch** [seen]: `AVCaptureSession` with `AVCaptureDevice.DiscoverySession`, authorisation check and request (`managers/WebcamManager.swift:85-108,118-168`), device connect and disconnect notifications (`:65-66`), start and stop on the session (`:75,215,290`). The session runs only while the view is open.
- **Atoll** [seen]: `WebcamManager.swift` plus a `CameraMonitor` for the privacy dot.
- **Windows.**
  - Simplest in Tauri: `getUserMedia` inside the WebView2 page. Handle the WebView2 `PermissionRequested` event so the prompt is not a surprise [inferred].
  - Native options: Media Foundation (`windows::Win32::Media::MediaFoundation`), or the `nokhwa` crate.
  - Pitfalls: some cameras are exclusive and fail if a call app holds them; always stop the stream when the island closes (boring does) so the camera LED goes off; mirror horizontally in CSS, not in capture.

### 2.15 Privacy indicators (mic and camera in use)

- **Atoll** [seen]: camera via CoreMediaIO property listener `kCMIODevicePropertyDeviceIsRunningSomewhere`, so it is event-driven (`managers/CameraMonitor.swift:25-40,66-69`). There are separate microphone and privacy-indicator managers.
- **DynamicWin-Legacy** [seen] (`Utils/DeviceUsageChecker.cs:10-11,44-86`):
  - It reads the Windows capability-consent registry keys `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\{microphone|webcam}`.
  - An app subkey is "in use" when its `LastUsedTimeStop` is 0. It also walks the `NonPackaged` subkey.
  - Results are cached for 750 ms (`:13`).
  - This is the most valuable Windows-specific find in this research.
  - The mic widget also shows loudness via `AudioMeterInformation.MasterPeakValue` and a sqrt curve (`UI/Widgets/Small/UsedDevicesWidget.cs:123-125`).
- **Windows alternatives.**
  - Event-driven registry watch with `RegNotifyChangeKeyValue` on the ConsentStore keys instead of polling [inferred].
  - Microphone: `IAudioSessionManager2` on capture endpoints, with `IAudioSessionNotification` for new sessions [inferred].
  - Pitfall: the registry values are undocumented (Windows' own privacy indicator uses them), and per-app names are raw paths with `#` instead of `\`. Do not parse them into pretty names without a fallback.

### 2.16 Audio visualiser

- **DynamicWin-Legacy** [seen]: NAudio `WasapiLoopbackCapture` on the default render device (`Utils/AudioVisualiser.cs:304-307`), a hand-written FFT (`:563`) with precomputed bin mapping (`:162-201`).
- **boring.notch** [seen]: a Metal shader visualiser (`metal/visualizer.metal`) and Lottie animation; the audio is faked or from the player [inferred].
- **Atoll:** credits `rtaudio`.
- **Tuck** already has WASAPI loopback (`loopback.rs`, 33 ms). Nothing to adopt except: re-bind loopback on default-device change (same pitfall as volume), and pause capture when nothing is playing.

### 2.17 System stats

- **DynamicWin-Legacy** [seen]: LibreHardwareMonitor (`Utils/HardwareMonitor.cs`, `UI/Widgets/Small/SystemUsageWidget.cs`). This pulls in a kernel driver, requires admin for temperatures and is flagged by antivirus.
- **Atoll** [seen]: CPU, GPU, memory, network, disk (`components/Stats/`, `StatsManager.swift`), with CPU temperature through SMC adapted from the Stats project.
- **Tuck** already uses `GetSystemTimes`, `GlobalMemoryStatusEx` and PDH. Those need no driver. Keep it that way. Temperatures are on the deferred list; a driver-based library is not worth it.

### 2.18 Lock screen

- **boring.notch and Atoll** [seen]: private SkyLight space (`CGSSpace`, `SLSRemoveWindowsFromSpaces`) to draw over the macOS lock screen; Atoll has full lock-screen widgets (`components/LockScreen/`).
- **Windows.** No equivalent. The lock screen is the secure desktop and user processes cannot draw there.
- **What to do instead.** `WTSRegisterSessionNotification` with `WTS_SESSION_LOCK` and `WTS_SESSION_UNLOCK`. Hide and suspend polling on lock, re-assert TOPMOST on unlock (a delay as in boring's 150 ms after unlock, `boringNotchApp.swift:130-144`, avoids a flicker).
- **Display off or sleep.** `GUID_CONSOLE_DISPLAY_STATE` and `PBT_APMSUSPEND`. DynamicWin stops its hardware monitor and weather fetcher on suspend and restarts on resume (`App.xaml.cs:235-283`), which is the right pattern.

### 2.19 Other novel features worth knowing

- **LLM usage quotas (Atoll)** [seen], directly relevant to Tuck's `usage.rs`.
  - `ClaudeQuotaClient` calls `GET https://api.anthropic.com/api/oauth/usage` with `Authorization: Bearer` and `anthropic-beta: oauth-2025-04-20` (`managers/LLMUsage/Quota/ClaudeQuotaClient.swift:135-139`).
  - Credentials come from the macOS keychain item `Claude Code-credentials*` (JSON with `claudeAiOauth.accessToken`, `refreshToken`, `expiresAt`) (`:30-53,172-182`).
  - It **refreshes the token** against `https://platform.claude.com/v1/oauth/token` and writes the rotated token back (`:58-66,231-257`).
  - Risks for Tuck: the endpoint is undocumented; rotating a refresh token and writing it back can invalidate the CLI's copy; and `docs/feature-matrix.md:32` records that Tuck deliberately **does not read credential files**. Do not adopt the credential read or refresh. The Cursor, Codex and Antigravity providers are sibling files in the same folder.
- **Extension RPC (Atoll)** [seen]. Third-party apps push live activities and lock-screen widgets to the island through XPC and distributed notifications, with descriptor validation (`managers/Extensions/ExtensionDescriptorValidator.swift`, `services/Extensions/ExtensionXPCService.swift`, `ExtensionRPCServer.swift`). This is the model for Tuck's deferred "plugin widgets". See the port plan.
- **Hide from screen recording** (boring, Atoll) [seen]: see §2.1.
- **Caps Lock indicator** (Atoll) [seen]: `NSEvent` global and local monitors on `flagsChanged` (`managers/CapsLockManager.swift:41-47`). Tuck polls `GetKeyState` every 250 ms; fine.
- **Color picker, terminal tab, Notes sync, per-app volume** (Atoll): present as files, not read.
- **Parallax hover and gestures** (Atoll, boring): rubber-band scale tied to pan progress (`ContentView.swift:85-87,562-600`). Two-finger swipe is a trackpad event; Windows exposes it as wheel or pointer events only when the window receives input.
- **Auto-update**: Sparkle on macOS (boring); DynamicWin has a custom `Updater` project. Tuck has an updater on its deferred list.
- **Haptics**: `NSHapticFeedbackManager`, throttled to 0.5 s (NotchDrop `NotchViewModel+Events.swift:75-93`). No Windows equivalent; skip.

---

## 3. Pitfalls table

| Area | Pitfall | Source |
|---|---|---|
| Topmost | Other TOPMOST windows can cover the island. DynamicWin polls every 500 ms. Prefer event-driven re-assertion. | `DynamicWin-Legacy/.../App.xaml.cs:125-134` |
| Click-through | `AllowsTransparency` per-pixel hit testing does not exist in WebView2. Use `WS_EX_TRANSPARENT` or `set_ignore_cursor_events` plus polling. | [inferred] |
| Drag | A click-through window receives no OLE `DragEnter`. A system-wide drag cannot be seen before it enters. | [inferred] |
| DPI | Primary-monitor DPI only (DynamicWin) is wrong for mixed-DPI setups. | `WindowPositionHelper.cs:38-42` |
| Monitor identity | Index-based selection reshuffles on hot-plug. Use a stable id. | `MainForm.xaml.cs:192-207` |
| Default audio device | Resolved once and never refreshed (DynamicWin). Listen for default-device changes. | `App.xaml.cs:76-78` |
| Volume HUD | No documented way to hide the native flyout. SIGSTOP-style suppression is dangerous on Windows. | §2.5 |
| Keyboard hook | `WH_KEYBOARD_LL` has a timeout, is flagged by antivirus and conflicts with Tuck's privacy line. | `KeyHandler.cs:26-61` |
| SMTC | Artwork lags metadata; timeline is not live; events burst. | §2.4 |
| Stale async | Version counters needed so an old response cannot overwrite a new track. | `MediaController.cs:96-97,253-260` |
| Shelf storage | Name collisions are silently skipped (DynamicWin). Copy without TTL grows forever (NotchDrop). | `Tray.cs:398`; `TrayDrop.swift:81-88` |
| Drag-out | `DoDragDrop` is modal and can wedge a custom renderer (DynamicWin rebuilt the renderer). | `MainForm.xaml.cs:394-422` |
| Virtual file drops | Outlook and browser drags are not `CF_HDROP`. | [inferred] |
| Bluetooth battery | Classic HFP has no WinRT API; GATT reads keep the link alive. | [web] |
| Notification listener | Package-identity requirement for unpackaged apps is unknown. | [web] + `feature-matrix.md:36` |
| Focus / DND | Undocumented WNF state, build-specific. | [web] |
| Mic/camera usage | Registry values undocumented; paths encoded with `#`. | `DeviceUsageChecker.cs:10-11` |
| Timers | Decrementing on a 1 s tick drifts and mishandles sleep. | `TimerManager.swift:191-211` |
| Token refresh | Rotating a CLI's OAuth token can log the CLI out. | `ClaudeQuotaClient.swift:58-66,231-257` |
| Licensing | GPL-3.0 (boring.notch, Atoll) and CC BY-SA (DynamicWin) cannot be copied into Tuck. | LICENSE files |

---

## 4. Port plan for Tuck

Priority: **P1** = clear win, low risk. **P2** = worthwhile, needs a spike. **P3** = optional or blocked. "File" is where it would land; Tuck's existing modules are named in §1. All APIs below are [inferred] from API knowledge unless tagged; each needs an implementation spike on a real Windows 11 build.

| # | Feature | Priority | Approach (Rust crate / Win32 / WinRT) | Lands in | Notes |
|---|---|---|---|---|---|
| 1 | Event-driven media | P1 | `windows::Media::Control`: handlers for `SessionsChanged`, `CurrentSessionChanged`, `MediaPropertiesChanged`, `PlaybackInfoChanged`, `TimelinePropertiesChanged`. 120 ms trailing debounce, first event immediate. Version counter and `SourceAppUserModelId` check before applying async results. Extrapolate position from `LastUpdatedTime` and playback rate; keep a slow tick only for the progress bar. | `media.rs` (replace the 500 ms poll at `:418-432`) | From DynamicWin `MediaController.cs:116-232` and boring `MusicManager.swift:567-572`. |
| 2 | Artwork accent | P2 | Downscale to about 32×32 with the `image` crate, saturation-weighted mean, boring's brightness floor 0.5 and near-black to grey 0.5 rule. | `accent.ts` or a Rust command | From boring `NSImage+Extensions.swift:19-90`. Check what `accent.ts` already does first. |
| 3 | Volume HUD, event-driven | P1 | `IAudioEndpointVolumeCallback` via `windows-rs` `#[implement]`; `IMMNotificationClient::OnDefaultDeviceChanged` to re-bind; fall back to the current poll if registration fails. | `desktop_signals.rs` (`:19-52`) | Fixes DynamicWin's stale-endpoint bug. Do not hide the native flyout by default (§2.5). |
| 4 | Native flyout suppression | P3 | Opt-in: `SetWindowsHookExW(WH_KEYBOARD_LL)` that swallows only `VK_VOLUME_*` and sets volume through `IAudioEndpointVolume`. | new `volume_keys.rs` | Privacy and antivirus cost. Spike on target builds before committing. Window-hiding approach needs runtime discovery. |
| 5 | Brightness | P2 | Keep WMI; add the `WmiMonitorBrightnessEvent` subscription. External monitors via `dxva2` DDC/CI behind a "supported" probe. Latch unsupported once. | `desktop_signals.rs` (`:54-130`) | DynamicWin `BrightnessAdjustMenu.cs:95-141`. |
| 6 | Window z-order and display events | P1 | Hidden message-only window or `SetWinEventHook` for `EVENT_SYSTEM_FOREGROUND`; handle `WM_DISPLAYCHANGE`, `WM_DPICHANGED`, `WM_SETTINGCHANGE`. Re-assert `HWND_TOPMOST` with `SWP_NOACTIVATE` on those. Replace the 2 s re-centre loop (`system.rs:459-470`). | `system.rs` | Replaces DynamicWin's 500 ms timer. |
| 7 | Session, sleep and lock | P1 | `WTSRegisterSessionNotification` (`WTS_SESSION_LOCK/UNLOCK`), `RegisterPowerSettingNotification` for `GUID_CONSOLE_DISPLAY_STATE`; `PBT_APMSUSPEND` / `PBT_APMRESUMEAUTOMATIC`. Pause pollers, re-assert on unlock after about 150 ms. | `system.rs` or new `session.rs` | From boring `boringNotchApp.swift:92-145` and DynamicWin `App.xaml.cs:219-283`. |
| 8 | Fullscreen | P2 | Keep `check_fullscreen`. Trigger it from `SetWinEventHook` events, not only from the 40 ms loop. Add `nowPlayingOnly` mode: compare the foreground process's AUMID with the SMTC source. | `system.rs:271-338` | Boring `FullscreenMediaDetection.swift:44-47`. |
| 9 | Multi-monitor | P2 | `EnumDisplayMonitors`; stable id from `QueryDisplayConfig` or `EnumDisplayDevices`; settings: primary, specific, follow cursor. One window per monitor is optional (boring `showOnAllDisplays`). | `system.rs` (`center_on_primary_monitor` at `:237`) | Avoid DynamicWin's index-based pick. |
| 10 | Hover tuning | P1 | Add the 100 ms leave grace, a "prevent close" token set (popover, drag, dictation, text input), configurable open delay (default 0.3 s) and open vs stay-open hysteresis. | `src/runtime.ts`, `system.rs` | boring `ContentView.swift:513-558`; NotchDrop `NotchViewModel+Events.swift:56-64`. |
| 11 | Spring motion | P2 | rAF loop with DynamicWin's stable second-order integrator for width, height and corner radii; stop the loop at rest. Starting values from §2.3. | `src/styles.css`, `main.ts` | Retargetable mid-flight. `SecondOrder.cs:13-48`. |
| 12 | Hide from capture | P1 | `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` toggled by a setting. | `system.rs` | boring `BoringNotchSkyLightWindow.swift:88-94`. Win10 2004+. |
| 13 | Single instance | P1 | `tauri-plugin-single-instance`, focusing the existing instance. | `lib.rs` | The `nusa.exe` conflict in the feature matrix is a different app; this only prevents two Tucks. |
| 14 | Battery events | P2 | `RegisterPowerSettingNotification` (`GUID_ACDC_POWER_SOURCE`, `GUID_BATTERY_PERCENTAGE_REMAINING`, `GUID_POWER_SAVING_STATUS`); `Windows.Devices.Power.Battery` `ReportUpdated` for charge rate and health. Serialise change events through a queue. | `system.rs` (`read_power` at `:103`) | boring `BatteryActivityManager.swift:69-190`. |
| 15 | Bluetooth device alerts | P2 | `DeviceInformation.CreateWatcher` with `BluetoothDevice.GetDeviceSelectorFromConnectionStatus`; `ConnectionStatusChanged`; BLE battery via GATT Battery Service; PnP property route and RFCOMM only after a hardware spike. Share one cache, refresh about every 20 s. | new `bluetooth.rs` | Atoll `BluetoothAudioManager.swift:44-93,167-182`. Show "battery unknown" honestly. |
| 16 | Mic and camera dots | P2 | `RegNotifyChangeKeyValue` on the `ConsentStore\microphone` and `webcam` keys; `LastUsedTimeStop == 0` means in use; include `NonPackaged`. Optional `IAudioSessionManager2` for mic level. | `desktop_signals.rs` or new `privacy.rs` | DynamicWin `DeviceUsageChecker.cs:44-86`. Read-only; no capture. Undocumented values; degrade to hidden. |
| 17 | Focus / DND state | P3 | `NtQueryWnfStateData` / `RtlSubscribeWnfStateChangeNotification` with the observed state name; treat failure as unknown. | new `focus.rs` | Single-source, build-specific. Ship only behind a flag. |
| 18 | File shelf auto-open on drag | P2 | In the 40 ms loop, if `GetAsyncKeyState(VK_LBUTTON)` is down and the cursor enters the notch zone, set interactive so OLE `DragEnter` fires. Dwell time to cut false positives. | `system.rs`, `extensions.rs` | boring `DragDetector.swift:51-100`. Spike first: false positives from window drags; cannot inspect the payload beforehand. |
| 19 | Virtual-file drops | P3 | Custom `IDropTarget` reading `FileGroupDescriptorW` plus `FileContents`, writing to a temp dir. Check what Tuck's current drag-drop event reports first. | `extensions.rs` | boring `ShelfDropService.swift:40-55`. |
| 20 | Shelf options | P2 | Keep references. Add: per-item "stale" state when the source disappears, optional temp copy for drag-out, and collision-safe naming for any copies. Skip TTL copies unless transient sources are added. | `extensions.rs` | DynamicWin `Tray.cs:398` and NotchDrop `TrayDrop.swift:81-88` show what to avoid. |
| 21 | Share sheet | P3 | `IDataTransferManagerInterop::ShowShareUIForWindow` for selected shelf files. | new command | DynamicWin README mentions share; the code was not read. |
| 22 | Calendar sources | P2 | ICS URL subscription with proper `RRULE`, `EXDATE`, `VTIMEZONE`. Later: `AppointmentStore` spike, or Google/Graph OAuth (PKCE, loopback). | `extensions.rs:178` | Verify the current parser's recurrence handling first. |
| 23 | Notification mirroring | P2 | Keep `UserNotificationListener`. Spike a sparse MSIX package for identity if unpackaged access is denied. Do not read `wpndatabase.db`. | `extensions.rs:614-690`, packaging | Package-identity requirement is [unknown]. |
| 24 | Timers | P1 | Audit: store an absolute end time, recompute on every tick and on resume. Add `SetThreadExecutionState` as an optional "keep awake" action. | `src/activities.ts`, new command | Atoll `TimerManager.swift:191-211`; `CaffeinateManager.swift:49,83`. |
| 25 | Clipboard listener | P2 | `AddClipboardFormatListener` on a message-only window; retry `OpenClipboard`. Keep the existing exclusion markers. | `clipboard_history.rs` | Atoll polls at 0.5 s (`ClipboardManager.swift:378`). Check whether Tuck already listens (`:496`). |
| 26 | Webcam mirror | P3 | `getUserMedia` in the WebView2 page with a `PermissionRequested` handler; stop the stream on close; CSS mirror. | frontend only | boring `WebcamManager.swift:85-168`. Alternative: `nokhwa`. |
| 27 | Loopback re-bind | P2 | Re-open WASAPI loopback on default-device change; pause when nothing plays. | `loopback.rs` | DynamicWin `AudioVisualiser.cs:304-307`. |
| 28 | Idle render throttling | P2 | Stop the rAF loop at rest; drop to a low rate when static; pause on lock and sleep. | frontend | DynamicWin `MainForm.xaml.cs:40-56,257-300`. |
| 29 | Extension / plugin API | P3 | Local named pipe or loopback HTTP with a per-install token and a validated JSON descriptor. Live activities only; no code execution. | new module | Atoll `ExtensionDescriptorValidator.swift`, `ExtensionRPCServer.swift`. Already on the deferred list. |
| 30 | LLM quota sources | **Do not adopt** | The Atoll approach reads the CLI's credential store and rotates tokens. Conflicts with `docs/feature-matrix.md:32` and risks logging the CLI out. Keep Tuck's current explicit-key and status-line approach. | n/a | `ClaudeQuotaClient.swift:58-66,135-139,231-257`. |
| 31 | Lock-screen widgets | **Not portable** | The Windows lock screen is the secure desktop. Only hide on lock (item 7). | n/a | boring/Atoll use private SkyLight. |
| 32 | Haptics | **Not portable** | No Windows equivalent. | n/a | NotchDrop `NotchViewModel+Events.swift:75-93`. |

### Suggested order

1. **Ship first (P1):** items 1, 3, 6, 7, 10, 12, 13, 24. They remove polling, fix lifecycle holes (lock, sleep, display change, TOPMOST) and cost little.
2. **Then (P2):** 2, 5, 8, 9, 11, 14, 15, 16, 18, 20, 22, 23, 25, 27, 28.
3. **Spike before committing:** 4, 17, 19, 21, 29. Each rests on an undocumented API or an unverified premise.

### Open questions

Things this research did not settle, each needing a check on a real machine:

- Whether an unpackaged Tauri exe can use `UserNotificationListener`, and what a sparse MSIX package changes.
- Which window hosts the Windows 11 volume flyout on the current build, and whether it can be hidden without side effects. Partly answered (2026-11-03, build 26300): two `XamlExplorerHostIslandWindow` hosts owned by `explorer.exe` have the `DesktopWindowXamlSource` child; the flyout is the small one at the bottom centre, the other covers the screen. The flyout host measures 169x61 at (875, 971) on a 1920x1080 screen and only exists after the first flyout of a session. Tuck (`osd_flyout.rs`) matches the class pair as HideVolumeOSD 1.4 does, then requires a shell-owned window with the flyout's shape (bottom half, horizontally centred, under a third of the screen), and only after a volume key press. A brightness change can re-minimize a host already identified but never introduces a new one, because brightness keys have no key code and look like the Quick Settings slider. Side effects on other builds are untested.
- Whether the Bluetooth battery PnP property key I recalled is right, and which of the user's devices expose it.
- How Tuck's current drag-drop event reports virtual files.
- Whether Tuck's ICS parser expands recurring events.
- What `accent.ts` does today, and whether Tuck's timers store an absolute end time.
- Whether Tuck's clipboard path is already event-driven.
- Everything about the closed-source Windows apps. Nothing was inspected.

### Sources outside the clones

Web checks made this session, all thin:

- [Windows volume OSD hiding tools](https://www.windowsdigitals.com/?p=17139), [ModernFlyouts / HideVolumeOSD discussion](https://answers.microsoft.com/en-us/windows/forum/all/hide-or-move-on-screen-volume-control/53faeb50-d96e-438a-93b8-1f013ec4c63c)
- [Notification listener documentation](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/notification-listener)
- [GattServiceUuids.Battery](https://learn.microsoft.com/en-us/uwp/api/windows.devices.bluetooth.genericattributeprofile.gattserviceuuids.battery?view=winrt-19041), [classic Bluetooth battery on Windows (community)](https://github.com/SpartanX1/bluetooth_classic_battery_windows)
- [WNF-based DND detection (community)](https://docs.rs/crate/taskbar-focus/0.3.4/source/src/os/dnd/wnf.rs)
