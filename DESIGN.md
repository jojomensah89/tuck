# Tuck visual system

Mode: Operate. A black island merges into the desktop display edge and stays dark under either Windows theme. Settings use a conventional readable desktop form.

Tokens live in code: `src/tokens.css` (spacing, radius, type, icon, size, motion; shared with settings) and the color block at the top of `src/styles.css` (island only). Use a token, not a literal; add a token when a new value is genuinely needed.

## Color

Surface #000000; raised #1c1c1e / hover #2c2c2e / active #3a3a3c. Ink #ffffff, secondary #a1a1a6, muted #636366. Hairline `--line` white 8%; washes white 10% and 18%. Accent comes from the artwork, white fallback. Semantic: ok #30d158, caution #ff9f0a, warn #ff453a (fills), warn-ink #ff766e (red text on raised surfaces). The artwork color rising from the bottom edge (`--art-wash`) is the signature; no other gradients on chrome.

## Spacing, radius, type, icons

- Spacing: 4px grid, `--s-1`..`--s-12` (4, 8, 12, 16, 20, 24, 32, 48). `--s-half` (2px) only for nudges inside small controls.
- Radius: 2, 4, 8 (controls), 12 (chips), 16; island 16 idle, 20 peek, 32 open; notch ears 14.
- Type: Segoe UI Variable Text, Segoe UI, system-ui. Sizes 10, 11, 12, 13, 14, 20, 28, 40. Weights 400 and 500; 600 only for rare emphasis such as the track title and page headings. No negative tracking below 28px. Numbers use `tabular-nums`.
- Icons: one hand-drawn line family (`src/icons.ts`) on a 20x20 grid: 1.5px stroke (`--stroke`, non-scaling so every size carries the same weight), round caps and joins, no icon fonts. Transport glyphs and small dots are filled with the same rounded outline. Sizes 10 (badge glyph), 14 (compact), 16, 20 (controls, chips, rail), 24 (play/pause).
- Hit targets: 36px; 32px for media transport and the lyrics toggle.

## Geometry

Idle 212x32, peek 300x38, open `openWidth` x 210 (default 500, setting range 440-500). The notch is flush with the top edge with 14px concave ears; float has a 10px top gap. Open content sits inside `16px 20px 16px 56px` insets: the left inset clears the 36px tab rail at 8px from the edge, the right inset holds the section index. At 500 wide that leaves 424x178 for content.

### Idle and peek

Idle follows compact leading/trailing Live Activity slots. With no activity, locale-formatted time occupies the leading slot and a 24x12 battery glyph occupies the trailing slot. The glyph fill tracks the reported percentage, adds a bolt while charging, and turns red below 20 percent while discharging. There are no labels, date strings, or decorative visualizers.

An activity owns both slots by explicit priority, highest first: dictation (recording, transcribing or in review), calls, playing media, running timers, downloads, then paused timers and paused media. All share one pure selector. Media uses artwork or the note glyph with a three-bar indicator that moves only during playback; paused playback keeps uneven resting bars. Timers use their activity glyph and tabular `mm:ss`. Dictation uses the microphone glyph and a recording-state meter. When any activity other than the one shown is running, the trailing slot adds one 6px live dot. Without a system battery the idle pill shows only the time; an unknown battery level draws a dashed outline. Settings > Collapsed content can show the current lyric line instead of the track title.

Peek is one line: a leading glyph or artwork, a center title that marquees only when it overflows, and a trailing status indicator. The no-activity peek uses a clock glyph, the same locale-formatted time, and the battery glyph. It never adds a date. Compact slot owners cross-fade and scale using the shared motion tokens; value updates within the same owner do not restart the transition. Reduced motion removes the transition and all looping indicators while preserving their resting shapes.

The open media header contains only the battery glyph beside its existing metadata. Date, clock, and duplicate equalizer chrome are omitted so the track title cannot clip.

### Open navigation

The open island has four tabs on a vertical icon rail at its left edge, and a settings button pinned to the bottom of the rail that opens the settings window (it is not a tab). Tabs group the former pages into sections, top to bottom (`TABS` in `src/tab-nav.ts`):

