# Reuse and automatic restoration

A Download request first performs fresh bounded discovery in the existing known model folders on the model worker. Tuck verifies catalog size and SHA-256 using its file-identity cache before reusing an existing copy. No copy, hard link, deletion, or modification of another application's model is performed. A valid managed copy also skips HTTP. Only pinned compatible catalog artifacts qualify; Handy ONNX models remain unsupported.

A missing saved external selection retains its original model ID and path and cannot silently fall back to Tiny. Startup, a settings scan, or the next dictation attempt requests restoration: rediscover the same artifact first, otherwise use the existing pinned official downloader with its ownership receipt, byte bounds, cancellation, stall timeout and progress. Existing downloading-model operation flags cover preflight as well as HTTP. Restoration does not open the microphone or discard review text.

There is one automatic attempt per selection generation per process. Failure or Cancel suppresses subsequent automatic attempts for that selection during this launch. Download is the explicit retry; restarting permits one new automatic attempt. First launch without an external selection never downloads automatically. Stop using and new selections cancel pending acquisition and generation checks prevent stale activation. Shutdown cancels acquisition; tracked partial downloads are swept by the existing startup cleanup.

## Service integration evidence

Eight `service_worker_*` native tests execute `start_model_download` (directly or through repair), a real spawned worker, catalog size/SHA-256 verification, selection guards, real atomic temporary-file settings persistence, and worker cleanup. A three-byte fixture catalog and synchronized acquisition callback bound filesystem/network work; no native speech library is stubbed. Production passes the pinned catalog through the same discovery and verification implementation. The callback substitutes acquisition transport, not persistence or worker orchestration.

- Moved external copy: same catalog ID rediscovered, verified and persisted at its new path; source bytes unchanged and zero acquisition callbacks.
- Deleted external copy: repair, explicit start and settings scan overlap with one acquisition callback. A real scan is held immediately before verification while acquisition commits; the saved external path clears and the managed model resolves active.
- Atomic settings replacement failure: acquisition finishes, old in-memory selection remains, model error survives cleanup, and no Using/ready success notice appears. The test forces failure with a directory at the settings destination, so it cannot also retain the old on-disk file at that destination.
- Selection guards: a committed new choice during acquisition and a pending same-generation user verification prevent stale activation. Old-worker cleanup preserves the newer choice's model error.
- Cancel and acquisition failure: subsequent automatic repair stays suppressed, explicit Download retries successfully. A fresh service loaded from persisted settings gets a new automatic attempt.
- Manual reuse B while A is selected: A stays active and persisted, notice says Verified existing, source bytes remain unchanged, zero acquisition callbacks.
- Equal-size hash mismatch and unsupported filename/format do not reuse; acquisition runs instead. No name/size-only trust is used for activation.
- Existing ownership receipt and borrowed-file removal guard component tests remain unchanged and pass. The bounded callback writes no ownership receipt; receipt generation remains covered at the storage component level.

Coordinator fixes separate pending user selection verification from background scan checks, propagate commit outcomes and persistence failures, and distinguish Using from Verified. Worker cleanup also avoids overwriting a newer selection's error/notice.

## Limits and manual runtime gaps

The fixture does not exercise an actual HTTP socket, official multi-hundred-MB transfer, full Tauri startup, microphone, or native transcription. It uses the real acquisition dispatch with a bounded callback. Command-level explicit Use/Stop using/new-choice races during **preflight** remain unproven by this fixture: the service tests exercise commit-generation changes during acquisition and the same-generation pending-user guard, while focused component tests cover cancellation and stale commits. Cancellation of a borrowed file during preflight is not independently service-tested. This is substantive service coverage, not complete coverage of every requested race.

Only pinned compatible catalog artifacts in known Hex/Handy/Buzz/OpenWhispr/podcli/Hugging Face model locations qualify. Other applications' settings/credentials are never discovery inputs. Handy ONNX and noncatalog artifacts remain unsupported. No automatic first-run/default download or different-model autochoice is introduced.

## Local evidence

- Restoration cleanup uses the generation returned by its successful activation, captured under the selection lock. A later user choice still invalidates cleanup ownership. Existing service fixtures assert both reused-copy and downloaded-replacement completion notices, alongside stale-selection guards.
- Native `cargo test --lib --locked`: 273 passed, 17 ignored (eight new service tests)
- `cargo clippy --lib --locked -- -D warnings` and `cargo fmt -- --check`: passed using scripts/prepare-native.ps1 and the existing native cache
- Bun: 152 passed, 0 failed; production frontend build passed
- UI contracts unchanged; no new UI smoke run or production app launch
- Actual model files and user preferences were not changed; no real HTTP model download was performed in this fixture run

## Main integration verification (2026-11-07)

Merged main fd15d8d into the branch and retargeted PR #35 to main. Resolved the snapshot visibility conflict while retaining automatic restoration. Acquisition preflight hashes current bytes even when Windows metadata timestamps coalesce for a same-size rewrite; recording still uses the verification cache. Existing files remain in place, and discovery only opens catalog model files from known model locations.

Required checks: Bun 225 passed; build and UI smoke passed; cargo fmt and all-target Clippy passed; Rust tests 248 library tests plus 16 integration tests passed (17 ignored live diagnostics). All final commands exited 0. An initial all-target Clippy run exited 101 for a test-only callback type; extracting a type alias resolved it. Native setup used scripts/prepare-native.ps1 -InstallBuildTools. The Git release-planning integration tests now use the same 60-second allowance as other release tests.

UI smoke asserts the unselected-model guidance and saves docs/screens/model-reuse-settings.png. This is a browser fixture, not evidence of native model reuse. Live Tauri, real model downloads and transcription remain unverified. The installed Tuck was not stopped.
