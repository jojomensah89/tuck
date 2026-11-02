# Phase two integration verification

Integration is in draft PR 6; main has not been changed. PR 7 supplies the original native media packet. Coordinator recovered the interrupted dictation/clipboard worker's files and completed native registration, compilation, frontend adapters, GPU counters, UI Automation target checks, WASAPI loopback, system peeks and build prerequisites on the integration branch.

## Scope delivered in code

- Artwork-derived muted bottom gradient; compact lyrics/title; configurable hover, delays, pin, size and system peeks; split lyrics/player, source selection and capability-aware transport.
- LRCLIB opt-in lookup with bounded response/cache and rate backoff. Request-scoped replies and event correlation reject obsolete playback instances. Online-lyrics and loopback opt-ins are persisted; no audio is persisted.
- Real CPU/RAM and named GPU usage, plus dedicated GPU memory readings when available. GPU process samples are summed within each engine; the busiest engine supplies adapter utilization, matching the documented Windows model. Missing counters remain null. A software adapter is hidden when physical GPUs exist.
- Measured loopback spectrum in the media pane and a dedicated visualizer screen. CPAL's Windows WASAPI backend transparently uses loopback for a render-device input stream. Audio is bounded, in memory and discarded; microphone capture is separate. Output changes restart loopback; reduced motion hides animation.
- Offline Whisper tiny dictation, explicit verified model download, toggle/hold shortcuts, partial transcript preview, immediate insertion by default and optional review. UI Automation runtime identities support accessible editable browser/Electron controls in addition to standard Windows edits. Password/read-only controls are rejected, focus is rechecked, modifiers must be released, clipboard is untouched and Enter is never sent.
- Clipboard text history: opt-in, bounded, delete/clear/pin/copy, clear on exit unless persistence is enabled. Mutations and persistence serialize. Cooperative private/password/transient formats are excluded; unmarked sensitive text cannot always be identified.
- Local to-dos/reminders while Tuck is running, existing timers/Pomodoro/shelf retained. Native volume, supported WMI brightness, Wi-Fi connectivity, audio-output, Caps Lock/input-language and charger peeks. No water reminders.

## Recorded evidence, 2026-11-02

Frontend build and all 29 Bun tests pass. Browser/native-bridge fixture smoke passes, including A→B→A lyrics rejection, out-of-order telemetry rejection, clearing measurements after a failed poll, selected-tool retention, text injection safety, native command routing and existing v1 interactions. These use synthetic data and are not hardware tests. Impeccable detector returned no findings.

Full native suite: 70 passed (53 library, 7 phase-two integration, 10 existing Windows integration), 3 intentionally ignored live diagnostics. Full Clippy with warnings denied passed at the prior integration checkpoint; it is rerun after final changes. A Common Controls v6 linker manifest fixes test executables' missing TaskDialogIndirect entry point; the Tauri resource manifest is disabled so app targets do not receive duplicate manifests.

Read-only Windows diagnostics passed twice. Spotify exposed real title, artist, artwork, timeline, playing state and seek/play-pause capability. No transport command changed the user's playback. Real CPU/RAM samples and utilization/dedicated-memory readings were returned for NVIDIA GeForce RTX 3060 Laptop GPU and Intel UHD Graphics. Exact values vary with workload. The diagnostic does not establish Task Manager numerical equivalence at different sample intervals.

Whisper model metadata was checked directly against Hugging Face: ggml-tiny.bin is 77,691,713 bytes, SHA-256 be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21. No speech model was downloaded and no microphone activated during development. Model download and recording remain explicit user actions.

## Open verification gates

The integrated frontend review identified keyboard focus remaining in an inert outgoing page and a two-command clipboard settings partial commit. Both are fixed: navigation relocates focus, clipboard_configure writes both preference flags together, failed writes refresh displayed native state, UI smoke asserts the behavior, and a real Windows file-lock regression test confirms failed atomic replacement preserves both flags. The original 70-test suite plus this new focused native test pass; Clippy and updated UI smoke also pass.

Independent Sol low review of the integrated head and fresh Windows CI must finish before merge. Live pause/next/seek/player switching, authenticated real-track lyric matching, system-audio capture, actual microphone transcription and insertion into representative Windows/browser inputs, clipboard marker/persistence behavior, WMI brightness/radio permissions, device peeks, DPI and click-through remain to verify. An existing Nusa overlay previously blocked full same-position Tuck desktop smoke; it has not been stopped by this work.

UI Automation cannot guarantee support for every custom editor, protected/elevated window or password implementation. Generic audio-output changes do not claim universal headphone battery telemetry. Brightness requires a monitor exposing WmiMonitorBrightness; external-monitor DDC control and Bluetooth peripheral battery are deferred. To-do reminders do not run when Tuck is closed. No public release, signing or updater publication is included.

## Build checks

In PowerShell: `.\scripts\prepare-native.ps1` (first use add `-InstallBuildTools`), then `cargo test --manifest-path src-tauri/Cargo.toml`, `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`, and `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`. Read-only hardware/media diagnostics: `cargo test --manifest-path src-tauri/Cargo.toml --lib live_windows -- --ignored --nocapture`.

Review fixes are not certified by an earlier head's green checks. Keep main unchanged until the final head is reviewed and verified.

## Final review corrections

At the final regression checkpoint, 75 native tests pass (58 library, 7 phase-two integration, 10 Windows integration), with 3 explicit live diagnostics ignored. Formatting, Clippy with warnings denied, frontend build, all 29 Bun tests and UI smoke pass.

Confirmed review findings are corrected: model removal checks active startup and download lifecycle under the runtime lock; the UI disables unavailable removal. Partial SendInput acceptance or focus loss after an accepted prefix prevents whole-transcript retry, exposes a manual reconciliation message and disables Insert. Review buffers are taken and zeroized on completion/discard. Late partial transcript events cannot restore a completed recording activity. A stop received during model loading prevents microphone startup.

The reported Windows rename defect was not reproduced: Rust explicitly documents replacement of an existing destination, and Windows regression tests save false, true and false to the same file successfully. Keep replacement atomic rather than removing the destination first. Source: https://doc.rust-lang.org/std/fs/fn.rename.html . Clipboard settings failure tests also verify both preference flags remain unchanged on replacement failure.

The live GPU Adapter Memory counter contains adapter-level `luid_..._phys_0` instances, rather than process-scoped instances. The aggregation now sums distinct physical nodes of the requested adapter while excluding process-counter names and duplicate nodes; a fixture covers multiple nodes and processes. Process shared-memory totals must not be substituted for adapter totals. Source: https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/ .

These corrections require a fresh exact-head independent review and CI before merge. The live verification gates above remain open and are not implied by fixture tests.

The final independent review of f6e4a84 confirmed the earlier corrections and found model removal could strand a pending review transcript. Native removal now rejects review state, Settings disables Remove, and a regression test confirms the transcript remains in review and can be explicitly discarded/zeroized. Both model-removal tests and the frontend build pass. The corresponding UI smoke asserts Remove is disabled during review. This is a narrow correction to the reviewed lifecycle guard; main remains unchanged pending current-head CI.