- Media: now playing with lyrics, then the system audio visualizer
- Today: clock and battery, timer and pomodoro, calendar
- Tools: quick actions, clipboard, files and devices (shelf, calls, radios, downloads), dictation
- Usage: AI usage meters, system stats

Inside a tab the sections scroll vertically and snap one section per view, each the full 178px content height, so no section is restyled to fit. A passive 2px section index at the right edge shows the position; it never takes input.

The rail is the leftover strip from the old previous-page arrow, so it costs no content space; the right arrow strip and the bottom dot row are reclaimed (content grew from 412x170 to 424x178). Its vertical order matches the vertical sections: scrolling past the last section of a tab continues into the next tab.

- Rail: 20px icons, 36x36 hit targets, `title` and `aria-label` on each. `role="tablist"` (vertical), `role="tab"` with `aria-selected`, `aria-controls` and roving `tabindex`; each panel is a focusable `role="tabpanel"` so sections without controls can be scrolled from the keyboard. Icons: Media a pair of notes, Today a calendar, Tools a 2x2 grid, Usage a gauge, Settings a gear. Inactive icons are `--ink-3`; the active tab is one `--raise` tone step (no outline, no glow) with white ink that slides between tabs on `--spring-enter`; hover lifts the ink to `--ink-2` on a half step (`--raise-subtle`).
- Keys: on the rail, arrows move and wrap, Home and End jump. Elsewhere in the open island, ArrowLeft/Right switch tabs and move focus into the new tab.
- Wheel: vertical wheel scrolls the innermost scroller that still has room (a list, then the tab's sections). Only a fresh gesture that starts at the edge switches tabs, so momentum never flips a tab. Moving forward lands on the first section, moving back on the last. Horizontal wheel (trackpad swipe, wheel tilt) switches directly. A switch locks further switching for 350ms. Wheel and swipe stop at the first and last tab.
- Drag: a horizontal pointer drag of 56px, or a 20px flick faster than 0.35px/ms, switches tabs (drag left for the next tab) and never also clicks what it started on. Vertical drags scroll. The seek bar, inputs and file drag-out are excluded.
- The last tab chosen by the user is stored in the live preferences (`lastTab`) and restored on launch; Media is the default. A file drag reveals Tools on the files section without changing the stored tab.

## Motion

Springs are sampled into CSS `linear()`:

- `--spring` (damping 0.68, ~5% overshoot): shell open and peek, 420ms.
- `--spring-enter` (damping 0.78, ~2% overshoot): content and pages entering, 280ms.
- `--ease-out` (critically damped): shell close 300ms, hovers 160ms, value changes.
- `--ease-exit` (accelerating): content leaving, 100ms.

Durations: 100 (instant/exit), 160 (fast), 280 (base), 300 (close), 420 (shell). Press feedback is `scale(0.96)` everywhere.

Shell morph: only the shell box animates width/height/radius. Its layers sit in `.island-clip`; the open layer is laid out once at the open size (`contain: strict`) and clipped, so the morph never relayouts page content. On open, compact content fades and scales out within 100ms, and the open layer fades and scales in (0.96 to 1) starting at 90ms, when the shell is ~65% open; the rail and section index follow 80ms later. On close the order reverses. No blur, no shadow, no permanent `will-change`.

Tabs cross-fade vertically, matching the rail: the outgoing tab slides 8px opposite to the travel direction and fades in 100ms while the incoming tab slides 12px in on `--spring-enter`. Panels share one absolutely positioned box, so the content area never changes height. The rail is built once; only its state changes.

Live content (compact slots, timers, seek bar, header) is written only when a value changes, so CSS animations never restart. The seek fill, track hover and equalizer are transform-only.

Reduced motion: all durations and delays go to zero, nothing slides, scales or loops. Equalizer bars and spectra stay visible as calm, static bars.

## Rules

Consistent SVG icons; overflow-only marquee; tabular times; visible focus; state-synchronized ARIA and inert hidden layers. Real system and provider state always wins over illustrative content. The browser demo is explicitly labeled, with a badge pinned to the bottom-right of the window, below the island, so it never covers content.
