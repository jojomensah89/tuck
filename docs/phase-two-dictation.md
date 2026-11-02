# Phase two dictation and clipboard native contracts

This document describes the native implementation packet. `lib.rs` is coordinator-owned and is deliberately unchanged in this PR.

## Dictation engine and model

- Engine: `whisper-rs` 0.16.0, which exposes a Rust interface to `whisper.cpp`. Windows docs and examples use the same `WhisperContext` / `FullParams` flow as this implementation: [crate API](https://docs.rs/whisper-rs/0.16.0/whisper_rs/), [Windows build notes](https://codeberg.org/tazz4843/whisper-rs/src/branch/master/BUILDING.md).
- Model: multilingual `ggml-tiny.bin`, 77.7 MB. Model bytes are downloaded only after `dictation_model_download` is invoked. Download is HTTPS-only to the pinned Hugging Face model URL, limited to 100 MiB, streamed to a `.part` file, checked against SHA-256 `be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21`, then atomically renamed under the app data directory. Failed or cancelled files are removed. The model is based on the upstream `whisper.cpp` converted OpenAI weights; the URL, size and hash are exposed by the [model file page](https://huggingface.co/ggerganov/whisper.cpp/blob/main/ggml-tiny.bin) and the upstream [model list](https://github.com/ggml-org/whisper.cpp/blob/master/models/README.md).
- Tuck keeps model data in `<appDataDir>/dictation/ggml-tiny.bin`; model removal is explicit through `dictation_model_remove`.
- OpenBot's current voice design confirms the value of explicit model selection/download, per-file SHA-256 checks, local storage and local-only processing for local recognizers. The public docs distinguish local recognition from optional cloud transcription; Tuck's implementation is local-only. See [OpenBot voice privacy](https://github.com/nightly-labs/openbot/blob/main/PRIVACY.md) and [voice runtime release notes](https://github.com/nightly-labs/openbot/blob/main/docs/RELEASING.md).

## Dictation module wiring

Coordinator setup should:

1. Add `pub mod dictation;` and `pub mod clipboard_history;` in `lib.rs`.
2. Construct `let dictation = Arc::new(dictation::DictationService::new());` and `let clipboard_history = Arc::new(clipboard_history::ClipboardHistoryService::new());`, then `.manage(dictation.clone()).manage(clipboard_history.clone())` before commands execute.
3. Install the plugin once on the Tauri builder:

   ```rust
   .plugin(tauri_plugin_global_shortcut::Builder::new()
       .with_handler(|app, shortcut, event| dictation::handle_shortcut(app, shortcut, event))
       .build())
   ```

   The plugin is Windows-supported and provides press/release events through its Rust API: [Tauri plugin docs](https://docs.rs/tauri-plugin-global-shortcut/2.4.1/tauri_plugin_global_shortcut/).
4. Register the commands listed below in `generate_handler!`.
5. In `.setup`, call `dictation::start(app.handle().clone(), &dictation)` and `clipboard_history::start(app.handle().clone(), &clipboard_history)`. Both functions return `Result<(), String>`. `dictation::start` only reads settings and registers shortcuts. It does not open the microphone, load the model or download it. `clipboard_history::start` loads opted-in local settings/history and starts a watcher that does not read clipboard contents unless `enabled` is true.
6. In the app exit path call `clipboard_history::stop(&clipboard_history)` (or set a stop event on `ClipboardHistoryService`) so the polling worker exits cleanly. With `persistOnExit: false`, no clipboard item file is written; with persistence enabled, each update is saved atomically to app data.

The dependency is `tauri-plugin-global-shortcut = "2.4"`; Cargo.lock pins the resolved crate. This module registers only its two shortcuts and does not unregister unrelated application shortcuts.

### Commands

All DTO fields use camelCase.

| Command | Parameters | Result |
| --- | --- | --- |
| `dictation_get_state` | none | `DictationSnapshot` |
| `dictation_set_settings` | `settings: DictationSettings` | updated `DictationSnapshot` |
| `dictation_model_download` | none | starts bounded explicit download; returns immediately |
| `dictation_model_download_cancel` | none | cancels current download |
| `dictation_model_remove` | none | removes installed model when idle |
| `dictation_toggle` | none | toggles a recording from an explicit command |
| `dictation_hold_start` | none | starts on explicit press command |
| `dictation_hold_stop` | none | stops on explicit release command |
| `dictation_cancel` | none | cancels capture/inference and drops audio, or cancels model download |
| `dictation_review_insert` | none | inserts a review transcript only if original foreground/edit target still matches |
| `dictation_review_discard` | none | discards the in-memory review transcript |

`DictationSettings` has `toggleShortcut`, `holdToTalkShortcut`, `reviewBeforeInsert`, and `partialPreview`. Defaults are `Ctrl+Shift+Space`, `Ctrl+Alt+Space`, immediate insertion, and no partial preview. Shortcut settings are syntax-validated, capped at 64 bytes, must differ, and are stored as bounded JSON in app config. Hotkey registration is global; it does not depend on window focus.

`DictationSnapshot` has `status`, `modelInstalled`, `modelDownloadedBytes`, `transcript`, `message`, and `settings`. Status is `idle`, `starting`, `recording`, `transcribing`, `downloadingModel`, `cancelling`, `review`, `modelMissing`, or `error`.

### Events

- `dictation_state`: `DictationSnapshot`.
- `dictation_partial`: `{ text }`, emitted only when partial preview is enabled.
- `dictation_transcript`: `{ text, inserted, requiresReview, reason }` for a completed result.
- `dictation_error`: string for worker/download/capture errors.

The app emits the final transcript once and retains it in memory only when review is required. Audio buffers are capped at 120 seconds, cleared after inference/cancel, and never written to disk. There is no cloud transcription path. The microphone is opened only from one of the explicit toggle/hold commands or their configured global shortcuts.

## Insertion safety and limitations

Tuck captures the foreground window and focused child at invocation. It only automatically types into a still-foreground standard Windows `Edit` or `RichEdit*` control, rejects the app's own process and read-only edit controls, and rechecks both foreground and focus before each bounded batch. It sends UTF-16 with Windows `SendInput` using Unicode input flags. This avoids modifying the clipboard, so clipboard contents set concurrently by the user are never restored over. It never sends Enter or otherwise submits the target.

Many applications expose browser, Electron, accessibility, terminal, password and custom editor controls through nonstandard window classes. These are deliberately unsupported for automatic insertion in this implementation and return a review reason; Windows input injection can also fail against elevated or protected applications. The current implementation does not detect every password field and does not claim universal editability. UI Automation patterns, elevated process integrity checks and richer target support require follow-up work before claiming support for those controls. Manual user action remains required to re-focus a supported edit control before review insertion.

## Clipboard history commands

Clipboard history is text-only, disabled by default, limited to 40 entries and 32 KiB per entry (about 1.25 MiB total), and stores nothing while disabled. It polls the Windows clipboard sequence every 350 ms only while enabled. Image, file and other non-text formats are not retained.

| Command | Parameters | Result |
| --- | --- | --- |
| `clipboard_get_state` | none | `ClipboardHistorySnapshot` |
| `clipboard_set_enabled` | `enabled: bool` | updated snapshot |
| `clipboard_set_persistence` | `persistOnExit: bool` | updated snapshot |
| `clipboard_delete` | `id: string` | updated snapshot |
| `clipboard_clear` | none | updated snapshot |
| `clipboard_pin` | `id: string`, `pinned: bool` | updated snapshot |

`ClipboardHistoryItem` contains `id`, `text`, `createdAtMs`, and `pinned`. The snapshot includes `enabled`, `persistOnExit`, and `items`. `clipboard_history_changed` carries a snapshot. `clipboard_history_error` carries a string.

When persistence is off, item text exists only in process memory and is not written to disk. When enabled, strict schema-versioned JSON is stored under `<appDataDir>/clipboard-history/`, with bounded input, item count and byte size. Clear removes the local item file. Pinning only affects eviction order. A malformed, oversized or unknown-schema file is ignored.

The watcher excludes clipboard entries marked with Windows `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory=0`, or the `PasswordProtected` format, and skips the `TuckDictationTransient` format. Automatic text insertion does not use the clipboard at all. These Windows markers are cooperative signals: password managers and applications do not all mark their formats, so Tuck cannot detect every sensitive value. Users should not treat this history as a complete password exclusion system.

No additional Tauri capabilities are required by these native Rust commands; the default capability set already supplies core event/invoke access. No clipboard-manager plugin is used.

## Evidence and remaining live checks

The native unit tests cover shortcut/settings validation, resampling, explicit defaults, private-store bounds and history eviction. Compile and run them on Windows after the coordinator registers modules. Live microphone capture, model download/integrity, actual transcription on representative hardware, actual Windows foreground-control input, elevated-target rejection behavior and Windows clipboard marker behavior still need a real Windows run. Development never downloaded the model or activated the microphone.
