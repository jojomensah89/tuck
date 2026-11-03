# Competitor landscape: notch / island utilities (Windows, macOS, iOS)

Research date: 2026-11-03. Method: TinyFish search and fetch (read-only), about 35 distinct sources.
All web content was treated as untrusted data. Where a fact comes only from a search snippet, a competitor's own marketing page, or a rival's comparison blog, it is flagged **[unverified]**. Several comparison pages are written by competing vendors (Notchify, Seam, Droppy, Notchy) and are biased.

Baseline for "what Tuck already has" is `PRODUCT.md` and `docs/feature-matrix.md`: SMTC media with seek and artwork, lyrics, clipboard history, file shelf with drag out, CPU/RAM/GPU metrics, audio visualization, offline dictation, local to-dos, timers/Pomodoro, ICS calendar, notification previews, downloads/call activities, Wi-Fi/Bluetooth radio toggles, screen capture, AI usage meters, hanging-notch or floating-pill style, and a 212x32 idle / 300x38 peek / 440x180 open size model. Deferred in Tuck: Bluetooth peripheral battery, mic/camera indicators, Num Lock HUD, weather, active-app context, lock/unlock behaviour, USB eject and others.

---

## 1. Key finding: the Windows niche is crowded, and the market has noticed

- A Reddit thread, "What's going on with all these notch apps for Windows?" (r/windowsapps, about 3 weeks before research), describes "fatigue" with new notch apps. One commenter called many of them "vibe-coded". Developers report single-digit to about 30 paying customers (AWNA: 10 sales; BuildNotch: 31).
- By Oct 2026 there are at least a dozen Windows entries: Notchify, Dynamic Edge, DynamicWin, Halo DynamicWin, Smart Notch, NotchPixel, BuildNotch, AWNA, Nusa, Pillar, NotchX/DynamicBar, Isle Notch, The Dynamic Notch, Win Dynamic Island, plus FluentFlyout as the media-flyout alternative.
- Differentiation therefore has to come from reliability, polish, and quiet behaviour, not from a feature checklist. The macOS history (NotchNook: feature sprawl, "form over function", 40% CPU reports) is the cautionary tale.
- NotchWin: I could not locate a distinct, maintained project with that name. Search returned only DynamicWin forks. Treat as not found.

---

## 2. Feature matrix

Legend: Y = present, p = partial or Pro/paid tier, - = not found or not stated, ? = unverified. "Tuck" column reflects the repo docs, including phase-two items still under review.

### 2a. Windows

| Feature | Tuck | Notchify | Dynamic Edge | DynamicWin | Nusa | AWNA | BuildNotch | Pillar | DynamicBar/NotchX | FluentFlyout |
|---|---|---|---|---|---|---|---|---|---|---|
| Price | n/a | Paid, free trial | Free (MS Store) | Free | Free | Free beta | Paid one-time | Free OSS | Free OSS | Free OSS; Store IAP 2.99 EUR |
| SMTC media + art | Y | Y | Y | Y | Y | Y | Y | Y | Y | Y |
| Seek bar | Y | Y | Y | ? | Y | ? | ? | Y (scrub) | Y | ? |
| Synced lyrics | Y | Y (click line to jump) | Y (also in idle capsule) | - | Y (split view, click line) | - | - | - | - | - |
| Audio visualizer | Y | Y | Y | - | - | - | - | - | - | - |
| Clipboard history | Y | Y (drag out) | Y | - | - | Y | Y | - | p (notify only, off by default) | - |
| File shelf / tray | Y | Y (drag in/out) | Y | Y (open, share) | Y (references, no copy) | Y | Y (v1.1) | - | p (2 image slots + note) | - |
| Notification mirror | Y (preview) | Y (stack, swipe to dismiss) | ? | - | ? | - | - | Y (history) | Y (internal only) | - |
| Bluetooth device battery | deferred | Y | ? | Y | Y (headphone peek) | - | - | - | - | - |
| Mic/camera in-use indicator | deferred | ? | - | - | - | - | - | - | Y | - |
| System stats | Y | Y (CPU, mem, disk, net) | ? | - | - | - | - | Y (CPU, RAM) | Y (1-min graphs) | - |
| Volume/brightness/lock-key HUD | partial | Y | Y (volume) | - | Y (also Caps, keyboard layout) | Y | - | Y (+ per-app mixer) | scroll-on-pill = volume | Y (lock keys, media-key flyouts) |
| Timers / Pomodoro | Y | Y | ? | - | Y | Y | Y | Y | Y | - |
| Calendar | Y (ICS) | Y | - | Y (Google) | - | - | - | Y (agenda) | Y (local notes) | - |
| To-do / notes | Y | Y | - | - | Y (with reminders) | Y | Y | Y | Y (day notes) | - |
| Screen record / capture | Y (capture) | Y (record, island hidden from video) | - | - | - | Y (screenshot, record, OCR, picker) | - | - | - | - |
| OCR | - | ? | Y | - | - | Y | - | - | - | - |
| Offline dictation | Y | - | - | - | - | Y (Whisper) | - | - | - | - |
| App/URL launcher | - | Y | Y | - | - | Y | Y | - | Y (40-odd targets) | - |
| Water reminder / mascot | - | - | Y (virtual pet) | - | Y | - | - | - | - | - |
| AI assistant / usage | Y (usage meters) | - | - | - | - | - | Y (AI usage) | Y (Groq chat) | - | - |
| Phone file transfer (QR) | - | Y | - | - | - | - | - | - | - | - |
| Position options | notch/pill, top | 8 edges/corners | top, L/R edge | top | top | top | top | top | top | corner flyouts + taskbar widget |
| Hides in fullscreen | Y | Y (can stay) | ? | ? | Y | ? | per-app rules (v1.2) | Y | auto-hide | n/a |
| Tech | Tauri 2 | Native GPU, 60-240 Hz | WinUI 3, ~75-180 MB | C#/SkiaSharp | Rust, ~30 MB, 0% idle CPU | WPF .NET 8 | WinUI 3 | Tauri 2, ~3 MB | Flutter + C++ | WPF/Fluent |

