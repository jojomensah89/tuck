# Tuck v1 Integrations & Native Extensions Guide

This document provides complete architectural specifications, coordinator wiring instructions, security models, event contracts, and acceptance verification for the Tuck v1 Native Extensions suite.

---

## 1. Feature Architecture Overview

Tuck v1 expands the dynamic island with 6 core native extensions tailored for Windows 11/10:

1. **Live Activity Downloads Monitor**:
   - **Privacy First**: Off by default. Explicit opt-in via Settings.
   - **Honest Tracking**: Monitors user Downloads directory (`%USERPROFILE%\Downloads`) for active partial files (`.crdownload`, `.part`, `.download`).
   - **Indeterminate Total Guarantee**: Calculates honest real-time transfer rates in B/s, KB/s, or MB/s. Total size is strictly reported as `null` / indeterminate (`Option<u64> = None`)—**never fabricating false completion percentages**.
2. **Manual Calls Activity**:
   - **Priority 95**: Ranks highest in the dynamic island hierarchy (above timers at 80 and media playback at 50).
   - **Manual Control**: Real-time elapsed counter and mute toggle without misleading claims of automatic VoIP detection.
3. **Local Calendar & 5-minute Meeting Alerts**:
   - **Local ICS Parser**: Parses RFC 5545 iCalendar files directly without external cloud or account credentials.
   - **URL Security**: Extracts meeting join links (Teams, Zoom, Google Meet) with strict scheme validation (`http://` and `https://` only; strictly rejects `javascript:`, `file:`, `data:`).
   - **Timely Alerts**: Delivers 5-minute warning payloads to both the compact dynamic island view and full island card.
4. **Windows Notifications Access**:
   - **WinRT UserNotificationListener**: Evaluates listener access on explicit user action.
   - **Honest Architecture Reporting**: Recognizes that unpackaged desktop Win32 applications lack the required MSIX package identity (`appx`), correctly reporting status as `Unavailable` / `DeniedBySystem` instead of faking success.
5. **File Shelf**:
   - **Allowlisted Launching**: Files dropped onto the shelf are registered in an in-memory UUID-keyed allowlist. Launching is permitted **only** via verified internal UUID, preventing arbitrary shell command execution.
   - **Non-Destructive**: Removing items or clearing the shelf purely deletes in-memory references, never deleting underlying files on disk.
6. **Hardware Radios & Focus Assist**:
   - **WinRT Radios**: Queries WiFi and Bluetooth hardware states via Windows `Devices.Radios` API with appropriate permission handling.
   - **Focus Assist**: Honors Windows system boundaries by invoking the official Windows Settings action (`ms-settings:quiethours` or `ms-settings:focusassist`) rather than fabricating toggles.

---

## 2. Coordinator Wiring Instructions

The coordinator integrates these extensions into the main application as follows:

### Step 2.1: Rust Backend Wiring (`src-tauri`)

#### 1. `Cargo.toml` Dependencies & Features
Ensure `src-tauri/Cargo.toml` includes:
```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tauri = { version = "2", features = [] }

[target.'cfg(windows)'.dependencies.windows]
version = "0.58"
features = [
  "Foundation",
  "Foundation_Collections",
  "Win32_Foundation",
  "Win32_UI_Shell",
  "Win32_UI_WindowsAndMessaging",
  "Devices_Radios",
]
```

#### 2. Module Registration in `src-tauri/src/lib.rs`
Add the module reference:
```rust
pub mod extensions;
```

#### 3. Command Handler Registration in `tauri::Builder::default()`
Register the v1 extension commands:
```rust
.invoke_handler(tauri::generate_handler![
    // Core commands...
    extensions::get_extension_settings,
    extensions::update_extension_settings,
    extensions::get_downloads_state,
    extensions::start_call,
    extensions::end_call,
    extensions::toggle_call_mute,
    extensions::get_call_state,
    extensions::import_calendar_ics,
    extensions::get_calendar_events,
    extensions::check_calendar_reminders_now,
    extensions::request_notification_access,
    extensions::get_notification_access_state,
    extensions::shelf_add_paths,
    extensions::shelf_get_items,
    extensions::shelf_remove_item,
    extensions::shelf_clear,
    extensions::shelf_open_item,
    extensions::get_radios_state,
    extensions::set_radio_state,
    extensions::open_focus_settings,
    extensions::get_extension_state_now,
])
```

#### 4. Background Service Launch in `setup()`
In `lib.rs` `setup` hook:
```rust
extensions::start(app.handle().clone());
```

---

### Step 2.2: Frontend Wiring (`src/`)

