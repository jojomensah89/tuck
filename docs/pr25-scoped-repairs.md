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

No real microphone, key injection, model-dependent ignored tests, PR merge or release actions were performed for these repairs. No performance improvement is claimed. Full-context final autodetection can cost more than reusing the initial guess. Real-model multilingual, short/silent clip and preview-to-final behavior remain manual verification requirements; the no-model harness does not prove Whisper buffer/runtime correctness.

PR27 independently changes full versioned snapshots and collapsed previews. Integrate its native/UI event contracts and provisional-text ownership deliberately later; expect overlapping edits in live-ui.ts, live-services.ts, main.ts and native dictation code. This branch adds no native event versioning and does not merge or cherry-pick PR27. Cross-session stale event protection remains part of that integration.

GGML_NATIVE/build-release CPU baseline remains a separate release blocker, unchanged here. Do not treat debug compilation or unit tests as a portable release certification.

Automated evidence for the final head is recorded in the PR description. The UI smoke harness uses synthetic native events, not real input. One initial smoke attempt failed at the pre-existing telemetry timing assertion before reaching dictation; the rerun passed.

## Astra follow-up repairs

- Cancellation is carried through target preparation, the modifier-release wait and every Unicode batch. Cancel and batch submission share an insertion mutex. Once cancellation is committed, no new SendInput batch can be submitted. Already-submitted OS input cannot be retracted; cancellation after a prefix has been submitted can leave that prefix in the target. Review insertion uses the same cancellation control.
- Download ownership is represented by durable receipts containing Windows file identity and, for completed files, size and change/modified times. Deletion checks identity while holding a handle that denies replacement. Unknown/replaced files and symlinks are preserved. Missing receipts mean Stop using, never Remove. Stop using is persisted so fallback cannot immediately select the same file again.
- Legacy migration is deliberately limited to ggml-tiny.bin in Tuck's model folder, with the catalog size and SHA-256 digest. Only that exact legacy install can gain an ownership receipt without a new download. Other existing files are unowned.
- Temporary downloads use exclusively created UUID filenames and tracked identity receipts. The sweep only considers tracked temporary files; arbitrary and old catalog-name .part files, including detect-only entries, are preserved. Download installation uses a no-overwrite hard link while the source handle denies replacement, then releases the tracked temporary link. Filesystems unable to establish identity or hard links fail safely rather than overwrite/delete user files.
- Model selection verification carries a generation token. Commit rechecks it and active-session state under the runtime lock; starting a session and selection commits share the settings-operation lock so the captured model cannot disagree with the committed selection. Removal invalidates outstanding verification.
- Network destinations are rejected after local link resolution and before model metadata/hash reads; direct file links to UNC targets are refused before resolution. Local Hugging Face file links remain supported. The Windows symlink test reports unavailable privileges if Developer Mode/symlink privilege is absent; that environment still needs a manual link check.
- Thread count is documented as half-logical-processor heuristic, not physical-core detection.
- origin/main was merged because GitHub reported CONFLICTING/DIRTY and no pull_request workflow existed for 37e20ec. The compact-candidate conflict retains main's shared helper and busy dictation phases. PR27 was not imported.

The accelerated-audio harness queue was widened from a roughly 5 ms scheduling allowance to 100 ms. A 400 ms blocking-preview regression still exceeds that queue. The production queue and capture/engine architecture are unchanged. An initial post-merge UI attempt timed out at the existing HUD return-to-open check; subsequent runs are recorded in the PR. Processing and cancelling screenshots use synthetic native events and do not prove microphone/runtime behavior.
