# PR25 scoped repair notes

## Behavior

- Final inference releases the narrowed preview WhisperState before creating a fresh state. Multilingual final inference chooses its own language; an opening preview guess (including a confident guess on a silent/short clip) never forces the final language. English-only models use English and skip language detection.
- Queued previews check stop/cancel before the uninterruptible preparation pass. Transcription checks abort before entering Whisper. A pass already loading or detecting language still must unwind.
- Recording status only follows starting. Cancellation cannot be overwritten by recording, transcribing or capture-failure processing. New recording requests during cancellation return an explanatory wait error.
- Capture failure closes the stream and reports neutral processing before joining the engine. The microphone is no longer implied open during engine unwind.
- Preview budget exhaustion emits `previewsPaused` through dictation_state snapshots. The island says “Live transcript paused for this recording.” New sessions reset the flag. Late partials after pause are ignored.
- The island retains its last provisional text during transcription, labels Transcribing/Cancelling explicitly and keeps activity busy through startup, transcription, processing and cancellation.
- The session audio vector reserves its maximum bounded capacity up front to avoid reallocating recorded samples into unzeroized old allocations.

## Boundaries and remaining work

No real microphone, key injection, model-dependent ignored tests, merge or release actions were performed for these repairs. No performance improvement is claimed. Full-context final autodetection can cost more than reusing the initial guess. Real-model multilingual, short/silent clip and preview-to-final behavior remain manual verification requirements; the no-model harness does not prove Whisper buffer/runtime correctness.

PR27 independently changes full versioned snapshots and collapsed previews. Integrate its native/UI event contracts and provisional-text ownership deliberately later; expect overlapping edits in live-ui.ts, live-services.ts, main.ts and native dictation code. This branch adds no native event versioning and does not merge or cherry-pick PR27. Cross-session stale event protection remains part of that integration.

GGML_NATIVE/build-release CPU baseline remains a separate release blocker, unchanged here. Do not treat debug compilation or unit tests as a portable release certification.

Automated evidence for the final head is recorded in the PR description. The UI smoke harness uses synthetic native events, not real input. One initial smoke attempt failed at the pre-existing telemetry timing assertion before reaching dictation; the rerun passed.
