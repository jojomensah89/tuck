# Phase two dictation and clipboard native contracts

This packet implements the native dictation and clipboard-history services. It preserves the existing v1 flows. The coordinator-owned `src-tauri/src/lib.rs` already registers both services, commands, global shortcuts, startup and shutdown; this patch leaves `lib.rs` and capabilities unchanged.

## Offline dictation

- Engine: `whisper-rs` 0.16.0 over `whisper.cpp`, using its CPU inference path with at most four threads. It does not require a CUDA or DirectML runtime. Current Windows build guidance and API are in the [whisper-rs crate docs](https://docs.rs/whisper-rs/0.16.0/whisper_rs/) and [Windows build notes](https://codeberg.org/tazz4843/whisper-rs/src/branch/master/BUILDING.md).
- Model: multilingual `ggml-tiny.bin`, about 77.7 MB. It is fetched only after `dictation_model_download` is invoked. The native downloader accepts no caller URL or path, requires HTTPS, streams at most 100 MiB to a temporary file, verifies SHA-256 `be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21`, then installs it under `<appDataDir>/dictation/`. Cancellation removes the temporary file. The pinned URL, size and digest correspond to the [Hugging Face model file](https://huggingface.co/ggerganov/whisper.cpp/blob/main/ggml-tiny.bin) and upstream [model list](https://github.com/ggml-org/whisper.cpp/blob/master/models/README.md).
- Each dictation checks the installed model size and digest before opening the microphone. Startup only reads bounded settings and registers shortcuts; it does not open the microphone, load the inference model or download model bytes.
- Audio is captured only after the explicit toggle or hold-to-talk command/shortcut. Capture is capped at 120 seconds, callback blocks at one second and queued audio at ten seconds. Overflow stops capture with an error. Audio buffers are zeroized after use/cancellation and are never written to disk. Whisper's abort callback receives cancellation during inference.
- Partial preview is optional and enabled by default. It re-runs local transcription on up to the latest 15 seconds every three seconds; it is not token-by-token streaming. No audio or transcript is sent to a cloud service.
- Model removal is explicit through `dictation_model_remove` and is rejected while recording, reviewing, inserting, downloading or running inference.

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
| `dictation_model_download` | none | starts explicit bounded download; returns immediately |
| `dictation_model_download_cancel` | none | cancels active model download |
| `dictation_model_remove` | none | removes installed model when idle |
| `dictation_toggle` | none | toggles recording from an explicit command |
| `dictation_hold_start` | none | starts recording from an explicit press command |
| `dictation_hold_stop` | none | stops hold-to-talk on release |
| `dictation_cancel` | none | cancels recording/inference/download, or discards a review transcript |
| `dictation_review_insert` | none | inserts the review transcript only if the original target still matches |
| `dictation_review_discard` | none | discards the in-memory review transcript |

`DictationSettings` contains `toggleShortcut`, `holdToTalkShortcut`, `reviewBeforeInsert` and `partialPreview`. Defaults are `Ctrl+Shift+Space`, `Ctrl+Alt+Space`, immediate insertion and partial preview. Shortcuts are validated, capped at 64 bytes, must differ, and are saved as bounded JSON in the app config directory.

`DictationSnapshot` contains `status`, `modelInstalled`, `modelDownloadedBytes`, `transcript`, `message`, `settings` and `canInsert`. Status values are `idle`, `starting`, `recording`, `transcribing`, `downloadingModel`, `removingModel`, `cancelling`, `review`, `modelMissing` and `error`.

### Dictation events

- `dictation_state`: `DictationSnapshot`.
- `dictation_partial`: `{ text }`, only when partial preview is enabled.
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
