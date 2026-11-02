# Live island phase two

User approved development after the grill-me interview. Scope: Windows app only. Preserve existing working v1 features; do not add water reminders.

## Approved experience

Music is the default while playing. Alerts temporarily interrupt and then restore the prior context. Explicitly selected stats or other tools must not be stolen by an ordinary track change. Compact lyrics bar expands on hover to a split player/lyrics layout, and collapses on pointer exit. Settings control delays, size, title-versus-lyrics compact content and pin-open behavior. Match supplied screenshots: pure black upper panel with a muted artwork-derived color rising from the bottom, white foreground and faded adjacent lyrics.

Native media: actual session metadata/artwork, playback/seek, supported-command availability and player selection. Lyrics follow Nusa: LRCLIB lookup by title/artist/duration, timed highlighting and click-to-seek. Online lookup is explicitly enabled through settings, with loading/offline/missing/instrumental states and stale-request rejection.

Stats are on demand: real total CPU, used/total RAM, named GPU and short histories. Missing telemetry is unavailable, never fabricated zero. Multi-adapter support and rate-counter warm-up are explicit.

Visualizer analyzes actual system playback audio, saved nowhere, and uses artwork color. Settle when silent/stopped; support reduced motion and explicit capture enablement. Never substitute random bars for measured audio.

Dictation is offline by default after an explicit model download. Settings offer toggle and hold-to-talk hotkeys; recording never starts merely by focusing an input. Preserve the target input while the island displays recording/transcribing/transcript state. Insert finished text immediately by default without submitting; optional review before insert. Support cancel, model/microphone failures, clipboard preservation and honest unsupported/elevated targets. No audio retention by default. Study Handy's Windows paste transaction and OpenBot's local transcription design; do not copy incompatible code or unreviewed executables.

Clipboard history clears on exit by default, with optional persistence and pinned items. Bound item count/size, allow delete/clear/pin, avoid retaining sensitive/marked-private formats and Tuck's transient dictation paste data. Start opt-in.

Also include to-do/reminders and system volume/brightness/key/device peeks, using real supported Windows APIs and showing unavailable states. Keep existing timers and file shelf. No water reminder.

## Ownership and PR packets

1. Frontend Luna medium: frontend files only, UI/settings/contracts/tests. No Rust or Cargo edits.
2. Media Gemini high: media.rs plus new lyrics.rs, telemetry.rs and visualizer.rs and their documentation/tests. No lib.rs, Cargo, system.rs or frontend edits.
3. Dictation/clipboard Luna max: new dictation.rs and clipboard_history.rs, Cargo manifest/lock, necessary capability additions and own docs/tests. No frontend, lib.rs, media/system/extensions/usage edits.

Coordinator owns native module registration/integration, window geometry, reminders/HUD integration and final verification. Workers return explicit commands, DTOs/events and dependency/features wiring. Each works in an isolated checkout and creates a feature PR without merging. No worker spawns children. Fresh Sol low reviewers inspect final heads before merge. Keep at most three implementation workers active.

## Acceptance

- Frontend build, behavior tests, native cargo check/test/clippy/fmt pass on the integrated tree.
- Test state arbitration, lyric parser/timing and obsolete-request cancellation, unsupported telemetry, dictation lifecycle/cancel/target safety, clipboard bounds/privacy and persistence.
- Native Spotify plus a browser player, seek/controls/player exit and lyrics timing require live Windows verification. CPU/RAM/GPU compare against Task Manager with documented aggregation/interval differences.
- Verify click-through/focus, DPI, keyboard and reduced motion. Do not claim a browser mock proves native behavior.
- Offline transcription must actually transcribe with a verified model when available; report download/hardware unknowns rather than shipping a fake success path.
- PR coverage includes all scope, with blockers fixed and a final feature matrix that distinguishes implementation, automated evidence and manual unknowns.
