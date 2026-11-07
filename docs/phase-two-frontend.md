> Historical plan or verification record. This is not the current feature list or setup guide. Some described features are being retired. Start with the [README](../README.md).

# Phase two frontend integration

Frontend was implemented by the coordinator after both Luna frontend attempts stopped without edits. Native service wiring and adapters are now integrated in draft PR 6; final review and live-verification gates are tracked in phase-two-verification.md.

## Delivered

Artwork-derived muted bottom-up wash, hover expand/collapse with configurable delays and pin mode, split timed lyrics with click-to-seek, media-session picker, CPU/RAM/GPU sample histories, measured-spectrum rendering, dictation recording/transcribing/review presentation, bounded clipboard list with pin/delete/copy, and persistent local tasks/reminders while Tuck runs. Existing pages, timers, shelf and AI usage remain.

Interaction preferences are local and bounded to a 500x210 maximum within the existing 520x230 canvas. Float top offset and native hit reporting remain. Keyboard Escape closes and restores native click-through. No random spectrum, synthetic telemetry or fabricated transcript is used by the new adapters. All system services reject browser-only actions. Browser demo is labeled by the existing shell.

## Adapter contract to align during integration

See src/live-services.ts for exact TypeScript interfaces. These are frontend adapter names. src/live-services.ts translates them to the actual native commands and DTOs on the integration branch; see the native module documents for wire signatures.

| Service | Commands | Event |
| --- | --- | --- |
| Players | media_sessions -> MediaSession[], media_select_session({id}) | existing media |
| Lyrics | lyrics_settings -> {enabled}, lyrics_set_enabled({enabled}), lyrics_lookup({trackId,title,artist,durationMs}) -> LyricsSnapshot | lyrics |
| Telemetry | telemetry_snapshot -> TelemetrySnapshot | telemetry |
| Audio spectrum | visualizer_settings -> {enabled}, visualizer_set_enabled({enabled}) | visualizer: {bands:number[] in 0..1,active?:boolean} |
| Dictation | dictation_snapshot, dictation_settings, dictation_set_settings({settings:{mode,shortcut,reviewBeforeInsert}}), dictation_download_model, dictation_toggle, dictation_stop, dictation_cancel, dictation_insert | dictation |
| Clipboard | clipboard_snapshot, clipboard_settings, clipboard_set_settings({settings:{enabled,persist}}), clipboard_copy({id}), clipboard_pin({id,pinned}), clipboard_remove({id}), clipboard_clear | clipboard-history |

Snapshots use milliseconds since Unix epoch for sampledAt/createdAt. Lyrics use LRC text; media identity must be echoed to reject stale responses. Partial native events must not erase the current track's artwork. Sensitive native settings must enforce appropriate caller labels. Services are disabled in settings until their initial settings query succeeds. No mutation reports success after a rejected command.

Tasks use bounded local storage, checked before announcing due reminders. Reminders do not run while Tuck is closed; the UI says so. Completion/deletion persists only when writes succeed. External text and transcripts enter DOM via textContent, never HTML.

## Verification

2026-11-02: bun run build passed; bun test passed 27 tests; bun run test:ui passed incumbent and phase-two browser/native-bridge fixture checks. UI smoke covers 500x210 geometry, hover expansion, keyboard seek, existing extension focus/drag behavior, lyrics timing and seek command, tint extraction, stats display, preserving selected stats during track changes, transcript/clipboard markup safety and clipboard copy routing. Impeccable detector returned no findings. These are presentation/contract checks, not native hardware evidence.

Screenshot fixture: artifacts/island-phase-two-media.png. The album cover, track and lyrics in this screenshot are explicitly synthetic test data. Final native command/DTO alignment, Spotify/browser verification, audio capture, dictation/model/insertion and clipboard privacy need integration tests and independent Sol low review before merge. Volume/brightness/key/device overlays are coordinator-native work still outstanding.
