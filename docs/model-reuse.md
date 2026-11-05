# Reuse and automatic restoration

A Download request first performs fresh bounded discovery in the existing known model folders on the model worker. Tuck verifies catalog size and SHA-256 using its file-identity cache before reusing an existing copy. No copy, hard link, deletion, or modification of another application's model is performed. A valid managed copy also skips HTTP. Only pinned compatible catalog artifacts qualify; Handy ONNX models remain unsupported.

A missing saved external selection retains its original model ID and path and cannot silently fall back to Tiny. Startup, a settings scan, or the next dictation attempt requests restoration: rediscover the same artifact first, otherwise use the existing pinned official downloader with its ownership receipt, byte bounds, cancellation, stall timeout and progress. Existing downloading-model operation flags cover preflight as well as HTTP. Restoration does not open the microphone or discard review text.

There is one automatic attempt per selection generation per process. Failure or Cancel suppresses subsequent automatic attempts for that selection during this launch. Download is the explicit retry; restarting permits one new automatic attempt. First launch without an external selection never downloads automatically. Stop using and new selections cancel pending acquisition and generation checks prevent stale activation. Shutdown cancels acquisition; tracked partial downloads are swept by the existing startup cleanup.

Validation: real native library tests and clippy with scripts/prepare-native.ps1; Bun tests, frontend build and UI smoke. The ignored read-only detection probe verified the installed Hex Parakeet and podcli Base without changing preferences or models. No network download test was run. Temporary fixture tests exercise production preflight and lazy network orchestration, invalid bytes, cancellation, missing-copy network dispatch, and stale activation after Stop using/new choice.

Remaining validation limitation: no end-to-end native service fixture currently proves external move rediscovery plus singleflight/failure suppression through settings persistence and fake HTTP together. Those paths have implementation coverage and focused component tests, but require independent review before this draft is made ready.
## Local evidence

- Native library: 264 passed, 17 ignored; clippy with warnings denied passed
- Bun: 152 passed, 0 failed; production frontend build and UI smoke passed
- Read-only installed-model probe: 1 passed; Hex Parakeet and podcli Base both verified
- Model files and user preferences were not modified; no model HTTP download was exercised
- Logs and smoke artifacts are in ignored artifacts/reuse-* paths
