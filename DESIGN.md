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
- Hit targets: 36px; 32px for media transport, the lyrics toggle and inline row actions (grip, radio icon). Nothing smaller.
- Icon buttons (`.icon-btn`): one line icon, 36px, `--ink-2` at rest and `--ink` with a `--raise` step on hover; the name is both `title` and `aria-label`. The one primary control of a group (play, record, start call timer) is a round `--raise` fill. Text labels stay only where an icon would be ambiguous (timer presets are numerals, row text is the content itself). Provider marks (`anthropic`, `openai`) are simplified monochrome glyphs for identification only; providers without one show their initial.

## Graphics instead of text

Every value that has a range is drawn, not written. Rings come from `src/rings.ts` (pure geometry, tones and labels, unit tested; `createRing` is the one DOM builder): a track one tone step above the surface (`--raise-hover`), a round-capped fill starting at 12 o'clock, and the value changing by dash offset so it animates on `--spring-enter` over `--d-shell`. An empty or missing value is the track alone (a round cap at zero would draw a dot). Reduced motion sets the change instantly.

- AI usage: a row of 64px rings (6px stroke), centred; more than five sources switch to 44px so two rows fit 178px. The provider mark sits in the centre; under it the value (`62%`, `$12.40`, or `31/40` for the ChatGPT estimate) and one line of window and reset (`5h · 2h 15m`, `API · off`, `Est`). The fill is a threshold tone, not a brand colour: green below 70%, amber from 70%, red from 90%. The mark already names the provider, OpenAI's brand colour is black and white, and the question this section answers is "how close am I to a limit". Stale or unavailable sources dim to 45% and show a dash. Full details (label, %, messages, spend, reset, source, error) are the tooltip and accessible name. The ChatGPT ring is itself the "Log message" button, marked by a small plus.
- System: CPU, memory and each GPU as 56px rings with the value inside, the name and a 56x16 sparkline under. Load is ink until it is high (amber 70%, red 90%), so an idle machine is not the loudest thing on screen. Unavailable is a dim track with a dash. A status dot beside the heading replaces the "Live · 14:02" text.
- Today: battery as a 72px ring with the level inside and the bolt above it while charging; green charging, red below 20% on battery, ink otherwise. A line under it only when it adds information (`47m left`). A desktop shows a plug in an empty track.
- Timer: presets 5 / 15 / 25 as numeric chips on the left, a 140px ring in the centre with tabular digits and the title with a status dot, and play/pause, stopwatch and reset as icon buttons on the right. Countdowns drain the ring; a stopwatch sweeps it once a minute. The ring follows the 250ms tick linearly instead of springing. Play with nothing running starts the last preset.
- Status dots (6px): grey ready or stale, green live or running, amber paused, red fault, dark off. The word is the dot's name.

Copy: no trailing period on labels, notes or empty states; one sentence at most. Durations use one format everywhere (`3d 4h`, `2h 15m`, `45m`, `<1m`), timers `mm:ss`, clock times the Windows locale (`formatClockTime`).

## Geometry

Idle 212x32, peek 300x38, open `openWidth` x 210 (default 500, setting range 440-500). The notch is flush with the top edge with 14px concave ears; float has a 10px top gap. Open content sits inside `16px 20px 16px 56px` insets: the left inset clears the 36px tab rail at 8px from the edge, the right inset holds the section index. At 500 wide that leaves 424x178 for content.

### Idle and peek

Idle follows compact leading/trailing Live Activity slots. With no activity, locale-formatted time occupies the leading slot and a 24x12 battery glyph occupies the trailing slot. The glyph fill tracks the reported percentage, adds a bolt while charging, and turns red below 20 percent while discharging. There are no labels, date strings, or decorative visualizers.

An activity owns both slots by explicit priority, highest first: dictation (recording, transcribing or in review), calls, playing media, running timers, downloads, then paused timers and paused media. All share one pure selector. Media uses artwork or the note glyph with a three-bar indicator that moves only during playback; paused playback keeps uneven resting bars. Timers use their activity glyph and tabular `mm:ss`. Dictation uses the microphone glyph and a recording-state meter. When any activity other than the one shown is running, the trailing slot adds one 6px live dot. Without a system battery the idle pill shows only the time; an unknown battery level draws a dashed outline. Settings > Collapsed content can show the current lyric line instead of the track title.

