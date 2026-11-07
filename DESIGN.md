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
- Hit targets: 36px; 32px for the rail, the segmented switch and the actions beside it, media transport, the lyrics toggle, the lyrics timing steppers and inline row actions (grip, pin, remove); 56px for the record control. Nothing smaller.
- Icon buttons (`.icon-btn`): one line icon, 36px, `--ink-2` at rest and `--ink` with a `--raise` step on hover; the name is both `title` and `aria-label`. The one primary control of a group (play, record) is a round `--raise` fill. Text labels stay only where an icon would be ambiguous (timer presets are numerals, row text is the content itself). Provider marks (`claude`, `anthropic`, `openai`) are the companies' own marks, unmodified and monochrome, for identification only (sources are in `src/icons.ts`); providers without one show their initial.

## Graphics instead of text

Every value that has a range is drawn, not written. Rings come from `src/rings.ts` (pure geometry, tones and labels, unit tested; `createRing` is the one DOM builder): a track one tone step above the surface (`--raise-hover`), a round-capped fill starting at 12 o'clock, and the value changing by dash offset so it animates on `--spring-enter` over `--d-shell`. An empty or missing value is the track alone (a round cap at zero would draw a dot). Reduced motion sets the change instantly.

- AI usage: one card per connected provider, two across. The 72px double ring (6px strokes) has the 5h window outside and the weekly window inside; a single-window provider keeps one ring. The official mark sits in the centre at 32px. Two compact lines beside it give window and what is left (`5h 38% left`, `7d 69% left`). The rings drain as quota runs out. Each ring has its own threshold tone from the share used: green below 70% used, amber from 70%, red from 90%. A fifth card shrinks the rings to 44px in three columns so two rows still fit. A connected source with a stale or failed reading stays visible, dimmed to 45% with one word beside its values (`Stale`, `Error`, `Unavailable`) and a dash where there is no value. Sources that are not connected do not appear, and the AI section leaves the Usage tab when none is connected. A model-scoped window (`Opus`) is never drawn as the general weekly, and both rings come from one quota bucket. Reset times, other windows and source or error details live in the tooltip and accessible name. Refresh belongs to this part and sits in the segmented row as a quiet `--ink-2` icon, with no filled tile at rest or on hover.
- System: CPU, memory and each GPU as 72px rings with the value inside, the name and a 72x20 sparkline under. Load is ink until it is high (amber 70%, red 90%), so an idle machine is not the loudest thing on screen. Unavailable is a dim track with a dash. Download and upload rates sit below the rings with line icons, tabular figures and decimal MB/s to one decimal place. Missing or warming values are dim dashes. There is no decorative status dot.
- Today: battery as a 96px ring with the level inside and the bolt above it while charging; green charging, red below 20% on battery, ink otherwise. A line under it only when it adds information (`47m left`). A desktop shows a plug in an empty track.
- Timer: presets 5 / 15 / 25 as numeric chips on the left, a 140px ring in the centre with tabular digits and the title with a status dot, and play/pause, stopwatch and reset as icon buttons on the right. Countdowns drain the ring; a stopwatch sweeps it once a minute. The ring follows the 250ms tick linearly instead of springing. Play with nothing running starts the last preset.
- Status dots (6px): grey ready or stale, green live or running, amber paused, red fault, dark off. The word is the dot's name.

Empty states are one calm line in the middle of the area and nothing else (`Nothing playing`, `Nothing copied yet`). Copy: no trailing period on labels, notes or empty states; one sentence at most. Durations use one format everywhere (`3d 4h`, `2h 15m`, `45m`, `<1m`), timers `mm:ss`, clock times the Windows locale (`formatClockTime`).

## Geometry

Idle is 212x32, or a stable 300x32 while playing media; peek is 300x38; open is `openWidth` x 264 (default 500, range 440-500). Notch ears remain 14px; float has a 10px top gap.

The native window is a fixed, click-through 520x288 canvas (`src-tauri/tauri.conf.json`), and `clamp_hit_size` in `src-tauri/src/system.rs` uses the same limits. That holds the 264px island, the 10px float gap and the shell spring's overshoot of about 12px at both shapes. The native hit area follows the shell, so it is `openWidth` x 264 open, or 274 tall when floating.

Content box: 16px insets top and bottom, 56px left (8px gutter, 32px rail, 16px gap), 24px right.

| Open width | Whole box (one part, no row) | Below the row (two parts) |
| --- | --- | --- |
| 500 | 420x232 | 420x192 |
| 440 | 360x232 | 360x192 |

The row is 32px with an 8px gap under it, at y=16..48, so a part starts at y=56. Tab panels own that offset, so an outgoing tab keeps its shape while it fades.

