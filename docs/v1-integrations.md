# Windows v1 integrations

Implemented using real Windows calls and a local activity store. Optional monitoring defaults off. Settings and imported calendar files live in the app data directory; shelf references and manual calls live in memory. No account logins or OS permission settings are changed automatically.

## Features and limits

- Calls: a manual call timer with highest compact priority. It neither places calls nor controls a microphone.
- Downloads: reads the Windows Known Folder Downloads directory only after opt-in, reporting partial-file bytes and observed growth rate. Total size is unknown, so progress remains indeterminate.
- Calendar: imports up to 2 MB and 2,000 individual ICS events, including UTC, local timestamps, and all-day dates. Named time zones and recurrence return explicit unsupported errors instead of incorrect reminders. Calendar stores locally and emits one five-minute reminder per event while running. Join actions use validated HTTP(S) URLs from imported event IDs.
- Notifications: invokes actual UserNotificationListener APIs. Only the explicit settings action initiates RequestAccessAsync, dispatched on the UI thread. Polls previews only when enabled and access is allowed; real permission/API failures remain visible. Windows package identity and notification capability may be required by the installation environment. This work does not claim permission has been granted or package-identity deployment has been validated.
- Radios: enumerates actual Wi-Fi/Bluetooth radios and requests Windows access before SetStateAsync. Unknown/disabled/unavailable hardware is displayed honestly. Focus remains an action opening Windows Settings because no stable public Focus toggle API is assumed.
- Shelf: native Tauri file drops or native file picker stage references. Reveal opens the containing folder without executing files. Native drag uses drag 2.1.1 on the UI thread in copy mode and validates a saved shelf ID. Remove/Clear never deletes originals. Maximum 100 shelf references.

## Sources checked

- [Microsoft notification listener](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/notification-listener): capability, UI-thread access request, access checks, and toast retrieval.
- [drag crate source and API](https://docs.rs/drag/2.1.1/drag/): native Windows file dragging with raw-window-handle 0.6.
- Windows 0.58 generated bindings installed in the Cargo registry were checked against compilation for Devices.Radios and UI.Notifications.Management.

## Validation

Native module compile and Clippy passed on Windows. Seven production-module tests cover opt-in defaults, calendar parsing/errors and concurrent import serialization, safe URLs, real temporary partial downloads, and shelf non-deletion. Frontend build and unit tests pass. Final combined application interaction and CI checks are tracked in the integration PR.

Real hardware radio changes, notification permission, drag/drop into another desktop application and calendar reminder delivery remain separate manual checks; fixture tests do not establish those behaviors.

## Sol low review fixes

Radio mutations capture an opt-in epoch and recheck it immediately before each Windows SetStateAsync request. Calendar imports and calendar-setting reloads share a dedicated operation lock so disk and memory commits stay ordered. The UI rebuilds live state and restores focused controls by stable action/item identity instead of freezing updates. These fixes passed combined tests and Clippy.
