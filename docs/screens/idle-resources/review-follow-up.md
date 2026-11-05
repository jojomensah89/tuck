# Astra review follow-up, 2026-11-05

Merged origin/main first (11550a4, merge commit c0a28a8). No Tuck instance, playback control, computer-use session, or real desktop input was touched in this round. All UI checks used the headless mock bridge.

1. Clipboard ownership: foreground rebinding now receives only foreground hook state and cannot remove the clipboard subscription. Clipboard removal belongs to disable and Context destruction. The mock test enables registration, switches foreground hook, receives a notification, then disables registration.
2. Contention: the reader distinguishes Busy, Ignored (unsupported/excluded), and Text. Busy work remains pending with exponential backoff capped at one second; new notifications supersede the retry delay. Capture, exclusion, disable, stop, and terminal errors stop retries. Settings changes wake the worker so disabling cancels pending work. Unit coverage exceeds the old five-retry limit, captures afterward, and checks return to an untimed wait.
3. Usage: open/startup pulls apply through onUsageUpdate, including an empty snapshot. Revision and request tickets reject pulls overtaken by an event or a newer pull. Unit interleavings and mock-bridge recovery/race tests cover this.
4. Cursor: 40 ms within 400 CSS pixels, 60 ms only beyond that region. Nominal worker timer rates are 25/s near and 16.7/s far, versus the old 25/s and 8.3/s. Each tick requests one UI-thread reconciliation, so the additional far cost is about 8.3 timer ticks and 8.3 UI messages/s; between 200 and 400 CSS pixels the old far rate is now the fast rate. These are configured rates, not measured context switches. UI-thread reconciliation re-reads cursor position and current CSS hit dimensions instead of applying an obsolete outside sample. Tauri/Wry and Tao apply that setter inline on their event thread. Unit tests cover queued outside-to-inside movement, stale CSS dimensions, missing geometry, and DPI boundaries.
5. Visualizer: opening clears both compact and wide spectrum bars synchronously before starting the one-second interval. Mock-bridge coverage checks this before a stale-frame timer can run.

## Remaining real-Windows validation

The polling design still has a detection window before the first poll; it cannot prove a zero-loss instantaneous approach-and-click. The stale queued reconciliation race is fixed, but fast approach-and-click, hover timing, click-through outside the island, DPI/window movement, actual clipboard enable/foreground-switch/copy and prolonged contention must be checked with real input when the desktop is available. If zero-loss clicks require a native input-routing redesign, that remains a follow-up rather than a claim supported by these mock tests.

The resource table and screenshots in this directory predate all these fixes. Re-measure current idle/playback footprints before attributing the original savings to the current head. Live lyrics/playback, HUD input, physical suspend/resume, and clipboard privacy interaction with other applications were not exercised on this occupied desktop.

## Current checks

- bun test: exit 0, 131 passed.
- bun run build: exit 0.
- bun run test:ui: three runs, exits 0 / 0 / 0.
- cargo fmt --check: exit 0.
- cargo test: exit 0, 185 passed across unit/integration suites; 10 live diagnostics ignored.
- cargo clippy --all-targets -- -D warnings: exit 0.

Rust commands used --manifest-path src-tauri/Cargo.toml. No new release binary was launched or measured during review remediation.