The rail occupies x=8..40, y=12..252, 240px tall: five 32px tabs from the top (to y=172), the Settings gear pinned at the bottom (y=220..252), and between them the reserved 32px update slot (`#railUpdate`) right above the gear (y=188..220). The empty slot is hidden and takes no space. Seven items use 224px with 16px spare, and the rail never covers the content box.

### Idle and peek

Idle follows compact leading/trailing Live Activity slots. With no activity, locale-formatted time occupies the leading slot and a 24x12 battery glyph occupies the trailing slot. The glyph fill tracks the reported percentage, adds a bolt while charging, and turns red below 20 percent while discharging. There are no labels, date strings, or decorative visualizers.

An activity owns both slots by explicit priority, highest first: dictation (recording, transcribing or in review), playing media, running timers, then paused timers. All share one pure selector. Playing media uses artwork or the note glyph with a moving three-bar indicator. A paused track gives the idle pill back to time and battery after a 1s hold, so a pause between two tracks does not resize it; a peek still names it, with uneven resting bars. Timers use their activity glyph and tabular `mm:ss`. Dictation uses the microphone glyph and a recording-state meter. When any activity other than the one shown is running, the trailing slot adds one 6px live dot. Without a system battery the idle pill shows only the time; an unknown battery level draws a dashed outline. Settings > Collapsed content sets what playing media shows between those slots: the current lyric (the default, falling back to the title when there is no line), the title, or artwork alone. Text stays up the whole time media plays. The pill widens to 300px once for the track, never per line, so a new lyric cannot make it jitter. A line too long for the pill scrolls once, starting after 300ms and finishing inside a 4s lyric line, then stops on an ellipsis until the text changes; reduced motion shows the ellipsis without scrolling. Lines are written only when they change, and the native hit area follows the pill. Collapsed with nothing playing, no animation runs at all; the equalizer bars run only during playback and stop on pause.

Peek is one line: a leading glyph or artwork, a center title that marquees only when it overflows, and a trailing status indicator. The no-activity peek uses a clock glyph, the same locale-formatted time, and the battery glyph. It never adds a date. Compact slot owners cross-fade and scale using the shared motion tokens; value updates within the same owner do not restart the transition. Reduced motion removes the transition and all looping indicators while preserving their resting shapes.

### Volume and brightness HUD

A volume or brightness change morphs the collapsed island into the HUD at peek size: a 20px glyph (speaker with no, one or two waves by level, a crossed speaker when muted, a sun for brightness), a 4px level bar across the centre (`--raise-hover` track, ink fill that eases on `--ease-out` over `--d-fast`) and the value in tabular figures. Muted volume dims the fill and value to `--ink-3` but keeps the level. The HUD outranks announcements and activities, leaves 1.5s after the last change and then returns to idle, or to peek while the pointer is over the island, so scrolling never opens it. An announcement that arrives during the HUD, or that the HUD interrupts, follows once it settles, and an open island that a peek interrupted comes back open if it is pinned or hovered. Settings > Island > Peeks has its own "Volume and brightness HUD" switch (on by default), separate from System peeks; wheel and middle-click feedback on the island always shows. Screen readers hear the settled value once through the polite status region, 700ms after input pauses, and never the same value twice in one HUD.

The open island does not collapse for a level change. The same glyph, bar and value paint over the pill-sized strip at its top centre for 1.5s, in the idle pill's size and colour, and the status region speaks the value. The strip only paints, so what sits under it stays clickable.

Brightness is noisier than volume, so it has three gates. It shows within 10s of keyboard, mouse or touch input, which keeps idle dimming quiet. It stays quiet for 8s after a charger change. A run of small steps (3% or less, a second or less apart) counts as a ramp and is ignored, which keeps adaptive brightness and charger ramps quiet while typing; a lone small step shows a second late, because only then is it clear no run follows. These gates are heuristics: a slower ramp, or a single adaptive step during input, still shows a HUD. While the Windows flyout is hidden the first two gates are dropped, because the island is then the only feedback and firmware brightness keys may leave no input behind; a laptop with such keys otherwise gets no HUD while idle.

The idle pill keeps its job when the island grows: over a strip at the top centre of the island with the size the pill would have if it closed now, the wheel changes system volume by 2% per notch (Shift: 10%, values snap to that grid, raising the volume unmutes) and middle click toggles mute, in the idle, peek and open states, including an island that opened by hover. On the collapsed island all of it counts as the strip. Inside the strip the wheel always means volume, even over a list that could scroll or the top half of the segmented row, and a plain horizontal swipe does nothing. Elsewhere on the open island the wheel follows the navigation rules below and middle click does nothing. Brightness is read from internal panels only; external monitors (DDC/CI) do not show the HUD. Settings > Island > Peeks can hide the Windows flyout (off by default). The flyout comes back while the HUD switch is off, while a fullscreen app hides the island, and while the island is pinned open (there both the flyout and the in-strip level show). See `src-tauri/src/osd_flyout.rs` for the undocumented method and its safeguards.

