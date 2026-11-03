# Dynamic Island & Notch UI/UX, Motion Physics, and Glanceable Status Architecture for Tuck

This research document provides concrete, production-ready UI/UX, motion physics, and glanceable interaction guidance for **Tuck**—a Windows 10/11 notch utility built with Tauri 2, Vite, and TypeScript. It addresses Tuck's core interface challenge: **replacing dense text buttons with high-density, icon-first glanceable status and tactile micro-interactions** within Tuck's fixed geometry:
* **Idle:** 212 × 32 px
* **Peek:** 300 × 38 px
* **Open:** 440 × 180 px
* **Canvas:** 520 × 230 px transparent WebView2 window

---

## 1. Apple Human Interface Guidelines: Live Activities & Dynamic Island

Apple’s Live Activities and Dynamic Island system establishes the baseline user expectation for compact, fluidly morphing desktop and mobile notches.

### 1.1 Presentation Modes & Layout Topologies

Apple divides Dynamic Island presentations into three functional tiers based on concurrency and user focus:

```
1. COMPACT (Single Activity Active)
+-----------------------------------------------------------+
|  [ Leading (Icon/Art) ]     (Hardware Cutout)     [ Trailing (Metric/Ring) ]  |
+-----------------------------------------------------------+

2. MINIMAL (Two Concurrent Activities Active)
+-------------------------------+       +-----------+
| [ Attached Slot: Icon/Timer ] |  ...  | [ Detached: Ring/Glyph ] |
+-------------------------------+       +-----------+

3. EXPANDED (User Touches & Holds / Opens)
+-----------------------------------------------------------+
|  [ .leading ]               [ Sensor Cutout ]            [ .trailing ]  |
|                             [    .center    ]                           |
|                                                                         |
|                             [    .bottom    ]                           |
+-----------------------------------------------------------+
```

