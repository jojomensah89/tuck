> Historical plan or verification record. This is not the current feature list or setup guide. Some described features are being retired. Start with the [README](../README.md).

# Phase two dictation and clipboard native contracts

This packet implements the native dictation and clipboard-history services. It preserves the existing v1 flows. The coordinator-owned `src-tauri/src/lib.rs` already registers both services, commands, global shortcuts, startup and shutdown; this patch leaves `lib.rs` and capabilities unchanged.

## Offline dictation

- Engine: `whisper-rs` 0.16.0 over `whisper.cpp`, using its CPU inference path with at most four threads. It does not require a CUDA or DirectML runtime. Current Windows build guidance and API are in the [whisper-rs crate docs](https://docs.rs/whisper-rs/0.16.0/whisper_rs/) and [Windows build notes](https://codeberg.org/tazz4843/whisper-rs/src/branch/master/BUILDING.md).
- Model: chosen in Settings from the compile-time catalog in `dictation/model.rs` (Tiny, Base, Small, Medium, Large v3 Turbo; files pinned to one Hugging Face commit). Files are fetched only after `dictation_model_download` is invoked. The downloader accepts a catalog id, never a URL or path, requires HTTPS, limits redirects, enforces that entry's exact size, verifies its SHA-256 before install, and fails after 60 s without data rather than after a fixed total time. A file is hashed once; the result is kept in `dictation/verified-models.json`, keyed by path, size, modified time, NTFS file ID and change time. Other apps' files are hashed again once per launch. The cache is plain JSON, so anyone able to edit it can also replace a model in the same folder: it guards against accidents and other apps, not against a process running as the owner. Anything left as `<id>.part` is deleted at start. The microphone opens first and the model loads beside it, so words spoken during loading are kept; the selected model is retained for two idle minutes and preloaded while the Dictation tab is open. Audio is moved only by the session thread and every transcription runs on a separate engine thread, so inference cannot overflow the capture queue.
- Detection runs only when the picker is on screen or on "Look for models". It lists known folders of other apps (section 3.6 of `docs/research/resources-lyrics-models.md`) by name and size, then hashes matches once on a low-priority thread. Only exact catalog matches are offered. They are used in place and read-only; "Stop using" never touches the file. No other file of those apps is opened.
- Each dictation checks the installed model size and digest before opening the microphone. Startup registers shortcuts and keeps the microphone ready once a speech model is available, unless Keep microphone ready is off. It does not load the inference model or download model bytes.
- Recording shows state and amplitude only. State snapshots retain session IDs and monotonically increasing revisions; legacy partial text events are ignored.
- Audio is captured only after the explicit toggle or hold-to-talk command/shortcut. Capture is capped at 120 seconds, callback blocks at one second and queued audio at ten seconds. Overflow stops capture with an error. Audio buffers are zeroized after use/cancellation and are never written to disk. Whisper's abort callback receives cancellation during inference.
- Holding both Shift keys starts dictation after 200 ms, so rolling across them while typing does nothing. Windows has its own use for that key: holding right Shift for 8 seconds turns Filter Keys on or off if that shortcut is enabled (https://support.microsoft.com/en-us/accessibility/windows/windows-keyboard-shortcuts-for-accessibility), and its prompt can take focus and stop insertion. It is switched off under Settings > Accessibility > Keyboard > Filter keys.
- Model removal is explicit through `dictation_model_remove` and is rejected while recording, reviewing, inserting, running inference or downloading that model. It deletes only files in Tuck's own folder.

The OpenBot voice design likewise documents explicit local-model lifecycle and local recognition as distinct from optional cloud transcription. Tuck uses only local recognition; see [OpenBot privacy](https://github.com/nightly-labs/openbot/blob/main/PRIVACY.md) and [release notes](https://github.com/nightly-labs/openbot/blob/main/docs/RELEASING.md).

## Existing coordinator wiring

The latest `main` baseline already contains the wiring below, so this feature patch does not edit `lib.rs`:

1. `lib.rs` declares `pub mod dictation;` and `pub mod clipboard_history;`, constructs and manages `Arc<DictationService>` and `Arc<ClipboardHistoryService>`.
2. The Tauri builder installs `tauri_plugin_global_shortcut` once and calls `.with_handler(dictation::handle_shortcut)`.
3. `.setup` calls `dictation::start(handle.clone(), &dictation_setup)` and `clipboard_history::start(handle.clone(), &clipboard_setup)`.
4. `generate_handler!` registers every command in the tables below.
5. On `RunEvent::Exit`, `dictation.shutdown()` signals cancellation and `clipboard_history::stop(...)` stops its polling worker.

The global-shortcut dependency is `tauri-plugin-global-shortcut = "2.4"`. Existing native dependencies include `whisper-rs`, `cpal`, `crossbeam-channel`, `sha2` and `zeroize`. This patch enables Reqwest streaming and adds direct Tokio `macros`, `net`, `rt` and `time` support for cancellable model downloads, plus Windows `Storage_FileSystem` and `UI_Input` features for atomic file replacement and Unicode input.

### Dictation commands and DTOs

All serialized DTO fields use camelCase.

| Command | Parameters | Result |
| --- | --- | --- |
| `dictation_get_state` | none | `DictationSnapshot` |
| `dictation_set_settings` | `settings: DictationSettings` | updated snapshot |
| `dictation_model_download` | `{ id }` | starts explicit bounded download of a catalog model; returns immediately; never replaces the active model unless there is none |
| `dictation_model_download_cancel` | none | cancels active model download |
| `dictation_model_remove` | `{ id }` | deletes a model Tuck downloaded, or stops using another app's file |
| `dictation_model_use` | `{ id }` | verifies off the UI thread, then selects a catalog model |
| `dictation_models_scan` | none | looks for catalog models in other apps' folders, on demand |
| `dictation_shortcuts_pause` | `{ paused }` | suspends Tuck's shortcuts while the key recorder listens; ends by itself after 30 s |
| `dictation_shortcuts_retry` | none | registers shortcuts again, for one another app was holding |
| `dictation_toggle` | none | toggles recording from an explicit command |
| `dictation_hold_start` | none | starts recording from an explicit press command |
| `dictation_hold_stop` | none | stops hold-to-talk on release |
| `dictation_cancel` | none | cancels recording/inference, or discards a review transcript |
| `dictation_review_insert` | none | inserts into the currently focused safe editable control, otherwise copies |
| `dictation_review_discard` | none | discards the in-memory review transcript |

`DictationSettings` contains `toggleShortcut`, `holdToTalkShortcut`, `reviewBeforeInsert`, `settingsVersion`, and optional `activeModel` and `activeModelPath`, which only the model commands can change. A file without them uses `ggml-tiny.bin` if installed. A saved field of the wrong type or a shortcut that cannot be used resets alone, with a notice, and keeps the other values. A file that is not JSON is kept as `dictation.json.bad`. A shortcut that an app holds is reported beside its field and blocks only a save that changes that shortcut. Defaults are `Ctrl+Shift+Space`, `Shift+Shift` (holding Left + Right Shift simultaneously), immediate insertion and partial preview. Shortcuts are validated, capped at 64 bytes, must differ, and are saved as bounded JSON in the app config directory.

`DictationSnapshot` contains `status`, `modelInstalled`, `modelDownloadedBytes`, `transcript`, `message`, `settings`, `canInsert`, `models`, `activeModelId`, `activeModelLabel`, `modelNotice`, `scanningModels`, `shortcutProblems` and `settingsNotice`. Status values are `idle`, `starting`, `recording`, `transcribing`, `cancelling`, `inserting`, `review`, `modelMissing` and `error`. A download no longer changes `status`, so the model in use keeps working during it.

### Dictation events

- `dictation_state`: `DictationSnapshot`.
- `dictation_transcript`: `{ text, inserted, requiresReview, reason }` once per completed result.
- `dictation_error`: error string for worker, download or capture failures.

The native service retains transcript text only while review is pending. Insertion uses Unicode keyboard input and does not modify the clipboard or submit the target. It captures the original foreground window and focused control, rechecks focus and identity before each bounded input batch, and aborts if the target changes. It accepts standard Windows `Edit`/`RichEdit*` controls and UI Automation Edit controls with a writable Value pattern. It rejects its own process, read-only/password controls and unsupported targets. A partial SendInput failure disables automatic retry and removes the review insertion target because some text may already have been typed.

Target insertion can still fail for elevated/protected apps or controls that do not expose reliable focus and edit information. Review insertion requires the captured target to remain the same; no focus is taken from the notch. The implementation does not claim universal editability or universal detection of password fields.

## Clipboard history

Clipboard history is text-only, disabled by default and keeps at most 40 entries of 32 KiB each. The watcher polls every 350 ms only while enabled; it does not retain images, files or other formats. With `persistOnExit: false` (the default), entries remain only in process memory and the item file is removed on startup when there is no valid persistence opt-in. With persistence explicitly enabled, bounded schema-versioned plaintext JSON is stored under `<appDataDir>/clipboard-history/`; item file reads are capped at 2 MiB. Clear removes the saved item file.

| Command | Parameters | Result |
| --- | --- | --- |
| `clipboard_get_state` | none | `ClipboardHistorySnapshot` |
| `clipboard_configure` | `enabled: bool`, `persistOnExit: bool` | atomically updates both preferences and returns snapshot |
| `clipboard_set_enabled` | `enabled: bool` | updated snapshot |
| `clipboard_set_persistence` | `persistOnExit: bool` | updated snapshot |
| `clipboard_delete` | `id: string` | updated snapshot |
| `clipboard_clear` | none | updated snapshot |
| `clipboard_pin` | `id: string`, `pinned: bool` | updated snapshot |
| `clipboard_copy` | `id: string` | copies that item to the Windows clipboard on explicit request |

`ClipboardHistoryItem` contains `id`, `text`, `createdAtMs` and `pinned`. `ClipboardHistorySnapshot` contains `enabled`, `persistOnExit` and `items`. `clipboard_history_changed` carries a snapshot; `clipboard_history_error` carries an error string.

The watcher skips Windows clipboard entries marked `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory=0`, `PasswordProtected`, or Tuck's `TuckDictationTransient` format. It also skips its own clipboard writes. These markers are cooperative: some password managers do not mark their clipboard contents, so password exclusion cannot be guaranteed. Automatic dictation insertion never touches the clipboard; only the explicit `clipboard_copy` command does.

Clipboard reads happen without holding the history state mutex. The watcher compares clipboard sequence numbers around each read and rejects stale snapshots; a generation check also rejects reads captured before a disable/re-enable transition. File updates use unique temporary files and atomic replacement. No extra Tauri capability or clipboard plugin is needed.

## Test coverage and live limits

Native tests cover shortcut/settings validation, UTF-8 truncation, atomic settings replacement, model digest/partial-file cleanup, bounded and cancellable audio reservations, resampling, insertion partial-failure handling, clipboard privacy markers, stale-read races, schema validation and bounded storage reads. Build and runtime evidence for the current Windows toolchain is recorded with the PR. Development did not download the model, activate the microphone, or exercise real microphone capture, transcription performance, Windows clipboard contents, or target insertion on elevated applications.

Current delivery and measurement evidence: see `dictation-like-hex.md`. Recording has no partial text payload or preview setting.