Peek is one line: a leading glyph or artwork, a center title that marquees only when it overflows, and a trailing status indicator. The no-activity peek uses a clock glyph, the same locale-formatted time, and the battery glyph. It never adds a date. Compact slot owners cross-fade and scale using the shared motion tokens; value updates within the same owner do not restart the transition. Reduced motion removes the transition and all looping indicators while preserving their resting shapes.

### Volume and brightness HUD

A volume or brightness change morphs the collapsed island into the HUD at peek size: a 20px glyph (speaker with no, one or two waves by level, a crossed speaker when muted, a sun for brightness), a 4px level bar across the centre (`--raise-hover` track, ink fill that eases on `--ease-out` over `--d-fast`) and the value in tabular figures. Muted volume dims the fill and value to `--ink-3` but keeps the level. The HUD outranks announcements and activities, leaves 1.5s after the last change and then returns to idle, or to peek while the pointer is over the island, so scrolling never opens it. An announcement that arrives during the HUD, or that the HUD interrupts, follows once it settles, and an open island that a peek interrupted comes back open if it is pinned or hovered. Settings > Island > Peeks has its own "Volume and brightness HUD" switch (on by default), separate from System peeks; wheel and middle-click feedback on the island always shows. Screen readers hear the settled value once through the polite status region, 700ms after input pauses, and never the same value twice in one HUD.

The open island does not collapse for a level change. The same glyph, bar and value paint over the pill-sized strip at its top centre for 1.5s, in the idle pill's size and colour, and the status region speaks the value. The strip only paints, so what sits under it stays clickable.

Brightness is noisier than volume, so it has three gates. It shows within 10s of keyboard, mouse or touch input, which keeps idle dimming quiet. It stays quiet for 8s after a charger change. A run of small steps (3% or less, a second or less apart) counts as a ramp and is ignored, which keeps adaptive brightness and charger ramps quiet while typing; a lone small step shows a second late, because only then is it clear no run follows. These gates are heuristics: a slower ramp, or a single adaptive step during input, still shows a HUD. While the Windows flyout is hidden the first two gates are dropped, because the island is then the only feedback and firmware brightness keys may leave no input behind; a laptop with such keys otherwise gets no HUD while idle.

The idle pill keeps its job when the island grows: over a strip with the idle pill's size at the top centre of the island, the wheel changes system volume by 2% per notch (Shift: 10%, values snap to that grid, raising the volume unmutes) and middle click toggles mute, in the idle, peek and open states, including an island that opened by hover. On the collapsed island all of it counts as the strip. Inside the strip the wheel always means volume, even over a list that could scroll, and a plain horizontal swipe does nothing. Elsewhere on the open island the wheel keeps switching tabs and middle click does nothing. Brightness is read from internal panels only; external monitors (DDC/CI) do not show the HUD. Settings > Island > Peeks can hide the Windows flyout (off by default). The flyout comes back while the HUD switch is off, while a fullscreen app hides the island, and while the island is pinned open (there both the flyout and the in-strip level show). See `src-tauri/src/osd_flyout.rs` for the undocumented method and its safeguards.

The open media header contains only the battery glyph beside its existing metadata. Date, clock, and duplicate equalizer chrome are omitted so the track title cannot clip. An overflowing title marquees with its edges faded by a mask instead of a hard cut. Artwork has no shadow. The lyrics toggle is in the transport row's flow at its start; the transport group centres in the remaining room (mirrored by an empty slot while lyrics are closed) and tightens its gaps to 12px with lyrics open, so the toggle never touches the previous-track button at the 440px minimum.

Tools: quick actions are four icon tiles (72px tall, 24px icons, names in tooltips). Clipboard rows are the text itself (select to copy) with pin and remove icons that brighten with the row; a pinned pin stays solid; clear is a trash icon in the heading. Dictation's heading carries its status dot and three icons: record (a stop square while recording), cancel and insert; visible text only for an error. The visualizer has one power toggle. Files, calls, radios and downloads are icon views (folder, phone, Wi-Fi, download) with a count badge, or a green dot while the call timer runs; each view's own actions sit at the right of the same row. Radios are switches (`role="switch"`), shelf rows have a grip to drag out, show-in-folder and remove icons, and calendar rows a video icon to join.

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
- Wheel, outside the pill-sized strip (which is volume, see the HUD above): vertical wheel scrolls the innermost scroller that still has room (a list, then the tab's sections). Only a fresh gesture that starts at the edge switches tabs, so momentum never flips a tab. Moving forward lands on the first section, moving back on the last. Horizontal wheel (trackpad swipe, wheel tilt) switches directly. A switch locks further switching for 350ms. Wheel and swipe stop at the first and last tab.
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
