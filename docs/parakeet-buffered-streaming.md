# Buffered Parakeet streaming recovery

One worker owns a borrowed transcribe.cpp stream per recording. Continuous 16 kHz mono samples enter through the existing one-slot job channel with sample offsets; rejected submissions retain their cursor. Finalization feeds the remaining tail exactly once. Whole source seconds preserve resampling phase. Capture remains nonblocking, recording remains bounded to 120 seconds, and the full clip remains zeroizing. Live words OFF still streams. Whisper and UI payloads are unchanged.

Recovery rejects init/feed/finalize errors, offset mismatches, deadline cancellation, aborted/truncated output, empty text, missing/invalid token timing, uncovered activity, and long low-level pauses. The stream drops before the full original clip is passed once to batch inference. The old native cancellation token is detached; batch installs its own fresh token. Cancellation during fallback suppresses the returned text. Scoped watchers now finish on unwind, including a panicking event callback, without requiring user Cancel. Both streaming and batch native watchers retain the completion guard.

Token times in pinned transcribe.cpp are signed i64 milliseconds, converted at 16 samples/ms. Coverage checks leading, interior, and trailing gaps with 400 ms slack; at least 300 ms of uncovered activity rejects streaming. Missing, reversed, negative, zero-length, overflowing, or implausibly late timings conservatively reject. Coverage does not prove that every word was recognized.

Accepted tradeoff: preserve all pause audio and fall back after more than 5.6 seconds below a fixed amplitude floor. No adaptive Hex PauseCap compression. Quiet speech is never cut, but this heuristic can cause unnecessary fallback. Cooperative eight-second native call budgets cannot interrupt native code that stops polling cancellation.

## Deterministic verification

A private borrowed-stream/backend and monotonic-clock seam executes the production buffered driver, including its real scoped watcher and cancellation token, without models or devices. Three new tests exercise:

- Init, feed, finalize, and offset failure; feed and finalize deadline expiry using an advanced fake clock and waiting for the actual watcher to cancel. Each rejection drops the stream before exactly one batch call with the unchanged full clip. A successful second recording on the same backend verifies fresh token state after each scenario.
- Successful contiguous feeds plus final tail: exact sample-vector equality proves no duplicated or dropped samples.
- Cancellation inside fallback: even a backend returning successful text yields an error, preventing delivery.
- A panicking partial callback: catch unwind through the production driver, verify stream destruction and watcher join within a timeout without user Cancel, then reuse the backend successfully.

Coverage regressions separately reject omitted opening/middle/end activity and accept valid intervals, including quiet speech. Existing capture, queue, session-limit, hidden-stream, and aligned-resampling tests remain.

## Fresh native evidence

2026-11-05, debug CPU build, synthetic WAVs only, installed read-only models:

- Native `cargo test --lib --locked`: **259 passed, 17 ignored**. `cargo clippy --lib --locked -- -D warnings`, `cargo fmt -- --check`, and `git diff --check` passed.
- Visible six-second clip: finalized stream accepted, first nonempty partial ~4.5 s, stop wait ~3.9 s, load ~0.78 s, peak capture queue ~0.04 s.
- Hidden same clip: finalized stream accepted, stop wait ~1.9 s, load ~0.50 s, queue ~0.02 s.
- Clip + seven-second silence + repeated clip: full-clip fallback, both sentences retained, stop wait ~7.3 s, queue ~0.02 s.
- Native feed/finalization cancel followed by session reuse passed. Whisper -> Parakeet -> Whisper coexistence passed; this does not prove concurrent recognition.

Five explicit ignored probes passed separately. Logs: ignored `artifacts/buffered-recovery-{tests,clippy,visible,hidden,pause,cancel,coexistence}.log`. These results supersede earlier diagnostic timings. No established speedup versus the previously observed ~2 s batch baseline; fallback can be slower. Generated-phrase checks do not establish general model accuracy.

Static sources: Hex 03ce5ea4f34029cc8145a3d26ec7e043939f34a6 `src/transcription/engine/streaming.rs` (no code copied); transcribe.cpp 2778ae4 Rust `session.rs`, `streaming.rs`, `result.rs`, `cancel.rs` and Parakeet model documentation.

No downloads, model copies/modifications, microphone/device access, playback, keyboard/clipboard writes, or messages. No frontend changes, so Bun/UI checks were not rerun. Live microphone, focus, selection, and insertion behavior remains manual validation. Native allocator/FFI failure injection and real eight-second stalls are not reproduced by deterministic fake-backend tests.