* **Compact Presentation (`compactLeading` & `compactTrailing`)**
  * Displayed when **exactly one** Live Activity is active.
  * The physical hardware housing splits the UI into two complementary zones:
    * **`compactLeading` (Left):** Identifies the entity or context (e.g., 20×20 px album art, app brand glyph, recording dot, or timer icon).
    * **`compactTrailing` (Right):** Displays high-value, dynamic status (e.g., circular countdown progress ring, live equalizer bars, battery level, or flight arrival time).
  * **Text rule:** Text is restricted to 3–4 tabular characters (e.g., `04:59`, `82%`, `+$4`). Continuous sentences or labels are forbidden in compact mode.
  * **Source:** [Apple HIG — Live Activities Presentation Modes](https://developer.apple.com/design/human-interface-guidelines/live-activities#Presentations)

* **Minimal Presentation (`minimal`)**
  * Activated when **two or more** activities run concurrently (e.g., active Media Player + running Pomodoro Timer).
  * The Dynamic Island splits into an **attached element** (anchored to the camera) and a **detached bubble** floating slightly to the right.
  * Each element displays only a single high-signal visual (e.g., a tiny progress ring or a 14×14 px equalizer icon).
  * **Source:** [Apple Developer — ActivityKit Minimal Presentation](https://developer.apple.com/documentation/activitykit/displaying-live-data-with-live-activities)

* **Expanded Presentation (`expanded`)**
  * Appears when the user expands the island (touch & hold on iOS, click or hover-intent on desktop).
  * Structured into four functional regions:
    * **`.leading`:** Context icon, thumbnail art (56×56 px), or primary title.
    * **`.trailing`:** Secondary status, elapsed duration, or dismiss button.
    * **`.center`:** Content placed directly below the top notch (subtitles, secondary indicators).
    * **`.bottom`:** Spans full width; houses interactive controls (media scrubbers, transport buttons, segmented switches).
  * **Source:** [Apple HIG — Live Activities Expanded Region](https://developer.apple.com/design/human-interface-guidelines/live-activities#Expanded-presentation)

### 1.2 Information Architecture & Content Rules

1. **Ongoing Tasks with Deterministic Lifecycles:**
   Live Activities must only represent events with clear start, progress, and end states (e.g., active playback, active Pomodoro session, system hardware status). They must not be used as static launchpads.
   * **Source:** [Apple HIG — Live Activities Design Principles](https://developer.apple.com/design/human-interface-guidelines/live-activities#Best-practices)
2. **Strict Ban on Marketing and Static Text:**
   Promotional copy, static slogans, and generic labels (e.g., "Nothing playing", "Focus mode is ready") are anti-patterns. If no media is playing, the slot should collapse to pure system status (clock, battery) rather than displaying placeholder copy.
3. **Glanceability over Interactivity in Resting States:**
   Compact states are read-only sensors. Interactive buttons belong strictly in the expanded state. Tapping/clicking the resting compact island expands the UI or routes directly to the underlying utility.

---

## 2. Motion & Animation Physics

Dynamic Island motion relies on physical spring mechanics rather than artificial easing curves.

### 2.1 Apple Spring Parameters vs. Fixed Curves

In iOS and SwiftUI, the island's fluid organic expansion uses a damped harmonic oscillator defined by **`response`** ($T$) and **`dampingFraction`** ($\zeta$):

| Interaction State | Perceptual Response ($T$) | Damping Fraction ($\zeta$) | Resulting Behavior |
| :--- | :--- | :--- | :--- |
| **Open / Expansion** | `0.38s` – `0.45s` | `0.78` – `0.84` | Snappy, fluid acceleration with a 3–6% subtle physical overshoot before settling. |
| **Close / Collapse** | `0.30s` – `0.35s` | `0.92` – `0.98` | Critically damped, rapid contraction with zero bounce to prevent visual jitter. |
| **Content Scale-In** | `0.32s` – `0.36s` | `0.75` – `0.80` | Inner cards pop smoothly into place with tactile spring responsiveness. |

* **Sources:**
  * [Apple WWDC 2018: Designing Fluid Interfaces (Session 803)](https://developer.apple.com/videos/play/wwdc2018/803/)
  * [Apple WWDC 2023: Animate with Springs (Session 10158)](https://developer.apple.com/videos/play/wwdc2023/10158/)
  * [SwiftUI Spring Documentation](https://developer.apple.com/documentation/swiftui/animation/spring(response:dampingfraction:blendduration:))

### 2.2 Mathematical Conversion to CSS `linear()`

CSS `linear(...)` allows developers to define multi-point spring trajectories calculated from differential equations ($m \ddot{x} + c \dot{x} + k x = 0$):

$$\omega_0 = \frac{2\pi}{T}, \quad k = \omega_0^2, \quad c = 2\zeta\omega_0, \quad \omega_d = \omega_0\sqrt{1 - \zeta^2}$$

#### Tuck-Calibrated Spring CSS Snippets

##### Open / Peek Spring (`560ms` duration, $T = 0.44s$, $\zeta = 0.80$)
Replaces Tuck's current `cubic-bezier(.32, 1.28, .4, 1)` (which causes an unnatural 28% ballooning overshoot) with an authentic 4.2% iOS physical spring:

```css
:root {
  /* Open / Peek Morph Spring: 560ms settling time */
  --spring-open: linear(
    0, 0.024, 0.0851, 0.1694, 0.2663, 0.3676, 0.4674, 0.5617,
    0.6477, 0.7241, 0.7903, 0.8462, 0.8925, 0.9299, 0.9593, 0.9818,
    0.9985, 1.0102, 1.018, 1.0227, 1.0249, 1.0254, 1.0245, 1.0228,
    1.0206, 1.0181, 1.0155, 1.013, 1.0107, 1.0085, 1.0066, 1.001, 1
  );

  /* Close / Contract Spring: 340ms settling time (critically damped) */
  --spring-close: linear(
    0, 0.0164, 0.0584, 0.117, 0.1856, 0.2589, 0.3334, 0.4063,
    0.476, 0.5411, 0.601, 0.6555, 0.7044, 0.7479, 0.7863, 0.8199,
    0.849, 0.8742, 0.8957, 0.9141, 0.9297, 0.9428, 0.9538, 0.9629,
    0.9705, 0.9767, 0.9818, 0.9859, 0.9893, 0.992, 0.9941, 1
  );

  /* Inner Content Pop: 280ms snappy reveal */
  --spring-pop: linear(
    0, 0.038, 0.143, 0.284, 0.441, 0.598, 0.738, 0.852,
    0.935, 0.989, 1.018, 1.029, 1.028, 1.021, 1.012, 1.005, 1
  );
}
```

* **Sources:**
  * [Jake Archibald's Linear Easing Generator](https://linear-easing-generator.netlify.app/)
  * [CSS Spring Easing Generator (kvin.me/css-springs)](https://kvin.me/css-springs/)

### 2.3 Morphing Orchestration & Content Decoupling

When the Dynamic Island transitions, the content does **not** scale with the container's bounding box:
1. **Asymmetric Cross-Fade & Scale:**
   * **Exiting Content (Compact):** Disappears rapidly. Fades `opacity: 1 -> 0` and scales `scale(1) -> scale(0.8)` within the first **80ms – 120ms** (first 25% of morph).
   * **Container Resizing:** The shell morphs its `width`, `height`, and `border-radius` simultaneously across 560ms.
   * **Entering Content (Full):** Remains hidden until the shell reaches ~65% of its target dimensions (approx. 200ms into the transition). It then springs into view: `opacity: 0 -> 1` and `transform: scale(0.92) -> scale(1)` across 280ms.
2. **Liquid Merging / Gooey Detachment:**
   Mac and iOS apps achieve liquid droplet separation between the notch and secondary detached pills by applying an SVG `feColorMatrix` threshold filter or blur-contrast layer (`blur(8px) contrast(20)`), though pure CSS transform separation is preferred on Windows to preserve GPU performance.
   * **Source:** [CSS-Tricks: The Gooey Effect](https://css-tricks.com/gooey-effect/)

### 2.4 macOS & Windows Notch Ecosystem Comparison

| Project | Platform / Architecture | Motion & Transition Model | Interaction Features |
| :--- | :--- | :--- | :--- |
| **boring.notch** ([GitHub](https://github.com/TheBoredTeam/boring.notch)) | macOS (Swift / AppKit) | Native Swift springs; audio visualizer morphs out of notch; hover delay (~150ms). | Drop-shelf, AirDrop drop-target, webcam mirror, live audio visualizer. |
| **NotchNook** ([Site](https://getnotchnook.com)) | macOS (Swift) | Interactive 2-finger swipe down to expand; modular widget tabs; spring expansion. | Media controls with scrubbing, file drop tray, widget carousels. |
| **Alcove** ([Site](https://getseam.app)) | macOS (Swift / Metal) | Ultra-lightweight native animations mirroring iOS 18 Dynamic Island 1:1. | HUD replacement (volume/brightness), Now Playing strip, meeting glance. |
| **DynamicWin** ([GitHub](https://github.com/FlorianButz/DynamicWin)) | Windows (WPF / C#) | XAML DoubleAnimation springs; top-screen notch dock. | Media controls, Google Calendar, battery status, drag clipboard. |
| **PILLAR** ([GitHub](https://github.com/warpirate/pillar-dynamic-island-for-windows)) | Windows (Tauri / Rust / React) | Framer Motion springs inside transparent web canvas. | Media player, focus timer, hardware monitors. |

### 2.5 Eliminating Animation Jank in Transparent WebView2 on Windows

Transparent windows in WebView2 require special care to prevent dropped frames and DWM composition stalls:

1. **Keep HWND Native Bounds Static:**
   * Resizing the Win32 `HWND` via Tauri IPC (`appWindow.setSize(...)`) during an animation forces continuous DWM swapchain reallocation, resulting in severe visual tearing.
   * **Tuck's Architecture is Correct:** Tuck reserves a static canvas of `520 × 230 px` and animates only the internal DOM element (`.island`), eliminating host window resize overhead.
2. **Eliminate Expensive CSS Filters during Transitions:**
   * In `styles.css`, Tuck currently applies `filter: blur(8px);` on `.full`. On Windows WebView2 with per-pixel alpha (`Color.Transparent`), real-time CPU/GPU Gaussian blurs across transparent surfaces trigger substantial rasterization penalties on integrated Intel/AMD GPUs.
   * **Recommendation:** Replace `filter: blur(8px)` with composite-only transitions (`opacity` + `transform: scale(0.92)`).
3. **Compositor Thread Isolation:**
   Ensure animated elements reside on dedicated GPU layers:
   ```css
   .island {
     will-change: width, height, border-radius;
     transform: translateX(-50%) translateZ(0);
     backface-visibility: hidden;
     contain: layout style;
   }
   .compact, .full {
     contain: strict;
   }
   ```
4. **Avoid Text Layout Thrashing:**
   Do not recalculate text reflow during the width expansion. Position `.compact` and `.full` absolutely with fixed coordinates so internal inline text elements do not trigger intermediate layout passes.
   * **Source:** [Microsoft Edge WebView2 Performance Best Practices](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/win32-api-best-practices)

---

## 3. Icon-First Glanceable Status Architecture

### 3.1 The Problem in Tuck's Current UI

In Tuck's current implementation, micro-spaces are overcrowded with verbose text strings and large labeled buttons:
* **Live Activities (Page 2):** Contains six text buttons: `[5m]` `[15m]` `[25m Focus]` `[Stopwatch]` `[Pause]` `[Reset]`. These consume immense horizontal space, force awkward wrapping, and look like a web form.
* **Quick Actions (Page 4):** Labeled buttons with icon + text chips (`[Wi-Fi]`, `[Bluetooth]`, `[Focus]`, `[Snip]`).
* **Media & Compact Status:** Idle states display redundant string fallbacks ("Nothing playing", "AC power", "Play something in any app") instead of clean, communicative glyphs.

### 3.2 High-Signal Visual Status Components

```
+-------------------------------------------------------------------------------+
| ICON-FIRST TRANSFORMATION (Idle / Peek / Open)                                |
+-------------------------------------------------------------------------------+
| COMPACT IDLE (212x32):                                                        |
|   [ 🎵 20px Art ]  --- [ 04:59 ] --- [ 🔋 82% ⚡ / 3-bar EQ ]                 |
+-------------------------------------------------------------------------------+
| OPEN ACTIVITIES (440x180):                                                    |
|   [ 🍅 Pomodoro ] ----------------------------------- [ ⏱ 24:32 remaining ]   |
|   [ ( 24:32 ) SVG Ring ]     [ ⏯ Play ]   [ ⏹ Stop ]   [ 🔄 Reset ]           |
|   Presets: [ (5) ] [ (15) ] [ (25) ] [ (45) ] (Circular Icon Chips)           |
+-------------------------------------------------------------------------------+
```

#### 1. Circular SVG Progress Rings (Timers & Batteries)
Instead of textual percentage bars or elapsed text, a 20×20 px or 24×24 px SVG circle with dynamic `stroke-dashoffset` communicates proportional time/charge instantly in zero horizontal text space:

```html
<svg class="progress-ring" width="22" height="22" viewBox="0 0 24 24" aria-hidden="true">
  <circle class="ring-bg" cx="12" cy="12" r="10" stroke="rgba(255,255,255,0.15)" stroke-width="2.5" fill="none" />
  <circle class="ring-fill" cx="12" cy="12" r="10" stroke="var(--ok)" stroke-width="2.5" fill="none"
          stroke-linecap="round" stroke-dasharray="62.83" stroke-dashoffset="15.7"
          transform="rotate(-90 12 12)" />
</svg>
```

#### 2. Waveform & Equalizer Bars
Replaces "Playing / Paused" text status. Three to four vertical bars bounce dynamically to indicate audio output. When paused, the bars smoothly collapse to equal 2px dots.
* **Size:** 14 × 12 px
* **Color:** Dynamically inherits `--accent` extracted from artwork.

#### 3. Album-Art Dynamic Ambient Glow (`--tint`)
Extracting the dominant saturated color from active media artwork and projecting a subtle radial back-tint (`radial-gradient(120% 100% at 50% 0%, var(--tint) 0%, transparent 80%)`) eliminates the need for colored label banners, instantly signaling the active media source.

#### 4. Battery & Power Capsule
Replaces "AC power" / "Battery 100%" strings. An inline 22×12 px battery capsule with internal level fill (`#ffffff`), turn-green on AC connect (`#30d158`), and a centered lightning glyph (`⚡`).

#### 5. Preset Duration Icon Badges
Replaces text buttons `[5m]` `[15m]` `[25m Focus]`. Instead, use compact 36×36 px circular pill buttons with an icon glyph (e.g., coffee cup for 5m break, tomato for 25m focus) with a tiny tabular duration sub-label underneath.

### 3.3 Micro-Gestures & Interaction Patterns

1. **Hover-to-Peek (Desktop Dynamic Hover Intent):**
   * Immediate hover reactions cause accidental triggers when moving the mouse to browser tabs.
   * **Pattern:** Introduce a **180ms hover intent delay** before transitioning from Idle (`212×32`) to Peek (`300×38`). If the pointer exits before 180ms, cancel the timer.
   * Mouse leave transitions back to Idle with an immediate 200ms debounce.
2. **Scroll-to-Adjust Volume / Brightness:**
   * When hovering over the Idle or Peek island during active audio playback, rolling the mouse wheel (`wheel` event) increments/decrements system master volume in 2% steps.
   * A micro-pill pops out showing a horizontal volume slider HUD that dismisses 800ms after scrolling stops.
   * **Source:** [Boring Notch Volume HUD Interaction](https://github.com/TheBoredTeam/boring.notch)
3. **Swipe / Wheel-Tilt to Switch Pages:**
   * In Open state (`440×180`), horizontal two-finger trackpad swipe or mouse wheel tilt (`wheel` deltaX) switches between pages (Media, Today/Battery, Activities, AI Usage, Quick Actions).
4. **Long-Press / Hold Gesture:**
   * Clicking and holding for 400ms immediately opens the expanded island without requiring a click on a secondary expand chevron.

### 3.4 Feature-by-Feature Icon Recommendations for Tuck

| Feature / Domain | Current Text UI | Recommended Icon-First Component | Visual States & Feedback | Hit Target Spec |
| :--- | :--- | :--- | :--- | :--- |
| **Media Transport** | Text title & artist scroll | 20×20 px mini-art + 3-bar animated equalizer | Bars bounce during playback; collapse to flat line when paused; art pulses on track change. | 36×36 px (prev/next), 44×44 px (play/pause) |
| **Volume Control** | Hidden / text sliders | Speaker glyph with 3 acoustic arc rays (`volume-2`) | Rays toggle dynamically: 0 rays (mute), 1 ray (<33%), 2 rays (<66%), 3 rays (>66%). | Mouse-wheel scroll target: entire notch surface |
| **Pomodoro / Timer** | `[5m]` `[15m]` `[25m Focus]` `[Stopwatch]` | Circular progress ring + tabular digits + preset glyph chips | Progress ring depletes clockwise; amber tick marks for breaks, emerald for focus. | 36×36 px circular chips |
| **Timer Actions** | `[Pause]` `[Reset]` text buttons | Circular icon buttons: `Play/Pause` (⏯) and `Reset` (↺) | Hover background shift (`--raise-hover`); active scale transform `scale(0.94)`. | 36×36 px minimum |
| **Battery & Power** | `100%`, `AC power` text labels | Mini battery capsule (22×11 px) with internal fill & bolt | Green fill on AC power (`#30d158`), red when <20% (`#ff453a`), lightning bolt when charging. | Glanceable / non-interactive in idle; tap opens power settings |
| **AI Usage Meters** | Plain percentage & model names | Provider icon glyphs (OpenAI, Anthropic, Ollama) + horizontal micro-gauges | Color changes: Green (<50%), Yellow (50-80%), Red (>80% quota); tabular token counters. | Full row tap to refresh |
| **Quick Actions** | `[Wi-Fi]` `[Bluetooth]` `[Focus]` `[Snip]` chips | 40×40 px rounded icon tiles (Lucide `wifi`, `bluetooth`, `moon`, `scissors`) | Active state turns tile accent solid white with dark ink; inactive is raised surface (`#1c1c1e`). | 44×44 px touch/click target |

* **Icon Glyphs Standard:** Compatible with Lucide Icons and Windows Fluent System Icons.
  * **Sources:** [Lucide Icons](https://lucide.dev/icons/), [Microsoft Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons)

---

## 4. Layout Grids for Tuck's Exact Screen Dimensions

### 4.1 Idle State (212 × 32 px)

The resting state must balance high context density with zero visual noise.

```
+-----------------------------------------------------------+
| [ Leading: 28px ]        [ Center: 1fr ]       [ Trailing: 48px ] |
| (20x20 Art/Icon)        (Glanceable Metric)     (Eq / Ring / Batt) |
+-----------------------------------------------------------+
```

```css
.island[data-state="idle"] .compact {
  display: grid;
  grid-template-columns: 28px 1fr 48px;
  align-items: center;
  padding: 0 10px;
  height: 32px;
  column-gap: 6px;
}

.island[data-state="idle"] .cl {
  display: flex;
  align-items: center;
  justify-content: flex-start;
}

.island[data-state="idle"] .cc {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.island[data-state="idle"] .cr {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 4px;
}
```

* **Slot Allocation:**
  * **`cl` (Leading):** 20×20 px rounded album artwork (6px border-radius) or 16×16 px Activity Icon (Timer glyph / AI processing pip).
  * **`cc` (Center):** Tabular time (`14:20`), remaining Pomodoro countdown (`24:18`), or concise track title (overflow marquee only when hovered).
  * **`cr` (Trailing):** Battery capsule with charge bolt OR 3-bar animated audio equalizer.

---

### 4.2 Peek State (300 × 38 px)

Triggered on hover intent or incoming notification. Provides expanded glanceable context before a full modal open.

```
+-----------------------------------------------------------------+
| [ Leading: 36px ]            [ Center: 1fr ]          [ Trailing: 72px ] |
| (24x24 Art + Play/Pause)     (Artist - Title)         (Time + Eq / Batt) |
+-----------------------------------------------------------------+
```

```css
.island[data-state="peek"] .compact {
  display: grid;
  grid-template-columns: 36px 1fr 76px;
  align-items: center;
  padding: 0 12px;
  height: 38px;
  column-gap: 8px;
}

.island[data-state="peek"] .mini-art {
  width: 26px;
  height: 26px;
  border-radius: 7px;
}

.island[data-state="peek"] .cc {
  font-size: 12px;
  font-weight: 500;
  color: var(--ink);
}

.island[data-state="peek"] .cr {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  font-size: 11px;
  color: var(--ink-2);
}
```

---

### 4.3 Open State (440 × 180 px)

The expanded modal card uses a 3-tier vertical grid dividing Header, Interactive Stage, and Footer/Pagination.

```
+-----------------------------------------------------------------+
| HEADER (24px): [ Date / Status Dot ]             [ Battery / Pin / Close ] |
+-----------------------------------------------------------------+
|                                                                 |
| STAGE (112px):                                                  |
|   [ Left Column: 80px ]          [ Right Column: 310px ]        |
|   (56x56 Album Art               (Title, Artist, Scrubber Bar,  |
|    or Big Circular Timer)         Transport Icon Controls)      |
|                                                                 |
+-----------------------------------------------------------------+
| FOOTER (20px): [ Prev Page ]    [ Dots: • • • • • ]    [ Next Page ] |
+-----------------------------------------------------------------+
```

```css
.island[data-state="open"] .full {
  display: grid;
  grid-template-rows: 24px 114px 22px;
  padding: 10px 16px 8px;
  height: 180px;
  width: 440px;
  box-sizing: border-box;
}

/* Stage Area: Two-column media / activity layout */
.page.media {
  display: grid;
  grid-template-columns: 68px 1fr;
  grid-template-rows: auto auto auto;
  align-items: center;
  gap: 8px 14px;
}

.page.media .art {
  grid-row: 1 / span 3;
  width: 64px;
  height: 64px;
  border-radius: 12px;
}

.page.media .meta {
  grid-column: 2;
  display: flex;
  flex-direction: column;
}

.page.media .bar {
  grid-column: 2;
  display: flex;
  align-items: center;
  gap: 8px;
}

.page.media .ctl {
  grid-column: 2;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 20px;
}
```

---

## 5. Accessibility in Micro-UIs

Designing for tiny screen boundaries presents strict accessibility hurdles that must be engineered into the DOM and CSS architecture.

### 5.1 Hit Targets: Visual Compactness vs. WCAG Compliance

* **WCAG 2.2 Success Criterion 2.5.8 (Target Size Minimum - Level AA):**
  Requires interactive pointer targets to measure at least **24 × 24 CSS pixels**, or provide sufficient perimeter spacing so a 24px diameter circle centered on the target does not intersect other targets.
  * **Source:** [W3C WCAG 2.5.8 Target Size (Minimum)](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html)
* **WCAG Success Criterion 2.5.5 (Target Size Enhanced - Level AAA):**
  Requires interactive targets to measure at least **44 × 44 CSS pixels**.
  * **Source:** [W3C WCAG 2.5.5 Target Size (Enhanced)](https://www.w3.org/WAI/WCAG22/Understanding/target-size-enhanced.html)

#### The Micro-Hit-Target Expansion Pattern
In compact UIs, rendering a visual button at 44px breaks the aesthetic. Tuck resolves this by decoupling the **visual icon size** (16–22px) from the **interactive bounding box** (36–44px) using CSS absolute pseudo-elements:

```css
/* Visual icon button remains sleek and tiny */
.mini-action-btn {
  position: relative;
  width: 22px;
  height: 22px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

/* Invisible hit target expands to 38x38px to satisfy WCAG AA */
.mini-action-btn::after {
  content: "";
  position: absolute;
  inset: -8px; /* Expands hit target 8px in all directions: 22 + 16 = 38px */
  cursor: pointer;
}

/* Main open/play control satisfies WCAG AAA (44x44px) */
.ctl button.big {
  width: 44px;
  height: 44px;
  border-radius: 50%;
}
```

### 5.2 Motion Sensitivity & `prefers-reduced-motion`

In accordance with **WCAG 2.3.3 (Animation from Interactions - Level AAA)**, animations must be disableable for users susceptible to motion sickness, vestibular dysfunction, or vertigo:

```css
@media (prefers-reduced-motion: reduce) {
  .island {
    transition: none !important;
  }
  .compact, .full {
    transition: opacity 0.08s ease-in-out !important;
    filter: none !important;
    transform: none !important;
  }
  .eq i {
    animation: none !important;
    height: 6px !important; /* Static paused indicator */
  }
  .title.scroll span {
    animation: none !important;
  }
}
```
* **Source:** [W3C WCAG 2.3.3 Animation from Interactions](https://www.w3.org/WAI/WCAG22/Understanding/animation-from-interactions.html)

### 5.3 Tooltips vs. Screen Reader Labels (WCAG 1.4.13)

When eliminating text labels in favor of icons:
1. **Mandatory Screen Reader Accessibility (`aria-label`):**
   Every icon-only control must possess an unambiguous `aria-label` describing the specific action and current state (e.g., `aria-label="Play Track"`, `aria-label="Reset Timer to Zero"`).
2. **Visual Tooltips (WCAG 1.4.13 Content on Hover or Focus):**
   * **Dismissible:** The tooltip must be dismissible by hitting <kbd>Esc</kbd> without moving the cursor.
   * **Hoverable:** The pointer must be capable of moving over the tooltip without it disappearing.
   * **Persistent:** Must remain visible until hover or focus is removed.
   * **Debounce:** Micro-tooltips must have a **400ms display delay** to prevent visual noise during fast mouse traversal across the notch.
   * **Source:** [W3C WCAG 1.4.13 Content on Hover or Focus](https://www.w3.org/WAI/WCAG22/Understanding/content-on-hover-or-focus.html)
3. **Live State Announcements:**
   Tuck’s existing `#statusMessage` polite live region (`role="status" aria-live="polite"`) must be updated whenever activities change states (e.g., "Timer started: 25 minutes remaining").

---

## 6. Implementation Architecture & Code Snippets

### 6.1 Complete CSS Motion Variables & GPU Rules

Add to `C:\Users\Jojo\Desktop\Projects\tuck\src\styles.css`:

```css
:root {
  /* Apple-calibrated spring transitions */
  --spring-morph: linear(
    0, 0.024, 0.0851, 0.1694, 0.2663, 0.3676, 0.4674, 0.5617,
    0.6477, 0.7241, 0.7903, 0.8462, 0.8925, 0.9299, 0.9593, 0.9818,
    0.9985, 1.0102, 1.018, 1.0227, 1.0249, 1.0254, 1.0245, 1.0228,
    1.0206, 1.0181, 1.0155, 1.013, 1.0107, 1.0085, 1.0066, 1
  );
  --spring-contract: linear(
    0, 0.0164, 0.0584, 0.117, 0.1856, 0.2589, 0.3334, 0.4063,
    0.476, 0.5411, 0.601, 0.6555, 0.7044, 0.7479, 0.7863, 0.8199,
    0.849, 0.8742, 0.8957, 0.9141, 0.9297, 0.9428, 0.9538, 0.9629,
    0.9705, 0.9767, 0.9818, 0.9859, 0.9893, 0.992, 0.9941, 1
  );
  --spring-enter: linear(
    0, 0.038, 0.143, 0.284, 0.441, 0.598, 0.738, 0.852,
    0.935, 0.989, 1.018, 1.029, 1.028, 1.021, 1.012, 1.005, 1
  );
}

/* Island Container Morphing */
.island[data-state="idle"] {
  --w: 212px;
  --h: 32px;
  --r: 16px;
  transition:
    width 340ms var(--spring-contract),
    height 340ms var(--spring-contract),
    border-radius 340ms var(--spring-contract);
}

.island[data-state="peek"] {
  --w: 300px;
  --h: 38px;
  --r: 19px;
  transition:
    width 560ms var(--spring-morph),
    height 560ms var(--spring-morph),
    border-radius 560ms var(--spring-morph);
}

.island[data-state="open"] {
  --w: 440px;
  --h: 180px;
  --r: 34px;
  transition:
    width 560ms var(--spring-morph),
    height 560ms var(--spring-morph),
    border-radius 560ms var(--spring-morph);
}

/* Decoupled Content Reveal (Zero Blur Overhead for WebView2) */
.full {
  position: absolute;
  inset: 0;
  opacity: 0;
  transform: scale(0.92);
  pointer-events: none;
  transition: opacity 100ms ease-out, transform 100ms ease-out;
}

.island[data-state="open"] .full {
  opacity: 1;
  transform: scale(1);
  pointer-events: auto;
  transition:
    opacity 220ms 160ms ease-out,
    transform 320ms 160ms var(--spring-enter);
}
```

### 6.2 Icon-First Pomodoro & Activity Card HTML

Replaces the 6 text buttons in `index.html` with SVG progress and circular action buttons:

```html
<section class="page activities" data-page="activities" aria-label="Timer and Pomodoro">
  <div class="act-header">
    <div class="act-title-row">
      <span class="act-badge">Pomodoro</span>
      <span class="act-status-pill active" id="actStatus">Focusing</span>
    </div>
    <span class="act-target-time" id="actTarget">Ends at 15:45</span>
  </div>

  <div class="act-stage">
    <!-- Circular Progress Graphic -->
    <div class="act-dial">
      <svg class="dial-svg" width="72" height="72" viewBox="0 0 72 72" aria-hidden="true">
        <circle cx="36" cy="36" r="31" class="dial-bg" />
        <circle cx="36" cy="36" r="31" class="dial-fill" id="actDialFill"
                stroke-dasharray="194.7" stroke-dashoffset="48.6" />
      </svg>
      <div class="act-digits" id="actDigits">24:18</div>
    </div>

    <!-- Quick Preset Chips (Icon Badges) -->
    <div class="act-presets" role="radiogroup" aria-label="Timer Duration Presets">
      <button class="preset-chip active" data-time="25" aria-label="25 minutes focus">
        <span class="preset-glyph">🍅</span>
        <span class="preset-label">25m</span>
      </button>
      <button class="preset-chip" data-time="5" aria-label="5 minutes short break">
        <span class="preset-glyph">☕</span>
        <span class="preset-label">5m</span>
      </button>
      <button class="preset-chip" data-time="15" aria-label="15 minutes long break">
        <span class="preset-glyph">🌿</span>
        <span class="preset-label">15m</span>
      </button>
    </div>
  </div>

  <!-- Primary Transport Actions (WCAG Compliant Hit Targets) -->
  <div class="act-action-row">
    <button class="act-round-btn" id="actReset" aria-label="Reset Timer">
      <svg class="ico" viewBox="0 0 24 24"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg>
    </button>
    <button class="act-round-btn big primary" id="actToggle" aria-label="Pause Timer">
      <svg class="ico" viewBox="0 0 24 24"><rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/></svg>
    </button>
    <button class="act-round-btn" id="actStopwatch" aria-label="Switch to Stopwatch">
      <svg class="ico" viewBox="0 0 24 24"><circle cx="12" cy="14" r="8"/><path d="M12 2v4"/><path d="M12 10v4l2 2"/></svg>
    </button>
  </div>
</section>
```

---

## 7. Canonical References & Sources

1. **Apple Human Interface Guidelines & APIs:**
   * [Apple HIG — Live Activities](https://developer.apple.com/design/human-interface-guidelines/live-activities)
   * [Apple HIG — Widgets](https://developer.apple.com/design/human-interface-guidelines/widgets)
   * [Apple Developer — ActivityKit Documentation](https://developer.apple.com/documentation/activitykit)
   * [WWDC 2018 Session 803: Designing Fluid Interfaces](https://developer.apple.com/videos/play/wwdc2018/803/)
   * [WWDC 2023 Session 10158: Animate with Springs](https://developer.apple.com/videos/play/wwdc2023/10158/)
2. **Motion, Physics & CSS Easing:**
   * [Jake Archibald's Linear Easing Generator](https://linear-easing-generator.netlify.app/)
   * [CSS Spring Easing Generator (kvin.me/css-springs)](https://kvin.me/css-springs/)
   * [MDN Web Docs: linear() easing function](https://developer.mozilla.org/en-US/docs/Web/CSS/easing-function/linear)
   * [Josh W. Comeau: An Interactive Guide to Keyframe & Spring Animations](https://www.joshwcomeau.com/animation/css-transitions/)
3. **Notch & Dynamic Island Implementations:**
   * [Boring Notch (macOS Open Source)](https://github.com/TheBoredTeam/boring.notch)
   * [NotchNook (macOS)](https://getnotchnook.com)
   * [Alcove (macOS)](https://getseam.app)
   * [DynamicWin (Windows Open Source)](https://github.com/FlorianButz/DynamicWin)
   * [PILLAR (Windows Tauri/React Dynamic Island)](https://github.com/warpirate/pillar-dynamic-island-for-windows)
4. **Web Platform & Windows Performance:**
   * [Microsoft Edge WebView2 Best Practices](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/win32-api-best-practices)
   * [web.dev: High Performance Animations](https://web.dev/articles/animations-guide)
   * [CSS-Tricks: The Gooey Effect](https://css-tricks.com/gooey-effect/)
5. **Accessibility Standards:**
   * [W3C WCAG 2.2 Criterion 2.5.8: Target Size (Minimum)](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html)
   * [W3C WCAG 2.1 Criterion 2.5.5: Target Size (Enhanced)](https://www.w3.org/WAI/WCAG22/Understanding/target-size-enhanced.html)
   * [W3C WCAG 2.1 Criterion 1.4.13: Content on Hover or Focus](https://www.w3.org/WAI/WCAG22/Understanding/content-on-hover-or-focus.html)
   * [W3C WCAG 2.1 Criterion 2.3.3: Animation from Interactions](https://www.w3.org/WAI/WCAG22/Understanding/animation-from-interactions.html)
   * [Lucide Icons Library](https://lucide.dev/icons/)
