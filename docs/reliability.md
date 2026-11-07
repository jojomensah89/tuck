# Reliability verification

Base: origin/main fd15d8d. Installed Tuck was not stopped. No runtime CPU measurements claimed.

## Idle wake audit (collapsed, no playback)

| Worker | Before | After |
| --- | --- | --- |
| Cursor hit testing | 40 ms near / 60 ms far | Moving: 40 ms near / 60 ms far. Resting: 100 ms near / 250 ms far. No global mouse hook |
| Local usage | 1 s wake / 60 s file refresh | 60 s or immediate manual request |
| Installed-client usage | 300 s when enabled, parked otherwise; explicit refresh wakes it | Unchanged |
| WebView trimming | One-shot 30 s after inactive | Unchanged |
| Update check | 20 s after startup, then 6 h | Unchanged |
| Telemetry | Request-driven, no idle timer | Unchanged |
| Media | Events + 30 s safety; manager failure retry 2/5/10 s | Unchanged |
| Clipboard | Event-driven; busy retries 20 ms to 1 s, ten attempts | Unchanged budget; duplicate notifications no longer force reads |
| Notification extension | 1 s even disabled | Parked when disabled; 1 s when opted in |
| Desktop signals | 250 ms; layout 1 s, Wi-Fi fallback 30 s | Unchanged; maintains brightness ramp / Caps Lock / OSD timing |
| Lifecycle | Native events; 200 ms layout debounce / 1 s maximum; fullscreen safety 1 s only when hidden | Unchanged |
| Frontend compact clock | Next minute boundary | Unchanged |
| Frontend media seek | 200 ms only playing | Unchanged, stopped idle |
| Frontend open render | 1 s only open | Unchanged, stopped collapsed |
| Frontend telemetry / sessions | 1 s / 5 s only relevant open page | Unchanged, stopped collapsed |
| Activities | Timer only while an activity runs | Unchanged, absent with no activity |
| Animation frames | One-shot transitions / style application | Unchanged, no continuous idle loop |
| Dictation | Capture / shortcut pause timers only in active sessions | Unchanged idle |

Live hover, fast approach-and-click, native credential migration and CPU usage still require Windows runtime validation.

Preview preparation checks stop before mel conversion and before language detection. Individual native calls cannot be interrupted; cancellation occurs at their nearest safe boundary. Preview failure logs a generic line and pauses previews, preserving final transcription. Final model-load errors already propagate through the session into the dictation error status. Settings already renders synchronous action failures, model notices, and asynchronous error status.

Credential cleanup follows removed usage.rs (service tuck; users anthropic-admin/openai-admin) and keyring 3 Windows target generation user.service. It deletes exactly two generic targets without reading, writes an app-data migration marker even on deletion failure, and logs generic failure text only. Tests inject the deleter and never touch actual credentials.

Dependabot #49: keep the windows-core 0.58 dependency required by windows 0.58 and the 0.62 alias required by webview2-com 0.39. Updating the former alone to 0.62 makes Cargo see the same crate under two names and also breaks the 0.58 COM types. Exclude that incompatible update, or migrate the whole windows API family and unify imports in a separate change.
