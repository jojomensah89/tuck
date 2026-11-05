# Approved dictation flow

Keep the existing hold/toggle shortcuts and selected Whisper model. Offer Parakeet Unified English explicitly, recommended for English; downloading and selecting remain user actions. All inference is local. The island stays collapsed and never requests focus. Show real sample RMS before provisional words; with previews off retain the meter. Never insert provisional text or submit.

At finalization insert into the unchanged original editable target. Missing/changed targets copy final text with a short Copied announcement. No-speech/cancel do not copy. Partial input stays in review with automatic retry disabled, avoiding a duplicate full transcript. Review-before-insert remains opt-in.

## Source comparison and judgments

Static evidence: Hex 03ce5ea4f34029cc8145a3d26ec7e043939f34a6, src/dictation/indicator.rs meter, src/transcription/artifacts.rs artifact pin, src/transcription/engine/streaming.rs authoritative finalize and PauseCap, src/windows/paste.rs clipboard handling. Tuck adapts the lifecycle and actual-amplitude approach, without copying Hex implementation code. Hex source: https://github.com/anomalyco/hex/tree/03ce5ea4f34029cc8145a3d26ec7e043939f34a6

Parakeet uses transcribe.cpp 2778ae4bf5ca5f394d129a39467f4ad9552dc34f, CPU only with portable build flags. Previews are bounded whole clips; final inference uses the entire retained recording, never concatenated tentative text. This avoids the buffered stream long-pause truncation described in Hex, at the cost of final-pass latency. Runtime performance is unknown until actual model testing.

Artifact metadata checked through the canonical Hugging Face tree API at revision 7e948f21b7bdbac698d3318db9d350f1096f3b6c. Model card: https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf/blob/7e948f21b7bdbac698d3318db9d350f1096f3b6c/README.md . No model downloaded during implementation. CC BY 4.0 attribution is recorded in parakeet-model-notice.md.

UIA TextPattern Document support is a source-based repair for contenteditable targets; actual T3 accessibility behavior remains runtime-unknown. Unicode injection preserves all clipboard formats on direct insertion. Insertion requires observed UIA before/after text to confirm the exact addition or replacement of a supported selected range. Unsupported selections or application normalization can conservatively require manual reconciliation. TextPattern reads request a truncation sentinel beyond the text limit. Interactive verification is required before claiming the T3 issue fixed.

Focus observation runs outside the audio consumer and runtime lock, at most once every 100 ms. A stalled UIA provider cannot block capture or Cancel. At most one observer exists; if a previous session's provider remains stuck, later sessions conservatively copy rather than insert. Publication checks the session ID; observers do not hold strong service references while calling UIA. Session shutdown never joins a potentially blocked provider. A move-and-return between samples is not observable. Snapshot meter/status/revision are read under the same runtime lock.


## Validation and remaining runtime evidence

Bun: 152 tests pass. Production frontend build passes. Headless Edge UI smoke passes, with collapsed/no-focus-transfer, provisional text escaping, stale revisions, waveform and reduced-motion coverage. Listening screenshot inspected. After review fixes, native cargo test: 251 passed, 16 explicitly ignored manual/hardware tests. The two real-model tests below were additionally run explicitly. Native clippy with warnings denied and rustfmt pass. No native stubs were used.

No microphone was activated, no production input was injected, and no clipboard was changed in tests. No model was downloaded or copied. The existing Hex GGUF was SHA-256 verified against the pinned catalog and read in place. Model discovery now recognizes that exact artifact in Hex's standard model directory; other GGUFs remain excluded.

Real inference with a generated six-second English WAV passed through the capture/inference session harness: first partial at 3.5 seconds, final result about 2 seconds after stop, model load about 0.74 seconds. These are one synthetic clip on this PC, not a general latency guarantee. The final result matched the generated sentence. Reported process peak working set was about 1086 MiB.

A same-process Whisper Tiny → Parakeet → Whisper Tiny test initially failed at Parakeet backend initialization. Both adapters now establish the same CPU-only shared backend policy before Whisper can register defaults; the real alternating-model regression passed, including aborting a Parakeet preview through its native cancel token and then completing final inference on that session. This tests those installed models on this CPU, not every ggml configuration. Live microphone use, real T3 insertion/selection replacement, natural-speech accuracy, and older CPU compatibility remain unverified. CPU builds disable host tuning and AVX512, retaining the native backend's default x86 instruction baseline.