### 2b. macOS

| Feature | Alcove | NotchNook | boring.notch | Atoll | DynamicLake Pro | MediaMate | NotchDrop | MewNotch | Notchmeister |
|---|---|---|---|---|---|---|---|---|---|
| Price | $13.99-14.99 once, 48-72 h trial | $15 once (was $25) or $2/mo | Free GPL-3 | Free GPL-3 | Paid (20% off promo) | ~$7 once | Free + Pro | Free GPL-3 | App Store |
| Hover expand | - (swipe/click) | Y | Y | Y (or 2-finger swipe) | Y | n/a | Y | Y (delay configurable) | n/a |
| Swipe gestures | Y (switch/dismiss) | Y (media) | Hover only | Y (open/close, media skip) | ? | - | - | - | - |
| Live Activities | Y | Y | Y (playback) | Y (media, Focus, rec, privacy, downloads beta, battery) | Y | - | - | - | - |
| Now playing | Y | Y | Y + visualizer | Y | Y (DynaMusix) | Y | Y | Y | - |
| System HUD replace | Y | - | Y | - | Y | Y (core) | - | Y (core) | - |
| File shelf | - | Y | Y + AirDrop | Y + AirDrop | Y (drag/drop, convert) | - | Y + AirDrop + LAN share | Y + AirDrop | - |
| Clipboard history | - | - | - | Y | ? | - | Y | - | - |
| Calendar | Y | Y | Y | Y | Y | - | Y | - | - |
| Lock-screen widgets | Y | - | Y | Y | ? | - | - | Y | - |
| Mirror camera | - | Y (tiny) | Y | Y | - | - | Y | Y | - |
| Bluetooth/AirPods connect | ? | ? | roadmap | Y (lock widget) | Y (DynaConnect) | - | - | - | - |
| Extension API | - | - | roadmap/Y | Y (AtollExtensionKit) | Y (extensions) | - | - | - | - |
| Custom scripts | - | - | - | - | - | - | Y (commands) | Y (bash view) | - |
| Fun/ambient | - | - | - | - | - | - | - | - | Y (Glow, Cylon, etc.) |
| Platform min | macOS 15 | 14.6 | 14 | 14 | 14 | 13 | ? | ? | ? |

### 2c. iOS (reference model)

| Presentation | Use | Trigger |
|---|---|---|
| Minimal | One dot or glyph when two or more activities compete; the extra activity detaches as a separate bubble | Automatic |
| Compact leading + trailing | Default; shown either side of the sensor housing | Automatic |
| Expanded (leading / trailing / center / bottom regions) | Rich view | Touch-and-hold on compact or minimal; also system-initiated on update/alert |

