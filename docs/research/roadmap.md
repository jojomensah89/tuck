# Island redesign roadmap

Combined from `competitor-landscape.md`, `island-ux-motion.md`, `how-notch-apps-work.md` and `ui-audit.md`, plus the owner's decisions from 2026-11-03.

## Decisions

- Idle: glyphs only. Time, battery/charging glyph, live-activity dot. No text labels.
- Open: icon tab rail with 5 tabs; wheel, swipe and arrow keys also switch. Prev/next arrows and page dots go.
  - Media: now playing, lyrics, visualizer
  - Today: clock, battery, calendar, timer
  - Tools: quick toggles, clipboard, file shelf, dictation
  - Usage: AI meters, system stats
  - Settings (opens the settings window)
- Wheel: over the idle pill it changes system volume and shows the HUD; over the open island it switches tabs.
- Motion: spring easing with a 3-6% overshoot (about 0.4s), via CSS `linear()`. Outgoing content fades/scales out in the first ~100ms; incoming content enters once the shell is about 65% open. No `filter: blur` on the shell. Reduced motion calms animations rather than hiding content.
- Icons over text: progress rings for usage, battery and timers; status dots instead of status pills; SVG glyphs, never emoji.
- Volume/brightness HUD: island HUD always on; hiding the Windows flyout is an opt-in, off-by-default setting that disables itself if it fails.
- Out of scope: novelty extras (JSON tools, Discord/OBS, mascots), and reading other apps' credential files.

## PR stack

Frontend round (each PR stacks on the previous one):

1. Design tokens and motion system. Spacing/radius/type/icon/duration tokens, spring easings, transform/clip based shell morph, staggered content enter/exit, page cross-fade with exit, diffed compact rendering (no `innerHTML` churn), remove duplicated CSS, bring `DESIGN.md` in line with the code.
2. Idle and peek. Compact leading/trailing slots, glyph-only idle, live-activity dot, one consistent time format, no animation when nothing plays.
3. Open state navigation. 5-tab icon rail, page grouping, swipe with velocity, remove arrows/dots.
4. Icon-first page content. Rings for usage/battery/timer, timer ring with gestures, icon buttons for clipboard, dictation, files, radios; drop trailing periods and stray weights.

Native round (in order):

5. Reliability and performance. Event-driven media/volume/power instead of polling, sleep/resume and display-change handling, TOPMOST re-assertion, fullscreen auto-hide, single-instance guard, monitor selection, idle CPU/RAM budget.
6. Volume/brightness HUD plus wheel-over-idle volume.
7. Device events. Bluetooth connect + battery, charging, mic/camera in use (ConsentStore registry), focus state.
8. Notifications and calendar. Notification mirror with swipe-to-dismiss (needs a spike on unpackaged UserNotificationListener access), next-event countdown.
9. File shelf. Open on system-wide drag (needs a spike: click-through windows get no OLE drag-enter), drag out.
