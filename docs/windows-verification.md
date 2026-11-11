# Windows Native Implementation Verification

This document records the automated test results and manual verification details for Tuck's native Windows implementation on branch `windows-native-services`.

## 1. Automated Test Suite

Executed via `cargo test --manifest-path src-tauri/Cargo.toml`:

- `test_hit_bounds_100_percent_dpi`: Validates cursor hit testing at 100% DPI (1.0 scale factor) including 6px edge slop and horizontal centering.
- `test_hit_bounds_various_dpis`: Validates hit boundary calculation across 125%, 150%, and 200% Windows display scaling.
- `test_hit_bounds_negative_monitor_origin`: Validates multi-monitor physical coordinates where the primary or secondary monitor has negative top/left coordinates.
- `test_clamp_hit_size`: Confirms logical CSS dimensions are safely clamped within the 520x230 window boundary.
- `test_fullscreen_rect_classification`: Confirms that true fullscreen detection strictly checks that the window covers the entire monitor rectangle, distinguishing it from maximized windows (which respect the taskbar and desktop work area), with no inaccurate 12px tolerance heuristic.
- `test_clamp_seek_validation`: Validates media seek parameter clamping (no negative seek, clamped to track duration).
- `test_ticks_ms_conversion`: Validates bidirectional conversion between 100ns WinRT ticks and milliseconds, including saturating arithmetic overflow protection.
- `test_power_unknown_and_desktop_handling`:
  - `status_known: false` and `percent_known: false` on unknown battery state (flag 255).
  - Desktop PC without battery correctly reports `has_battery: false`, `status_known: true`, `ac_connected: Some(true)`.
  - Discharging laptop reports `charging: false`, `ac_connected: Some(false)`, valid percentage and minutes remaining.
  - Charging laptop uses ONLY flag 8 (charging bit); does NOT guess charging from AC line status when flag 8 is unset.
  - Unknown percentage (255) reports `percent_known: false` while preserving fallback numeric value 100 for DTO frontend compatibility.
- `test_quick_action_allowlist`: Validates strict allowlist of system URIs (`wifi`, `bluetooth`, `focus`, `capture`).
- `test_media_state_defaults`: Validates initial idle media state.

Result: **10 passed; 0 failed; 0 warnings**.

---

## 2. Event and Command Contracts

### Native Events Emitted

| Event | Payload Type | Polling Rate | Description |
|---|---|---|---|
| `hover` | `bool` | 40 ms | Emitted when cursor enters or leaves the island hit rectangle. Toggles cursor click-through on the transparent window. |
| `fullscreen` | `bool` | 40 ms | Emitted when a fullscreen window or presentation is active on the **same monitor** as Tuck. Hides the island. |
| `media` | `MediaState` (`camelCase`) | 500 ms | Current SMTC session status (title, artist, playing, position, duration, and cached base64 thumbnail). |
| `power` | `PowerState` (`camelCase`) | 2000 ms | Battery and AC line status. Emitted on every poll so frontend receives initial state even when subscribing late. |
| `open-island` | `()` | Event-driven | Emitted when user selects 'Open island' from the system tray menu or calls `open_island_cmd`. Signals frontend to expand island and accept keyboard navigation. |

### DTO Schema: `PowerState`

```typescript
interface PowerState {
  percent: number;          // 0-100 (numeric fallback preserved for frontend DTO compatibility)
  charging: boolean;        // true ONLY when battery flag 8 is set
  hasBattery: boolean;      // true if physical battery is present
  minutesLeft: number | null; // estimated minutes remaining
  percentKnown: boolean;    // false if BatteryLifePercent is 255 (unknown)
  statusKnown: boolean;     // false if BatteryFlag is 255 (unknown)
  acConnected: boolean | null; // true (online), false (offline), null (unknown)
}
```

### Tauri Commands Registered

