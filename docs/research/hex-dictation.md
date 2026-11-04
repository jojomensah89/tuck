# Hex dictation comparison and Tuck live preview

Inspected 2026-11-04. Hex source pinned to
[`03ce5ea4f34029cc8145a3d26ec7e043939f34a6`](https://github.com/anomalyco/hex/commit/03ce5ea4f34029cc8145a3d26ec7e043939f34a6).
Tuck baseline: `139deb32d013188284e441c75e69ea9abdf4349e`.
Evidence is static source inspection, not execution of Hex or a microphone session.

## Seen in source

- Hex can publish live text during Windows recording. Its capture loop calls
  `report_live_text()` before release:
  [Windows dictation](https://github.com/anomalyco/hex/blob/03ce5ea4f34029cc8145a3d26ec7e043939f34a6/src/windows/dictation.rs#L780).
- Its streaming adapter distinguishes committed and tentative output and finalization:
  [streaming engine](https://github.com/anomalyco/hex/blob/03ce5ea4f34029cc8145a3d26ec7e043939f34a6/src/transcription/engine/streaming.rs#L7).
  This is path/model-dependent; it does not establish that every backend streams.
- Live text is owned by a recording and cleared with ownership checks:
  [live-text lifecycle](https://github.com/anomalyco/hex/blob/03ce5ea4f34029cc8145a3d26ec7e043939f34a6/src/dictation/live_text.rs#L71).
- Hex has custom vocabulary, pronunciation aliases, and boosting:
  [vocabulary](https://github.com/anomalyco/hex/blob/03ce5ea4f34029cc8145a3d26ec7e043939f34a6/src/dictation/vocabulary.rs#L21).
- Tuck already defaults to optional local previews and immediate insertion.
  Its Whisper preview retranscribes a rolling 15-second window. Before this change,
  `src/live-ui.ts` received partial text but the dictation branch in `src/main.ts`
  displayed only the microphone and state-driven bars in the collapsed island.

## Chosen change

Expose Tuck's existing preview in the collapsed island. Reserve 300px throughout
recording, processing, and review so each update does not change native hit bounds.
Show a bounded trailing excerpt, keep the full preview in the expanded transcript,
and retain it during final inference. Clearly distinguish provisional preview from
the authoritative transcript. Never concatenate overlapping rolling windows.

Native previews and state snapshots share monotonically increasing revisions and
a recording session ID. Full snapshots make reordered events self-contained; the
renderer rejects late events and late initial IPC responses. Clear provisional
text on cancellation, errors, successful insertion, and new sessions.

Skip missed preview deadlines, check release before another preview, and reject
publication after stop/cancel. Preserve the existing explicit microphone/model
lifecycle, local recognition, bounded audio, and foreground-checked Unicode
insertion. This implementation adapts behavior; no Hex code or dependencies are
copied into Tuck.

## Alternatives and next features

1. **Recommended next: separate capture draining from inference.** The current
   synchronous preview blocks the capture consumer while the bounded callback
   queue fills. A single inference worker and one replaceable pending preview
   would avoid a backlog. Inferred benefit: better release responsiveness and
   less queue pressure. Requires slow-inference and cancellation barrier tests.
2. **True streaming engine.** Hex demonstrates the architecture, but replacing
   Whisper or adding another backend requires separate model-size, Windows CPU,
   memory, accuracy, and language evaluation. Lowering the current preview
   interval alone increases work and does not remove the blocking consumer.
3. **Custom vocabulary.** Useful for personal names and project terminology.
   Evaluate a small local phrase list separately; pronunciation boosting support
   depends on the recognition backend, so it should not be promised for the
   current Whisper path without evaluation.

## Verification boundary

Automated checks can establish event ordering, safe text rendering, collapsed
visibility without focus transfer, retention during processing, clearing on
terminal states, and native publication guards. Actual microphone latency,
recognition accuracy, insertion into real Windows apps, and Hex HUD behavior
remain manual checks. The current rolling preview is not token-by-token streaming;
first text still requires capture time plus local inference.