The open media header gives the title a row of its own at 20px beside 112px artwork, with the artist below it and the battery at the end of that artist row. Date, clock, and duplicate equalizer chrome are omitted. A title that overflows marquees with its edges faded by a mask instead of a hard cut, and only then. Artwork has no shadow. Beside open lyrics the player column stacks: 80px artwork, then the 14px title and artist across the whole column. The lyrics toggle is in the transport row's flow at its start; the transport group centres in the remaining room (mirrored by an empty slot while lyrics are closed) and tightens its gaps to 12px with lyrics open, so the toggle never touches the previous-track button at the 440px minimum.

Without lyrics (none found, still looking, turned off, or an error) the lyrics pane stays closed however the toggle was left and the player takes the full width. The toggle goes quiet, `--ink-3` and `aria-disabled`, and its tooltip says why. With nothing playing the player gives way to one line.

Tools: Clipboard rows are the text itself (select to copy) with pin and remove icons that brighten with the row; a pinned pin stays solid. Shelf rows have a grip to drag out, show-in-folder and remove icons. Row actions are 32px and line up under the actions in the segmented row.

### Open navigation

The open island has five tabs, in order (`TABS` in `src/tab-nav.ts`):

- Media: player and available lyrics, no switch
- Today: Clock (clock and battery) | Timer (timer and pomodoro)
- Dictation: its own microphone tab, no switch
- Tools: Files | Clipboard
- Usage: AI | System

Segmented switch: on the left of the row, 32px tall with equal segments, `--raise` track and `--raise-active` selected. It is a horizontal tablist with `aria-selected`, `aria-controls` (each part is a `tabpanel`), roving tabindex, wrapping arrows and Home/End; these keys stay in the switch and do not change tabs. Selecting a segment moves the same snap scroller that holds the parts, and remembers the tab. The part off screen is inert. With one part left the switch goes, and the row goes too unless that part has an action.

Section actions: on the right of the row, 32px icons. Each group names the parts it serves (`data-section-actions`), moves there once and is only shown or hidden after that, so focus survives. Tools ends with one Screen snip icon on both parts, with the part's own action beside it (Clear shelf, Clear unpinned history). AI has Refresh. Only the part on screen shows its own.

Rail: 20px icons in 32px targets, title and aria-label, vertical tablist with roving focus and aria-selected/controls. Arrows wrap; Home/End jump. Elsewhere ArrowLeft/Right switch tabs. Inactive ink-3, active ink over raise; the indicator slides using transform.

Wheel, outside the volume strip: vertical wheel scrolls the innermost list that has room, then moves between the tab's parts, then on to the next tab. Each step needs a fresh gesture at the edge: one gesture moves one part or one tab, with a 350ms lock and momentum suppressed, and the panel itself never free-scrolls. Going forward lands on a tab's first part, going back on its last. Over the rail the wheel moves tabs directly. Horizontal wheel and horizontal pointer swipes switch tabs directly. Gestures stop at the first and last tab. Swipe thresholds remain 56px, or a 20px flick above 0.35px/ms; inputs, the seek bar, the transcript, the drop target and file drag-out are excluded.

Saved `lastTab` values from older versions (`media`, `today`, `tools`, `usage`) all stay valid. Media is the default. A file drag reveals Tools / Files without changing the saved tab. Nothing here polls or animates while collapsed, and hidden tabs do no polling: telemetry runs only on Usage, media sessions only on Media.

Dictation: a status row (dot, one status line that may wrap to two for an error, and the model name), the live transcript filling the room left on a `--raise-subtle` surface, then cancel, a 56px record control and insert. Live words follow the newest line and the text can be selected. A missing model disables record, says so and swaps the model name for a Model settings link to that page of Settings.

Files: one dashed drop target, the only dashed element in the product. On an empty shelf it fills the content area with a folder icon and "Drop files here"; with files it is a slim 32px strip above the list. Clicking it picks files. A drag over the island highlights it (`--raise` fill, ink text and border).

## Motion

Springs are sampled into CSS `linear()`:

- `--spring` (damping 0.68, ~5% overshoot): shell open and peek, 420ms.
- `--spring-enter` (damping 0.78, ~2% overshoot): content and pages entering, 280ms.
- `--ease-out` (critically damped): shell close 300ms, hovers 160ms, value changes.
- `--ease-exit` (accelerating): content leaving, 100ms.

