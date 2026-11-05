# Buffered Parakeet streaming follow-up

Approved direction: one worker-owned borrowed transcribe.cpp stream per recording, newly accepted continuous 16 kHz mono samples, display-only committed plus tentative hypotheses, authoritative finalized full text, retained zeroizing full clip for fallback. Whisper retains its preview/final pipeline. No insertion or focus code changes.

Implementation uses the pinned native default Auto commit policy (agreement default 3), and Unified English native 5.6 s left context / 1.04 s chunk / 2.08 s lookahead. Capture submits whole source seconds with sample offsets through the existing one-slot bounded channel. Rejected submissions retain their cursor and retry; finalization feeds exactly the remaining resampled tail. Whole-second source boundaries preserve resampler phase at all supported integer sample rates. Capture never waits on inference. On release the microphone is dropped and accepted callback audio drained before the final job can wait. Native cancellation has its own retained token and does not use the capture stop flag; individual stream calls have an eight-second cooperative budget.

Fallback rejects init/feed/finalize errors, mismatched offsets, aborted/truncated output, empty text, missing token timing, trailing uncovered activity, and long low-level pauses. It drops the stream before batch inference and clears provisional display text. The full original clip is preserved throughout. Presentation OFF still runs streaming.

Tradeoff / review objection: this version preserves ALL pause audio and conservatively falls back for >5.6 s below a fixed amplitude floor. It does not implement Hex's adaptive PauseCap head/tail compression. Quiet speech is never cut, but can cause unnecessary fallback. Token/activity coverage is a conservative heuristic, not proof of transcript accuracy. Review should decide whether adaptive pause capping is required before approval. Deterministic tests for every native failure/deadline branch remain incomplete.

Evidence sources (static reading, not runtime proof):
- Hex pinned 03ce5ea4f34029cc8145a3d26ec7e043939f34a6, src/transcription/engine/streaming.rs: StreamFeed, committed/tentative, PauseCap and token timing guard. No Hex code copied.
- transcribe.cpp pinned 2778ae4: bindings/rust/transcribe-cpp/src/session.rs, streaming.rs, cancel.rs; docs/models/parakeet-unified-en-0.6b.md. Real Rust APIs, safe borrowed ownership.

Observed synthetic CPU diagnostics (debug build, this machine only):
- Same six-second System.Speech clip as prior baseline: visible words first nonempty ~3.3 s, post-release ~1.8 s, load ~0.77 s. Prior recorded baseline ~2 s post-release; one run does not establish a speedup.
- Hidden words: finalized stream authoritative, no batch fallback; post-release ~1.8 s, load ~0.73 s, capture queue peak ~0.02 s.
- Clip + seven seconds digital silence + repeated clip: correct whole-clip fallback, both sentences retained, post-release ~8.1 s, capture queue peak ~0.02 s. Fallback can be slower than the original batch flow.
- W -> P -> W diagnostic passed, including preexisting real preview budget cancellation and final recovery. This diagnostic tests backend coexistence; it is not proof of interleaved concurrent recognition.

Local raw diagnostic logs live in ignored artifacts/buffered-*.log. No model downloaded/copied/modified, microphone opened, audio played, keyboard injected, clipboard touched, or messages sent. Live microphone/focus/selection/insertion behavior still requires manual validation. Frontend payload unchanged.

Validation: real native cargo test --lib: 255 passed, 17 ignored; cargo clippy --lib -- -D warnings; cargo fmt; bun test: 152 passed; bun run build. Explicit ignored real probes run separately: visible stream, hidden stream, seven-second pause/resume, W/P/W coexistence, native stream cancel during feed/finalization followed by session reuse. Unit checks cover aligned resampling/tail, hidden stream offset/order/full final clip, missing timings, empty text, uncovered trailing phrase, quiet speech, and long pause rejection; existing slow-engine/capture/stop/session-limit tests retained. No frontend payload or markup changed, so UI smoke was not run.
