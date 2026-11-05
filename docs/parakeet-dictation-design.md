# Approved dictation flow

Keep the existing hold/toggle shortcuts and selected Whisper model. Offer Parakeet Unified English explicitly, recommended for English; downloading and selecting remain user actions. All inference is local. The island stays collapsed and never requests focus. Show real sample RMS before provisional words; with previews off retain the meter. Never insert provisional text or submit.

At finalization insert into the unchanged original editable target. Missing/changed targets copy final text with a short Copied announcement. No-speech/cancel do not copy. Partial input stays in review with automatic retry disabled, avoiding a duplicate full transcript. Review-before-insert remains opt-in.

## Source comparison and judgments

Static evidence: Hex 03ce5ea4f34029cc8145a3d26ec7e043939f34a6, src/dictation/indicator.rs meter, src/transcription/artifacts.rs artifact pin, src/transcription/engine/streaming.rs authoritative finalize and PauseCap, src/windows/paste.rs clipboard handling. Tuck adapts the lifecycle and actual-amplitude approach, without copying Hex implementation code. Hex source: https://github.com/anomalyco/hex/tree/03ce5ea4f34029cc8145a3d26ec7e043939f34a6

Parakeet uses transcribe.cpp 2778ae4bf5ca5f394d129a39467f4ad9552dc34f, CPU only with portable build flags. Previews are bounded whole clips; final inference uses the entire retained recording, never concatenated tentative text. This avoids the buffered stream long-pause truncation described in Hex, at the cost of final-pass latency. Runtime performance is unknown until actual model testing.

Artifact metadata checked through the canonical Hugging Face tree API at revision 7e948f21b7bdbac698d3318db9d350f1096f3b6c. Model card: https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf/blob/7e948f21b7bdbac698d3318db9d350f1096f3b6c/README.md . No model downloaded during implementation. CC BY 4.0 attribution is recorded in parakeet-model-notice.md.

UIA TextPattern Document support is a source-based repair for contenteditable targets; actual T3 accessibility behavior remains runtime-unknown. Unicode injection preserves all clipboard formats on direct insertion. Insertion requires observed UIA before/after text to confirm the exact addition. Replacement of a selected range or application normalization can conservatively fail confirmation and require manual reconciliation. Interactive verification is required before claiming the T3 issue fixed. Focus sampling is bounded to the active capture loop; a change and return between samples cannot be observed.


## Validation and remaining runtime evidence

Bun: 152 tests pass. Production frontend build passes. Headless Edge UI smoke passes, with collapsed/no-focus-transfer, provisional text escaping, stale revisions, waveform and reduced-motion coverage. Listening screenshot inspected. Real native cargo test: 248 passed, 15 explicitly ignored manual/hardware tests. Native clippy with warnings denied and rustfmt pass. No native stubs were used.

No microphone was activated, no production input was injected, and no clipboard was changed in tests. No model was downloaded. Actual Parakeet inference, actual Whisper/Parakeet ggml runtime coexistence, performance/memory, and T3 accessibility insertion require runtime verification. Static native libraries link successfully; that alone does not establish runtime ABI compatibility. This is a review boundary, not measured model proof. CPU builds disable host tuning and AVX512, retaining the native backend's default x86 instruction baseline; older CPU compatibility is unmeasured.