#### 1. Island Full Layer (`src/main.ts`)
Import and initialize extensions before pages are queried:
```typescript
import { mountExtensions } from './extensions.js';

// Call before page transition handlers query DOM
mountExtensions();
```

#### 2. Settings Window (`src/settings.ts`)
Inject the opt-in settings controls:
```typescript
import { mountExtensionSettings } from './extensions.js';

// Call on settings window initialization
mountExtensionSettings();
```

#### 3. Stylesheet Import
Import `extensions.css` in `src/styles.css` or `<link rel="stylesheet" href="/src/extensions.css">` in `index.html` and `settings.html`:
```css
@import './extensions.css';
```

---

## 3. Security, Window Permissions & Privacy

| Area | Security Enforcement |
| :--- | :--- |
| **Window Origin Checking** | Privileged actions (`update_extension_settings`, `shelf_add_paths`, `shelf_open_item`, `set_radio_state`) strictly verify the caller is either `settings` or `main` `WebviewWindow` label. |
| **Shell Launch Allowlisting** | `shelf_open_item` accepts only valid 36-character UUIDs previously added to the active shelf map. Arbitrary string execution is prohibited. |
| **File Deletion Safety** | `shelf_remove_item` and `shelf_clear` strictly modify the in-memory map without deleting or altering files on disk. |
| **URL Sanitization** | `validateHttpUrl` strictly accepts only `http:` and `https:`. Any URI using `javascript:`, `file:`, `data:`, or `vbscript:` is rejected. |
| **Zero Secrets Stored** | Extension settings contain no tokens or sensitive user data. Persisted locally via secret-free format. |

---

## 4. Native Event Contracts & IPC DTOs

### Event 1: `extension-state`
Emitted by the background poller every 1000ms.
```typescript
export interface ExtensionStateDTO {
  settings: ExtensionSettings;
  downloads: DownloadsState;
  call: CallState;
  upcoming_event: CalendarEvent | null;
  active_reminder: CalendarReminderPayload | null;
  notifications: NotificationListenerState;
  shelf_items: ShelfItem[];
  radios: RadiosPayload;
}
```

### Event 2: `calendar-reminder`
Emitted when an upcoming calendar event is within `<= 5 minutes` of starting.
```typescript
export interface CalendarReminderPayload {
  eventId: string;
  title: string;
  startTimeMs: number;
  minutesUntil: number;
  joinUrl: string | null;
  location: string;
}
```

### DOM Event: `tuck-activity`
Dispatched on `window` for compact dynamic island slot coordination.
```typescript
export interface TuckActivityEventDetail {
  id: string;
  type: 'call' | 'download' | 'meeting' | 'shelf' | 'radio';
  title: string;
  text: string;
  priority: number; // Calls: 95, Meeting Alerts: 85, Downloads: 70
  active: boolean;
}
```

---

## 5. UI Design Specifications

- **Island Viewport**: Strict fit inside 440px wide × 180px high container with 136px vertical content area (`inset: 18px 46px 26px`).
- **Touch & Click Targets**: Minimum 36px height and width on all interactive buttons, tabs, and toggles.
- **Scrollable Panels**: Refined 4px custom scrollbar with subtle translucency.
- **Accessibility**: Full ARIA roles (`role="tab"`, `role="tabpanel"`, `aria-selected`, `aria-controls`), `inert` and `aria-hidden` toggling for inactive views.
- **Safe DOM Manipulation**: Exclusively uses `.textContent` and standard DOM constructors without untrusted `innerHTML`.

---

## 6. Acceptance Testing & Verification

1. **Rust Backend Tests**:
   - `test_optin_defaults_all_off`: Verifies monitors default to off.
   - `test_http_url_validation`: Verifies `https:` / `http:` acceptance and malicious scheme rejection.
   - `test_meeting_url_extraction`: Extracts links from locations and descriptions.
   - `test_ics_parser_rfc5545`: Parses standard RFC 5545 calendar entries.
   - `test_downloads_indeterminate_total_never_fabricated`: Verifies `total_bytes` is strictly `None`.
   - `test_call_state_priority_and_elapsed`: Verifies call priority 95 and non-negative elapsed duration.
   - `test_shelf_allowlist_security_no_file_deletion`: Confirms allowlisted lookup and non-deletion of files on disk.
   - `test_notification_listener_state_honest_acknowledgment`: Confirms honest reporting for unpackaged desktop apps.

2. **TypeScript / Frontend Unit Tests** (`bun run tests/extensions.test.ts`):
   - 38 test assertions covering formatters, URL sanitization, download contracts, call state priority, and DOM event payloads with 100% pass rate.