| Command | Arguments | Return Type | Description |
|---|---|---|---|
| `set_hit` | `w: u32, h: u32` | `()` | Reports CSS dimensions of current island state (`idle`, `peek`, `open`). Clamped to 520x230. |
| `quick_action` | `name: String` | `Result<(), String>` | Launches safe Windows URI (`wifi`, `bluetooth`, `focus`, `capture`). |
| `open_settings_cmd` | None | `Result<(), String>` | Opens or focuses the 460x700 settings window. Explicit error return avoids phantom success. |
| `open_island_cmd` | None | `Result<(), String>` | Temporarily removes `WS_EX_NOACTIVATE`, focuses window, and emits `open-island`. |
| `get_autostart` | None | `Result<bool, String>` | Queries Windows registry via `tauri-plugin-autostart` to check if launch at login is enabled. |
| `set_autostart` | `enable: bool` | `Result<(), String>` | Enables or disables launch at startup in Windows registry. |
| `media_toggle` | None | `Result<(), String>` | Toggles play/pause on current SMTC session. |
| `media_next` | None | `Result<(), String>` | Skips to next track on SMTC session. |
| `media_prev` | None | `Result<(), String>` | Skips to previous track or seeks to beginning. |
| `media_seek` | `ms: i64` | `Result<(), String>` | Seeks playback to given millisecond position. |

---

## 3. Keyboard Entry Route & Window Styles

1. **Alt+Tab and Focus Isolation**:
   - `apply_island_styles` applies `WS_EX_TOOLWINDOW` and `WS_EX_NOACTIVATE` via `SetWindowLongPtrW`.
   - Window remains invisible in Alt+Tab and does not steal focus on mouse clicks.
2. **Keyboard Entry Route**:
   - System tray menu includes **"Open island"** item.
   - When triggered, `system::open_island` dynamically removes `WS_EX_NOACTIVATE` (`set_island_focusable(hwnd, true)`), removes cursor pass-through, focuses the window, and emits `open-island`.
   - On `WindowEvent::Focused(false)`, `WS_EX_NOACTIVATE` is immediately restored so the window returns to non-activating behavior.
3. **Monitor Placement & Resize Loop Prevention**:
   - `placement::apply` computes the window rectangle from the chosen monitor (`placement::island_rect`: the monitor's rectangle, its own DPI scale and the dock) and compares it with the window's current rectangle before it moves anything, preventing recursive `WindowEvent::Moved` loops. Position and size move in one `SetWindowPos` call.
   - `WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE`, `WindowEvent::Moved` and `ScaleFactorChanged` re-run it, so a monitor that was unplugged falls back to the primary monitor and the island returns when it is plugged in again.

---

## 4. Manual Verification & Smoke Test Checklist

### Automated checks verified locally (CI tracked separately)
- [x] `cargo check --manifest-path src-tauri/Cargo.toml` compiles cleanly with 0 errors.
- [x] `cargo test --manifest-path src-tauri/Cargo.toml` executes and passes all 10 unit tests with 0 failures and 0 warnings.
- [x] `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` validates Rust code formatting.

### Unverified Manual Smoke Cases (Requires Interactive Windows Desktop GUI Session)
- [ ] Visual transparency: Window renders transparent background with black island notch and concave ears.
- [ ] Click-through: Mouse clicks pass through the transparent region around the island to background applications.
- [ ] Hover expansion: Moving cursor over island triggers `hover: true` and transitions from `idle` to `peek`.
- [ ] SMTC Media integration: Media title, artist, playback controls, and artwork sync with active player (Spotify, Chrome, VLC).
- [ ] Fullscreen suppression: Playing a borderless or fullscreen game on the island's monitor hides the island.
- [ ] Placement: Settings > Island > Display and Position move the island at once, on a second monitor at another DPI scale, left of or above the primary monitor, and back to the primary monitor when the chosen one is unplugged.
- [ ] Tray interaction: Right-clicking tray icon displays "Open island", "Settings...", and "Quit Tuck".
- [ ] Keyboard entry route: Selecting "Open island" focuses island, handles Arrow/Enter keys, and re-locks focus on blur.
- [ ] Autostart toggle: Toggling autostart in Settings updates `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`.

## Sol low review follow-up

The independent GPT 5.6 Sol low review identified two native blockers. The cursor poller now reconciles click-through every tick, including after tray activation when no hover transition happens. The frontend invokes `deactivate_island` whenever it leaves the open state, restoring WS_EX_NOACTIVATE and click-through. `fullscreen_now` exposes the cached state after frontend listener registration so startup events cannot be lost. Native `media_now` similarly supplies a cached initial media snapshot; artwork is retried up to ten polls and only retransmitted on change. Media controls return errors and seek relative to the timeline start.

These changes require the coordinator integration PR frontend hooks. Desktop-without-battery rendering and persisted-settings failure feedback are corrected in that PR as well. Native real-desktop validation remains distinct from parser/geometry tests.
