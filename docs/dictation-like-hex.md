# Final-only dictation verification

2026-11-08. Base: origin/main d431249. Synthetic fixtures, Windows debug CPU build.

## Interaction and implementation

Recording shows state and an amplitude meter. Removed the live-transcript preference, snapshot fields/event, rolling Whisper jobs, budgets, language preparation and partial publishers, including Parakeet text publication. Version ordering, bounded capture/draining, CPU instruction guard, cancellation, buffered Parakeet processing, reliable-stream acceptance and one full-clip recovery pass remain.

Whisper retains weights but creates fresh per-utterance decoding state. Parakeet retains its session and fresh cancellation-token handling. The selected engine is cached after use; the open Dictation tab requests preloading/refresh every 30 seconds. Recording can open capture while a preload is running. A single weak-reference reaper checks every 10 seconds and unloads after 120 idle seconds (up to 10 seconds reaper scheduling slack). Model removal clears the cache before touching the file; a different selection replaces weights before loading its engine. The microphone stays ready by default once a model is available; Settings can restrict capture to dictation sessions. No startup preload.

Automatic insertion retains the original-target/focus-change rules. Review checks end focus to choose review versus copy, then Insert captures the field focused at that later moment. No safe editable field means clipboard fallback, whether review is enabled or disabled. Partial/ambiguous SendInput delivery disables automatic retry and copy. Clipboard errors retain review text and show the error. Default review remains false; legacy true persists and obsolete partialPreview is ignored.

Tuck-process exclusion, password checks, modifier release, per-batch focus/editability checks, cancellation and the insertion gate are unchanged. Static check: system.rs applies WS_EX_NOACTIVATE; only explicit keyboard/tray activation removes it, and review Insert calls no window activation API. A real mouse-click focus test against another Windows app has not been performed.

Ideas only, no Hex code copied: [Hex dictation map](https://github.com/anomalyco/hex/blob/ebba1c25772b0c7d55d4738874f29462c81dd173/docs/features/dictation.md), [engine ownership and idle unload](https://github.com/anomalyco/hex/blob/ebba1c25772b0c7d55d4738874f29462c81dd173/src/transcription/engine.rs). Hex's optional features are not Tuck requirements.

## Timing: measured, no demonstrated stop-wait speedup

Same generated 7.4-second Windows System.Speech sentence, mono 16 kHz PCM WAV, same installed read-only podcli Whisper Base model. Baseline Rust source was d431249. Ran the existing ignored real-session harness before editing Rust; the new harness uses the identical callback delivery path, realtime 320-sample blocks and production session loop. It timestamps the transcribing transition after capture closes and measures until authoritative engine output. This excludes UIA target checks, clipboard and keyboard delivery, and is not a microphone or release build benchmark. One run per case, rounded to 0.1 second; concurrent build/load and filesystem caching can affect results.

| Case | Model acquisition | Stop to authoritative text | Peak working set / commit |
| --- | ---: | ---: | ---: |
| Before, live previews on | 522.929 ms | 2.4 s | 356 / 845 MB |
| After, cold engine | 996.4456 ms | 2.6 s | 347 / 834 MB |
| After, preloaded engine | 0.0084 ms | 2.6 s | 347 / 834 MB |

Preload itself took 191.7762 ms in the warm probe and happened before capture. Peak capture queue was 0.02 seconds in every run. The recognized sentence was the same in all cases. Cached acquisition avoids loading again, and removed previews avoid redundant inference; **this measurement shows no stop-to-text speedup**. Do not present a percentage improvement. Full Whisper final decoding remains the dominant wait. No new Parakeet timing claim; its buffered fast path and recovery are covered by deterministic tests.

Reproduce: source scripts/prepare-native.ps1 -InstallBuildTools in PowerShell, set CARGO_TARGET_DIR=F:\tuck-agent-target-2, TUCK_TEST_MODEL to a catalog-compatible Base model, TUCK_TEST_WAV to the 16 kHz mono PCM fixture. Run cargo test --manifest-path src-tauri/Cargo.toml real_session_stop_to_text -- --ignored --nocapture. Set TUCK_TEST_WARM=1 for preload. Baseline test name: real_session_with_live_transcript, with TUCK_TEST_NO_PREVIEW unset. The generated sentence is: This is a local dictation fixture. Release the shortcut and insert these words into the focused text field.

Local ignored logs: artifacts/before-timing.log, after-cold.log, after-warm.log; WAV: artifacts/dictation-fixture.wav. Not committed. No model files copied or modified.

## Validation

All required commands exited 0: bun test (224 pass), bun run build, bun run test:ui, cargo fmt --check, cargo clippy --all-targets -- -D warnings, cargo test, using --manifest-path src-tauri/Cargo.toml for Cargo. Final Rust result: 272 library tests and 16 integration tests passed; 16 explicit device/network/model probes ignored. Cold and warm real-model fixture probes each exited 0 separately.

Tests cover typed/copied/review decisions, review focus moved to a new field, missing/rejected targets, clipboard errors, uncertain delivery, legacy review preference, removal of preview payloads, one final inference, idle lifetime, capture while loading, cancellation, CPU guard and update blocking. Existing buffered-stream offset/recovery/deadline tests remain. Smoke checks recording without words, rejected legacy partial events, review Insert invocation, done, and Copied feedback. Four screenshots in docs/screens/dictation-{recording,done,copied,review}.png use mocked native events and fixture text.

Unverified: real microphone, accented/noisy audio accuracy, hardware shortcut release timing, real cross-app typing/clipboard writes, password-field behavior on hardware, physical nonactivating Insert click, measured idle RSS after unloading, native noncooperative stalls, new Parakeet timings. No installed Tuck process stopped, release published, or PR merged.
