# Tuck feature scope and acceptance

Source: supplied notch-island-build-guide.md. User confirmed on 2026-11-02: specified Windows app plus v1; track later ideas separately. No automatic publication or release is included.

## Delivery inventory

| Features | Owner / PR | Acceptance evidence required |
| --- | --- | --- |
| Transparent 520 by 230 logical pixel canvas; always on top; taskbar/Alt-Tab exclusion; no focus stealing | Native / PR 3 | Windows compile and native smoke |
| Idle 212 by 32, hover peek 300 by 38, open 440 by 180; close timing | UI / PR 1 | Rendered state and geometry smoke |
| Hanging notch with concave ears; floating pill with 10 pixel gap; live persistent style choice | UI / PR 1 | Settings and storage smoke |
| Spring open, quick close, page motion, reduced motion, SVG icons, focus rings | UI / PR 1 | CSS inspection and rendered smoke |
| DPI-aware cursor hit testing; outside click-through; primary display repositioning | Native / PR 3 | Geometry tests at 100/125/150/200 percent and negative origins; native smoke |
| Fullscreen hiding and click-through on the island monitor | Native / PR 3 | Rectangle tests plus fullscreen native smoke |
| Real SMTC title, artist, artwork with retries, accent, overflow marquee, playback, previous/next and seeking | UI + Native / PRs 1 and 3 | Runtime tests, native compile, real-player smoke |
| Today clock/date, battery/AC, charging, low and unknown states | UI + Native / PRs 1 and 3 | Power parsing tests and native smoke |
| Arrows, dots, wheel and keyboard navigation; accessible seek and hidden-layer handling | UI / PR 1 | Rendered interaction smoke |
| Tray Open island, Settings and Quit; keyboard focus entry | Native / PR 3 | Native smoke |
| Separate settings window; persistent budgets and style; opt-in startup | UI + Native / PRs 1 and 3 | Settings smoke and autostart native check |
| Claude status-line limits, stale flags, UTC API costs, budgets and reset countdowns | Usage / PR 2 | Parser/cache tests and verified official schema |
| Write-only Credential Manager keys, explicit removal, redacted errors | Usage + UI / PRs 1 and 2 | Allowlist/redaction tests and settings failure smoke |
| ChatGPT manual rolling message estimate, configured cap/window, expiry, clear log | UI / PR 1 | Real module tests including corrupt storage and seven-day window |
| Priority activity registry, timer, stopwatch and Pomodoro | UI / PR 1 | Timer tests and rendered layout/control smoke |
| Downloads and manual call live activities; priority compact override | V1 integrations | Opt-in, unknown totals honest, start/stop tests |
| Notification previews, permission request and unsupported access state | V1 integrations | WinRT compile; permission/native smoke with app identity constraints recorded |
| Local ICS calendar, upcoming events, five-minute reminder and join action | V1 integrations | ICS parser, timestamp and safe URL tests |
| Real Wi-Fi/Bluetooth radio state/control where Windows grants permission; Focus settings; screen capture | V1 integrations | WinRT compile, denied/unavailable states, allowlisted actions |
| File shelf, native drops, remove/clear and drag out | V1 integrations | Native drag/drop smoke; never removes original files |

## Deferred ideas from the guide

These are explicitly tracked, not claimed implemented: plugin widgets; lyrics; clipboard history; weather/stocks/crypto/sports; CPU/RAM/disk/network stats; disk-space alerts; Caps/Num Lock HUD; active-app context; lock/unlock behavior; WASAPI audio-reactive equalizer; volume HUD; mic/camera indicators; meeting-app mute automation; Wi-Fi/VPN quality; Bluetooth peripheral battery; USB safe eject; battery health; GPU/VRAM; hardware temperatures/fan drivers; Windows Update/reboot alerts; Docker/WSL; Git/CI/deploy status; local development server discovery; macOS/Linux builds; signing certificate and updater/release pipeline. Calendar OAuth providers can extend the local ICS implementation later.

## Review and merge policy

Each implementation goes through a feature PR. Coordinator reads actual code, resolves confirmed findings, and runs relevant checks before merge. Fresh read-only review packets provide independent review. A compile/test pass does not establish real desktop, hardware, permission, or provider-account behavior. Manual checks are recorded separately and never inferred from unit tests.
