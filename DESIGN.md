# Tuck visual system

Mode: Operate. A black island merges into the desktop display edge and stays dark under either Windows theme. Settings use a conventional readable desktop form.

Tokens live in code: `src/tokens.css` (spacing, radius, type, icon, size, motion; shared with settings) and the color block at the top of `src/styles.css` (island only). Use a token, not a literal; add a token when a new value is genuinely needed.

## Color

Surface #000000; raised #1c1c1e / hover #2c2c2e / active #3a3a3c. Ink #ffffff, secondary #a1a1a6, muted #636366. Hairline `--line` white 8%; washes white 10% and 18%. Accent comes from the artwork, white fallback. Semantic: ok #30d158, caution #ff9f0a, warn #ff453a (fills), warn-ink #ff766e (red text on raised surfaces). The artwork color rising from the bottom edge (`--art-wash`) is the signature; no other gradients on chrome.

## Spacing, radius, type, icons

- Spacing: 4px grid, `--s-1`..`--s-12` (4, 8, 12, 16, 20, 24, 32, 48). `--s-half` (2px) only for nudges inside small controls.
- Radius: 2, 4, 8 (controls), 12 (chips), 16; island 16 idle, 20 peek, 32 open; notch ears 14.
- Type: Segoe UI Variable Text, Segoe UI, system-ui. Sizes 10, 11, 12, 13, 14, 20, 28, 40. Weights 400 and 500; 600 only for rare emphasis such as the track title and page headings. No negative tracking below 28px. Numbers use `tabular-nums`.
- Icons: one filled SVG set (`src/icons.ts`), sizes 10 (badge glyph), 14 (compact), 16, 20 (controls, chips), 24 (play/pause).
- Hit targets: 36px; 32px for media transport and the lyrics toggle.

## Geometry

Idle 212x32, peek 300x38, open `openWidth` x 210 (default 500, setting range 440-500). The notch is flush with the top edge with 14px concave ears; float has a 10px top gap. Open pages sit inside `16px 44px 24px` insets (the side inset meets the 36px nav buttons).

### Idle and peek

Idle follows compact leading/trailing Live Activity slots. With no activity, locale-formatted time occupies the leading slot and a 24x12 battery glyph occupies the trailing slot. The glyph fill tracks the reported percentage, adds a bolt while charging, and turns red below 20 percent while discharging. There are no labels, date strings, or decorative visualizers.

An activity owns both slots by explicit priority, highest first: dictation (recording, transcribing or in review), calls, playing media, running timers, downloads, then paused timers and paused media. All share one pure selector. Media uses artwork or the note glyph with a three-bar indicator that moves only during playback; paused playback keeps uneven resting bars. Timers use their activity glyph and tabular `mm:ss`. Dictation uses the microphone glyph and a recording-state meter. When any activity other than the one shown is running, the trailing slot adds one 6px live dot. Without a system battery the idle pill shows only the time; an unknown battery level draws a dashed outline. Settings > Collapsed content can show the current lyric line instead of the track title.

Peek is one line: a leading glyph or artwork, a center title that marquees only when it overflows, and a trailing status indicator. The no-activity peek uses a clock glyph, the same locale-formatted time, and the battery glyph. It never adds a date. Compact slot owners cross-fade and scale using the shared motion tokens; value updates within the same owner do not restart the transition. Reduced motion removes the transition and all looping indicators while preserving their resting shapes.

The open media header contains only the battery glyph beside its existing metadata. Date, clock, and duplicate equalizer chrome are omitted so the track title cannot clip.

## Motion

Springs are sampled into CSS `linear()`:

- `--spring` (damping 0.68, ~5% overshoot): shell open and peek, 420ms.
- `--spring-enter` (damping 0.78, ~2% overshoot): content and pages entering, 280ms.
- `--ease-out` (critically damped): shell close 300ms, hovers 160ms, value changes.
- `--ease-exit` (accelerating): content leaving, 100ms.

Durations: 100 (instant/exit), 160 (fast), 280 (base), 300 (close), 420 (shell). Press feedback is `scale(0.96)` everywhere.

Shell morph: only the shell box animates width/height/radius. Its layers sit in `.island-clip`; the open layer is laid out once at the open size (`contain: strict`) and clipped, so the morph never relayouts page content. On open, compact content fades and scales out within 100ms, and the open layer fades and scales in (0.96 to 1) starting at 90ms, when the shell is ~65% open; nav and dots follow 80ms later. On close the order reverses. No blur, no shadow, no permanent `will-change`.

Pages cross-fade: the outgoing page slides 8px opposite to the travel direction and fades in 100ms while the incoming page slides 12px in on `--spring-enter`. Dots are built once; only their state changes.

Live content (compact slots, timers, seek bar, header) is written only when a value changes, so CSS animations never restart. The seek fill, track hover and equalizer are transform-only.

Reduced motion: all durations and delays go to zero, nothing slides, scales or loops. Equalizer bars and spectra stay visible as calm, static bars.

## Rules

Consistent SVG icons; overflow-only marquee; tabular times; visible focus; state-synchronized ARIA and inert hidden layers. Real system and provider state always wins over illustrative content. The browser demo is explicitly labeled.