Durations: 100 (instant/exit), 160 (fast), 280 (base), 300 (close), 420 (shell). Press feedback is `scale(0.96)` everywhere.

Shell morph: only the shell box animates width/height/radius. Its layers sit in `.island-clip`; the open layer is laid out once at the open size (`contain: strict`) and clipped, so the morph never relayouts page content. On open, compact content fades and scales out within 100ms, and the open layer fades and scales in (0.96 to 1) starting at 90ms, when the shell is ~65% open; the rail and segmented row follow 80ms later. On close the order reverses. No blur, no shadow, no permanent `will-change`.

Tabs cross-fade vertically, matching the rail: the outgoing tab slides 8px opposite to the travel direction and fades in 100ms while the incoming tab slides 12px in on `--spring-enter`. Panels share one absolutely positioned box, so the content area never changes height. The rail is built once; only its state changes.

Live content (compact slots, timers, seek bar, header) is written only when a value changes, so CSS animations never restart. The seek fill, track hover and equalizer are transform-only.

Reduced motion: all durations and delays go to zero, nothing slides, scales or loops. Equalizer bars stay visible as calm, static bars.

## Rules

Consistent SVG icons; overflow-only marquee; tabular times; visible focus; state-synchronized ARIA and inert hidden layers. Real system and provider state always wins over illustrative content. The browser demo is explicitly labeled, with a badge pinned to the bottom-right of the window, below the island, so it never covers content.

## Update alerts

The island owns the startup and six-hour one-shot check; Rust owns the update handle, verified downloaded bytes and serialized operation state. Both webviews subscribe to the same snapshot and events; opening Settings during a download reads its current progress. Checks cannot replace a downloaded package. Download and restart require separate clicks; Retries reuse downloaded bytes, including after an installer failure restarts the app to restore services. The package is saved in the app cache and verified again against current release metadata, signature and signed version before reuse; corrupt or obsolete cache is discarded.

A version is claimed in island local storage once across restarts. Its one-line download-icon peek waits for dictation and the HUD. No update polling or idle animation is added. The reserved rail slot contains a 32px button outside the tablist, with ink line artwork and a 6px ok dot. Hidden means no space. Download progress uses the shared ring; unknown size is a resting track. Restart uses the refresh glyph. Press scale is 0.96; reduced motion disables transitions. The Settings sidebar ends with the same quiet action, while About retains release notes and full controls. Errors keep the action available, say `Update failed`, and retain details in the tooltip and console. Installation is announced through the polite status region.

AI usage settings: one switch per detected installed provider, off by default. Empty detection shows one calm line. Claude and Codex migrate the old opt-in; Antigravity requires a plain-language confirmation before reading its ACP sign-in file and contacting Google. Its model/bucket-scoped allowance uses one ring and the initial A; it never represents a general weekly limit. The service is undocumented and may stop working. Provider initials are used without a sourced official mark.


## Settings and first-run welcome

Settings retain the folded T, Segoe UI Variable, sidebar/search and Windows theme preference. Content fills the remaining width with 32px gutters and a 1120px cap; page titles, group headings and row text share a left edge. Rows start at 44px, or 56px with supporting text, and controls retain the shared 32px height. Group headings are sentence case at one size. Destructive buttons use secondary ink until hover. Antigravity confirmation uses native dialog focus containment, Escape cancellation and the shared theme/control tokens.

Activities combines notification previews and access. About supplies exact allowlisted browser links to GitHub, bugs, MIT and third-party notices. Welcome lives in Settings and moves the actual autostart, detected-provider and model nodes, restoring them on Done/Skip. Local models sort before downloadable models; downloads remain explicit actions with size shown. Startup snapshots native config-directory presence before service initialization and checks pre-existing frontend preferences. An exclusive marker claim precedes opening; unreadable storage, failed claims, fullscreen or busy dictation suppress automatic display. About can reopen welcome. Browser smoke never automatically requests it.

### Baseline audit (both theme captures, browser fixtures)

- General: centred width cap and redundant startup/shape/width descriptions
- Island: loose rows and long HUD/flyout explanations crowd controls
- Media: repeated lyric description and an orphan online-lyrics group
- Activities: separate notification preview/access groups break one task apart
- Dictation and clipboard: dense explanatory paragraphs with loose rows; downloaded models do not lead
- AI usage: ambiguous setup-link button and loose refresh row
- Privacy: introductory interface description; repeated punctuation
- About: lacks project, bug, licence, notices and welcome entry points

Captures are browser fixtures, not live account readings or native hardware verification. Native focus/fullscreen interactions require manual Windows acceptance testing.