---

## 3. Per-app notes

### Windows

**Notchify** (Kingpindev). Microsoft Store; paid with free trial; claims to be earliest on the Store (April 2026). Rendering is native and GPU-accelerated with a selectable 60-240 Hz. Features: synced lyrics (click a line to jump), clipboard history with drag out, file tray, notification panel with stacking and swipe-to-dismiss, Bluetooth battery including BLE, 20+ widgets, screen recording (the island is excluded from the video), Discord call controls, phone file transfer by QR, 14 themes, 8 screen-edge positions, 12 languages. The 5.6 update notes "no more stutter in fullscreen games" and "island now draws nothing at all while it sits behind a fullscreen app", plus lower GPU use while Discord holds the mic. Hover opens, click pins. Windows Central covered it in Sep 2026. Dev also states "always on top could wrongly drop behind windows on some PCs" was fixed, which is a recurring problem class for this category.
Sources: https://notchify.app/en/compare/ , https://www.reddit.com/r/windowsapps/comments/1x1de9e/notchify_dynamic_island_for_windows_the_56_update/ , https://www.windowscentral.com/software-apps/want-your-pc-to-be-a-bit-more-like-a-mac-the-dynamic-island-for-windows-just-got-a-big-update . Note: the comparison page is vendor-authored.

**Dynamic Edge** (Eyuel Engida). WinUI 3, Store, free. Lyrics (also inside the compact idle capsule), universal media hub, app and folder shelf, clipboard and text OCR, spring physics, notch mode, side docking, auto-hide when idle. A user measured about 75 MB start rising to 180 MB, CPU 0.5% with 2.5-3% spikes. Developer chose closed source and Store-only. Community reaction mixed ("AI slop" comments; also accusations of copying a rival's lyrics update within a day).
Sources: https://www.reddit.com/r/windowsapps/comments/1wxkfye/dynamic_edge_a_native_dynamic_island_for_windows/ , https://www.microsoft.com/en-ie/p/dynamic-edge/9pf1587ksn2k [store page not fetchable; snippet only].

**DynamicWin** (Florian Butz). Earliest of the group (C#/SkiaSharp, about 2 years old); repo now only hosts releases, no source. 641 stars. Media controller with favourites, calendar with Google Calendar, file tray (double-click executes, Windows share dialog), Bluetooth view with device battery, an activity system with "spring notches" (see or open an action that is not the current view), scroll to switch views, auto-updater. Shake-to-drop was cut for performance. Shows the value of a small, focused feature set.
Source: https://github.com/FlorianButz/DynamicWin . Forks: Halo DynamicWin (MIT, Store, 21 languages per Notchify's table).

**Nusa** (rama2555). Rust, GPU drawn, about 3 MB installer, about 30 MB RAM, 0% CPU idle, "waits instead of checking" (event driven). Hover opens, leave closes. Lyrics split view, Pomodoro, water tracker, file shelf (stores references, drag out), to-do with reminders, volume/brightness/Caps Lock/keyboard-language overlays, peeks for charger, lost connection and earbud pairing, minimal notch mode (thin black line for thick bezels), idle mascot eyes. Unsigned installer warning prominently explained. Notably it already ships overlapping with Tuck's design (the Tuck repo notes an existing nusa.exe overlay colliding at the same screen position, which is a real co-existence risk for all users running two notch apps).
Source: https://rama2555.github.io/nusa/

**AWNA** (AfterimageDev). WPF/.NET 8, free beta, about 78 MB (mostly an offline Whisper model). Ten tabs: Today, Tasks, Notes, Time, Music, Controls, Launch, Shelf, Clipboard, Capture (screenshot, record, OCR, colour picker, webcam mirror). Offline voice transcription for tasks and notes. Global Ctrl+Alt+Space opens it; quick timer, tasks, UUID/base64 generators. Closest feature overlap with Tuck (offline dictation).
Source: https://www.reddit.com/r/windowsapps/comments/1wox22m/i_made_a_minimalist_windows_notch_app_with_a_file/

**BuildNotch**. WinUI 3, paid one-time (about $3 mentioned by developer), 31 customers. Developer-focused: project shortcuts, GitHub streak, AI usage compact view, code playground, JSON formatter, seven timer themes, clipboard grid with filters, per-app fullscreen hide rules (1.2.0). Not code-signed per vendor comparison.
Source: https://www.reddit.com/r/sideprojects/comments/1x5nu7p/my_windows_notch_app_reached_31_paying_customers/

**Pillar** (warpirate). Tauri 2 + React + Rust, free OSS, about 3 MB. Same stack as Tuck. SMTC media with album-tinted pill, Pomodoro, toast notifications folded into the pill with history, system and per-app volume with output-device switching, CPU/RAM, brightness, battery, a Groq-backed AI assistant that knows the active app name and window title, tasks/notes/agenda, keyboard nav, screen-reader announcements, reduced motion, hides in fullscreen. Hover to peek, click to expand.
Source: https://github.com/warpirate/pillar-dynamic-island-for-windows

**DynamicBar / NotchX** (rahozosman). Flutter shell + hand-written C++ over Win32/WinRT, no network calls, seven-page pager (Now, Calendar, Shortcuts, Media, Shelf, Timers, System). Notable platform choices: ReadDirectoryChangesW for downloads with "progress only when the total size is knowable", capability consent store for the privacy (mic/camera) indicator, AddClipboardFormatListener for clipboard (reports *that* something was copied, not content, off by default), scroll over closed pill = volume, Ctrl+Alt+Space global open, auto-hide with proximity reveal. Windows 11 only. The NotchX Store listing appears to be a different but related project.
Sources: https://github.com/rahozosman/notchX , https://apps.microsoft.com/detail/9n2lwtdsx986 [snippet only].

**FluentFlyout / ModernFlyouts** (adjacent category: not a notch). FluentFlyout: free OSS (Store version has a 2.99 EUR unlock), Fluent 2 / Mica look, audio flyout, "Up Next" flyout when a song ends, lock-keys flyout, taskbar widget showing media in the taskbar, customizable flyout positions, repeat/shuffle. ModernFlyouts: Fluent replacement for Windows 10 volume/brightness/airplane-mode/lock-key flyouts. These are the "native feel" bar for HUDs. ModernFlyouts details are from snippets only **[unverified]**.
Sources: https://github.com/unchihugo/FluentFlyout , https://modernflyouts-community.github.io/

**Other Windows entries seen but not examined:** Smart Notch (free, Pro tier), NotchPixel ($9.99; mute key, OBS pill, stream chat), Isle Notch, The Dynamic Notch, Win Dynamic Island, opencode-notch (agent approvals in a notch), Halo DynamicWin. Features taken from Notchify's comparison table only **[unverified]**.

### macOS

**Alcove** (Henrik Ruscon). $13.99-14.99 one-time, 48-72 h trial, signed, native Swift, macOS 15+. Closest to the iPhone Dynamic Island in motion; Live Activities, swipe gestures (move between activities, dismiss one), instant notifications, customizable HUDs, lock-screen widgets. No file shelf, clipboard, timer or dictation per rival blogs. Praised for polish and stability; criticised for scope.
Sources: https://tryalcove.com/ , https://getseam.app/blog/alcove-vs-boring-notch (rival vendor).

**NotchNook** (lo.cafe). $15 lifetime (struck from $25) or $2-3/mo, two Macs, 48 h trial. Two areas: "nook" (widgets: media, shortcuts, calendar, mirror) and a file tray. Early Reddit review (Aug 2024): mirror "the size of a postage stamp", shortcuts do not accept files, calendar ignores default-calendar selection, "form over function", reports of 40% CPU and multi-GB RAM, crashes on startup, support slow. 2026: a founder dispute took payments and the site offline; 7 of 8 Trustpilot reviews are one star, mostly licence deactivation and no refunds; removed from Setapp and Homebrew. MacUpdate 4.7, Product Hunt 4.6. Praise: looks native, tray and widgets. Lesson: licensing and trust failures hurt more than missing features.
Sources: https://www.reddit.com/r/macapps/comments/1ejqaeb/notchnook_not_ready_for_prime_time/ , https://getdroppy.app/blog/notchnook-reviews (written by a competitor; cites primary sites).

**boring.notch** (TheBoredTeam). Free, GPL-3, 11.2k stars, macOS 14+. Music with audio visualizer, calendar and reminders, battery, file shelf with AirDrop, HUD replacement, mirror, compact player layout; roadmap includes extension system and gesture customisation. Hover to expand, menu-bar settings. Complaints: unsigned (Gatekeeper workaround via xattr), crashes after sleep/wake, visualizer drains battery, a GitHub issue about dropped files being copied to a temp cache. Atoll is a fork that adds clipboard, terminal, CPU/GPU stats.
Sources: https://github.com/TheBoredTeam/boring.notch , https://getseam.app/blog/alcove-vs-boring-notch

**Atoll** (Ebullioscopic). Free GPL-3, 4.5k stars, notch MacBooks only (14/16 inch MBP). Live Activities for media, Focus, screen recording, privacy indicators, downloads (beta), battery; lock-screen widgets (media, timers, charging, Bluetooth, weather); CPU/GPU/memory/network/disk; timers, clipboard, colour picker, calendar; extension API package. Gesture model: two-finger swipe down opens when hover-open is disabled, swipe up closes; horizontal swipes in the music pane skip track or seek +/-10 s (choice of behaviour per gesture vs button). Per-feature toggles, shortcut remapping. Open issues include Space-switch overlay glitches and OS-beta compatibility.
Source: https://github.com/Ebullioscopic/Atoll

**DynamicLake Pro** (Aviorrok). Paid, macOS 14+. Modules: DynaMusix (music), DynaGlance (calendar/weather), DynaConnect (Bluetooth/AirPods connect and disconnect), DynaKeys, notifications and message alerts, calls (FaceTime/Phone), AirDrop, timer, audio/video conversion, battery and calendar alerts, "miniLake" smaller format, Liquid Glass look, external display support. Reputation for responsive developer support (Reddit 2024).
Source: https://www.dynamiclake.com/

**MediaMate** (wouter01). About $7 one-time, macOS 13+. One job done well: replaces volume, brightness and keyboard-backlight HUDs with compact ones and parks Now Playing in the notch. Evidence that a narrow tool at low price sells.
Sources: https://wouter01.github.io/MediaMate/ , https://lifehacker.com/tech/mediamate-macos

**NotchDrop.** Free + Pro. File drop zone with AirDrop, local network sharing across macOS/Windows/Linux/iOS/Android, clipboard history, quick to-do, calendar, calculator, signature drawing, AI chat and notes with user-supplied API key, screen recording with cursor-following zoom, screenshot, screen draw, camera, music controls. Source: https://www.notchdrop.com/

**MewNotch.** Free GPL-3. Notch/island/HUD modes per display, brightness, volume, power-state HUDs, stock-HUD suppression, file shelf that persists across restarts, mirror, custom bash-script view, configurable hover delay, about 57 MB baseline. Source: https://github.com/monuk7735/mew-notch

**Notchmeister.** App Store; decorative notch effects (Glow cursor light, Cylon, Plasma Leak, Festive). Not a utility. Source: https://apps.apple.com/us/app/notchmeister/id1599169747 (snippet only **[unverified]**).

**Others noted:** Notchy ($24.99, Face Unlock, stocks, per-app volume, AI usage tracking), Seam ($19.90-29, dictation, Pomodoro, AirPods per-earbud battery), Droppy ($9.99, clipboard, 36 extensions, "dynamic island mode" for non-notch Macs), Sapphire, NotchBox, Crest, Brow. All from rival-authored rankings **[unverified]**.

### iOS Dynamic Island / Live Activities (the design reference)

Three presentations: compact (leading + trailing flanking the sensor), minimal (when multiple activities compete), expanded (touch and hold; regions leading, trailing, center, bottom). Live Activities also appear on the Lock Screen and StandBy. Apple's HIG page requires JavaScript, so details come from search snippets of Apple documentation and WWDC23 notes **[partially unverified]**. From general knowledge, not re-verified here: updates arrive via ActivityKit or push, there is a data size limit, and an activity ends after a time limit; I did not confirm exact numbers.
Sources: https://developer.apple.com/documentation/activitykit/displaying-live-data-with-live-activities , https://developer.apple.com/videos/play/wwdc2023/10184/ , https://wwdcnotes.com/documentation/wwdc23-10184-meet-activitykit/

---

## 4. What users praise and complain about (across the category)

| Praise | Complaint |
|---|---|
| Looks native and moves like the iPhone island (Alcove, Notchify, NotchNook) | Battery, CPU or RAM cost: NotchNook 40% CPU / GBs RAM; boring.notch visualizer; Dynamic Edge 75-180 MB |
| Light and event-driven (Nusa 0% idle CPU, Pillar 3 MB) | Crashes after sleep/wake or on startup (boring.notch, NotchNook) |
| File shelf that references files and never deletes originals (Nusa, AWNA) | Always-on-top / z-order failures; fullscreen game stutter (Notchify fixed these) |
| Click a lyric line to jump | "Half-baked features thrown in for novelty" (NotchNook) |
| Responsive developer support (DynamicLake) | Licence or trust failures (NotchNook 2026), unsigned installers and SmartScreen/Gatekeeper warnings |
| Free and open source with a clear privacy statement | Notch app "fatigue" and vibe-coded clones; unclear value on non-notch hardware ("thick bezel plus notch is odd") |
| Auto-hide, per-app fullscreen rules | Mirror too small, shortcuts that cannot take dropped files (NotchNook) |

---

## 5. Features Tuck should add or improve (ranked)

Ranking = (frequency across competitors) x (user-visible value) / effort, with a bonus for fit with Tuck's privacy-first, quiet design. Effort: S under 2 days, M under 2 weeks, L over 2 weeks. Windows API names are from my knowledge of the platform and should be validated in a spike before committing; I did not re-verify every API in this session.

| # | Feature | Why (evidence) | Effort | Windows API / approach |
|---|---|---|---|---|
| 1 | **Reliability hardening: z-order, fullscreen, sleep/resume, multi-monitor, co-existence with another notch app** | The top complaint class; Notchify's changelog is mostly these; Tuck's own smoke was blocked by nusa.exe at the same position | M | `SetWindowPos(HWND_TOPMOST)` re-assert on `WM_DISPLAYCHANGE`, `WM_POWERBROADCAST` (resume), `SHQueryUserNotificationState` for fullscreen/D3D, `EVENT_SYSTEM_FOREGROUND` hook; detect other overlay windows and warn |
| 2 | **Idle cost budget and event-driven rendering** (stop timers/animation frames when idle, document RAM/CPU numbers) | Nusa and Pillar market 0% idle CPU; NotchNook and Dynamic Edge criticised. Tauri's WebView2 baseline is the risk | M | Pause rAF when hidden; native callbacks (GSMTC events, `WM_POWERBROADCAST`) rather than polling; measure with ETW or Task Manager |
| 3 | **Bluetooth peripheral battery + connect/disconnect peek** (already on Tuck's deferred list) | DynamicWin, Nusa, Notchify, DynamicLake DynaConnect, Atoll widgets | M | `Windows.Devices.Enumeration.DeviceWatcher` + `Windows.Devices.Bluetooth` GATT Battery Service (0x180F) for BLE; for classic audio devices read `DEVPKEY_Bluetooth_Battery` via `SetupDi`/PnP property (`System.Devices.Aep.*`) |
| 4 | **Mic/camera in-use indicator as a compact activity** (deferred item) | DynamicBar, Atoll privacy indicator | S-M | Registry `HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\{microphone,webcam}` `LastUsedTimeStart/Stop` (DynamicBar uses this); change notification via `RegNotifyChangeKeyValue` |
| 5 | **Notification panel upgrade: stack, swipe to dismiss, click to open source app** | Notchify (unique among Windows apps per its own table), Pillar history. Tuck already has previews + permission flow | M | `Windows.UI.Notifications.Management.UserNotificationListener` (needs package identity and user permission; already scoped in Tuck) |
| 6 | **Volume/brightness/lock-key/layout HUD as a compact activity, plus scroll-on-pill to change volume** | Nusa, Notchify, Pillar, FluentFlyout, DynamicBar; MediaMate and MewNotch prove demand on macOS | S-M | `IAudioEndpointVolume` / `IAudioEndpointVolumeCallback` (Core Audio); brightness via WMI `WmiMonitorBrightness` or DDC/CI `SetMonitorBrightness`; `GetKeyState` + low-level keyboard hook for Caps/Num Lock; `WM_INPUTLANGCHANGE` for layout. Hiding the stock OSD is fragile; show alongside it initially |
| 7 | **Compact leading/trailing + minimal + expanded model for stacked live activities** (formalise like iOS) | iOS model; Alcove/Atoll Live Activities; DynamicWin "spring notches"; Tuck already has priority registry | M | UI only (activity registry contract with compact-leading, compact-trailing, minimal, expanded slots; touch-and-hold analogue = click/hold) |
| 8 | **Gestures: horizontal swipe on media = next/prev or +/-10 s; swipe to dismiss notifications** | Alcove, Atoll | S-M | Pointer events plus `WM_MOUSEWHEEL`/precision-touchpad `WM_POINTER`; Windows Precision Touchpad gestures arrive as pointer or wheel input to the webview |
| 9 | **Per-app fullscreen/hide rules and position options (top/bottom/edges)** | BuildNotch 1.2.0 rules; Notchify 8 positions; Dynamic Edge side docking | S-M | Foreground process name via `GetForegroundWindow` + `QueryFullProcessImageName`; `SHAppBarMessage(ABM_GETTASKBARPOS)` for taskbar avoidance |
| 10 | **Media polish: click-lyric-to-seek, "Up Next" peek, compact-idle lyric line, per-player filter** | Notchify, Nusa, Dynamic Edge (lyrics in idle capsule), FluentFlyout Up Next | S-M | GSMTC `GlobalSystemMediaTransportControlsSessionManager` (`SessionsChanged`, `TimelinePropertiesChanged`, `SetPlaybackPositionAsync`); next-track from the same session where exposed |
| 11 | **Clipboard: drag items out, pin, image preview** (confirm Tuck's current coverage first) | Notchify is the only one with drag-out; BuildNotch grid with filters | S-M | `AddClipboardFormatListener`, OLE drag-and-drop source (`IDataObject`), Tauri drag-out plugin or native `DoDragDrop` |
| 12 | **Active-app / context action chips** (deferred) | Pillar "knows which app you're in" | S | `GetForegroundWindow` + `GetWindowText`, process name only; keep privacy promise (no content) |
| 13 | **Phone-to-PC quick transfer via QR over LAN** | Notchify, NotchDrop Local Sharing | L | Local HTTP server on a random port with one-time token + QR; firewall prompt is the cost. Optional, off by default |
| 14 | **Signed installer and published privacy stance** | Unsigned warnings are a repeated friction (Nusa, BuildNotch, AWNA, Pillar); Tuck's release pipeline is out of scope today | M (cost is cert, not code) | Authenticode/Azure Trusted Signing; MSIX for Store (also gives package identity for the notification listener) |
| 15 | **Minimal/hairline idle mode and idle personality (optional)** | Nusa minimal notch and mascot; Dynamic Edge pet; useful for thick-bezel laptops | S | UI only |
| 16 | **Weather and calendar widgets** (weather deferred) | Notchify, Smart Notch, DynamicLake DynaGlance | M | Open-Meteo style HTTP call (opt-in, privacy note) or no weather at all; `Windows.Devices.Geolocation` is the heavier path |
| 17 | **Extension/plugin API** | Atoll AtollExtensionKit, DynamicLake extensions, boring.notch roadmap | L | Out-of-process widgets over a local IPC channel; defer until core is stable |

Deliberately **not** recommended: more bundled utilities of the AWNA/BuildNotch kind (UUID generator, JSON formatter, code playground, mascot-heavy features, Discord/OBS controls) and a feature-count race. The macOS record (NotchNook) shows that breadth without reliability is what users penalise.

### Suggested sequencing

1. Items 1-2 (stability and idle budget). They are the market's loudest complaint and cheap relative to value.
2. Items 3, 4, 6 (Bluetooth battery, privacy indicator, HUDs). Already on or near Tuck's deferred list, widely expected.
3. Items 5, 7, 8, 10 (notification panel, activity presentation model, gestures, media polish). Raises perceived polish toward the Alcove/Notchify bar.
4. Items 9, 11, 12 next; items 13-17 only after a decision on distribution.

---

## 6. Caveats and gaps

- Pricing and feature status change fast (several apps shipped updates in the last month). Numbers are as of Oct 29 - Nov 3, 2026 pages.
- Store pages for Dynamic Edge and NotchX returned empty content; those rows rely on Reddit posts, GitHub README and search snippets.
- I did not read the Apple HIG body, the ModernFlyouts site, Isle Notch, Smart Notch, NotchPixel, Sapphire, or review sites other than those cited. No Reddit threads on user complaints about Notchify or Alcove specifically were fetched.
- Rival-authored comparison pages (Notchify, Seam, Droppy, Notchy) were used for feature tables and are biased toward their own product.
- Windows API suggestions have not been prototyped on this machine. Package-identity requirements (notification listener, some BLE and background APIs) interact with Tuck's current unpackaged setup and should be tested first, as `docs/feature-matrix.md` already notes.
