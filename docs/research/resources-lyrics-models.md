# Resource use, lyrics sources, and dictation models

Research date: 2026-11-04. Method: web research through TinyFish search and fetch, plus a read-only look at this repo. Web pages were treated as data, not instructions. No application code was changed.

Evidence tags:

- **[seen]** Read in a file of this repo, or in a local binary, during this session. Repo line numbers are for `main` plus PR 20 (`ac342a6`); only `dictation.rs` differs between the two.
- **[web]** Read on the cited page during this session.
- **[inferred]** Reasoned from the above or from general Windows/Chromium knowledge. Not confirmed in a source.
- **[unverified]** Claimed by a secondary source, or not checkable here. Test before relying on it.

## Top recommendations

1. **Resources.** The idle cost is mostly *wakeups and IPC into the webview*, not Rust code size. Gate the visualizer, telemetry and the 1 Hz emitters on "island open", emit only on change, and slow the 40 ms hit-test and 20 ms dual-shift threads. Then set `MemoryUsageTargetLevel = Low` while the island is collapsed. Measure first (section 1.5).
2. **Lyrics.** Keep LRCLIB as the only source. Fix the matching, not the source: `/api/get` is only an exact-duration fast path (it needs ±2 s [web]); after a 404 fall back to `/api/search` with a scored match. The owner's case (171 s vs 163 s, `(feat. Jireel)` suffix) is accepted by the scoring in section 2.2. Nusa does the same thing in effect: its binary contains only `https://lrclib.net/api/search?` (section 2.4).
3. **Models.** Replace the single hard-coded `ggml-tiny.bin` with a table-driven catalog (section 3.2: exact URLs, bytes, SHA-256). Offer `base.en-q5_1` (fast), `small-q5_1` (default candidate), `large-v3-turbo-q5_0` (best). `whisper.cpp` cannot load the GGUF files from `handy-computer` (section 3.5). Parakeet is a spike, not a swap: 7x to 10x faster on CPU but English and European languages only, and a new runtime.

---

## 1. Lowest practical system resource use

### 1.1 What the platform gives us

| Fact | Source |
|---|---|
| A WebView2 process group is one browser process, one or more renderers, and helpers such as the GPU and audio processes. The total therefore counts `tuck.exe` plus every `msedgewebview2.exe` child. | [web] https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model |
| `MemoryUsageTargetLevel = Low` is for inactive WebViews that still run scripts. It is best effort and "could potentially cause memory for some WebView browser processes to be swapped out to disk". Nothing resets it to Normal automatically. | [web] https://github.com/MicrosoftEdge/WebView2Feedback/blob/main/specs/MemoryUsageTargetLevel.md |
| `TrySuspend` pauses scripts and further cuts resource use; `Resume` brings them back. It automatically sets the level to Low, and `Resume` sets it to Normal. Pick one pair, do not mix. Setting the level while suspended is ignored. | same spec; also https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/performance |
| Microsoft says not to disable the GPU: "GPU drivers and additional buffers must be allocated" but GPU rendering is critical for performance. Use CSS rather than JS for animation; batch host/web messages because each crossing costs CPU. | [web] performance page above |
| Browser flags are "for diagnosing issues… production apps shouldn't use WebView2 browser flags" because they may change or vanish. | [web] https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/webview-features-flags |
| `--disable-gpu-compositing` and `--disable-direct-composition` move compositing to the CPU path and cured one WebView2 presentation bug on Tauri 2 hosts. This shows they work on a Tauri host, not that they save power. | [web] https://github.com/MicrosoftEdge/WebView2Feedback/issues/5574 |
| Nusa (native Rust, no web engine) advertises about 30 MB and 0% idle CPU. That is a marketing claim, but it is the bar users compare against. | [web] https://rama2555.github.io/nusa/ [unverified] |

### 1.2 Decisions on WebView2 knobs

| Knob | Decision | Why |
|---|---|---|
| `MemoryUsageTargetLevel = Low` when collapsed for N seconds (suggest 30 s), Normal on hover/open | **Do it.** Medium effort. | Scripts keep running, so event listeners and the 1 Hz ticks still work. Cost: the first paint after a long idle can stall while pages swap in [web]. Hook point: `Context::hide_island` in `lifecycle.rs:179` and the hover event. |
| `TrySuspend` while the island is hidden (locked, display off, suspended, fullscreen-hidden) | **Do it only for lock/display-off/sleep.** | Hidden states are `locked || suspended || display_off` (`lifecycle.rs:175`). Suspending stops page timers, so on resume re-emit state (media, power, volume) as `resume()` already does for native services (`lifecycle.rs:318-331`). The API needs the WebView not visible; whether Tauri's `win.hide()` flips the controller's `IsVisible` is [unverified], so call `put_IsVisible(false)` yourself. Reach the COM objects with `WebviewWindow::with_webview` [inferred]. |
| `--disable-gpu` / software compositing | **Do not ship.** Test once, behind an env var. | Fixed-size 520x230 transparent surface, so GPU memory is small to begin with. Software compositing moves animation cost onto the CPU, which is the scarcer resource at idle [inferred]. |
| Other `--disable-features=…` lists | **Skip.** | Unsupported for production [web]. Gains are unmeasured [unverified]. |
| Backdrop effects (Mica/acrylic, `backdrop-filter`) | **Keep off.** | The window is `transparent: true, shadow: false, decorations: false` (`src-tauri/tauri.conf.json:22-24` [seen]). The roadmap already bans `filter: blur` on the shell. Blur re-rasterises behind every frame [inferred]. |
| Rust release profile | **Already fine.** | `Cargo.toml:81-86`: `codegen-units=1`, `lto=true`, `opt-level=3`, `panic="abort"`, `strip=true` [seen]. At idle the process spends its time in wakeups, so codegen settings will not move the numbers. `opt-level="s"` is not worth testing until the wakeups are gone [inferred]. |

### 1.3 CSS and rendering in a transparent window

All [inferred] from the Microsoft guidance above plus how the repo already behaves; verify with the measurement in 1.5.

- No `animation-iteration-count: infinite` and no running `requestAnimationFrame` loop while idle. Stop at rest (DynamicWin does 8 Hz idle, `how-notch-apps-work.md` §2.3 [seen]).
- Animate only `transform` and `opacity`. Do not animate `width`, `height`, `box-shadow` or `filter`.
- A transparent layered window has no backing opaque surface, so every changed pixel region is recomposited by DWM. Keep damage small: avoid full-island repaints from a ticking clock or a dot that pulses.
- Pause visual work when hidden: stop intervals and rAF on island collapse, not just on window hide.
- Prefer one batched event per tick over several (each `emit` crosses process boundaries into the renderer).

### 1.4 Audit of timers, polls and always-on work (file:line)

Wakeup rates are per second. "IPC" means an event emitted into the webview, which wakes the renderer.

| # | Where | What | Rate | Runs when | Fix |
|---|---|---|---|---|---|
| A | `loopback.rs:76` (loop), `loopback.rs:90` (emit), `visualizer.rs:272-276` | WASAPI loopback capture, 512-sample FFT and a `visualizer` event every 33 ms, even for silence | ~30 + 30 IPC | whenever the setting is on, regardless of island state or whether media plays | Run only while island open, Media tab visible and media playing; otherwise drop the stream (`drop(stream)` path exists, `loopback.rs:92`). Skip emit when two frames are identical zero snapshots. **Highest saving.** |
| B | `dictation.rs:664-669` (PR 20) | Dual-shift thread: `sleep(20ms)` then `service.settings()` (a clone, `dictation.rs:241-243`) and `GetAsyncKeyState` | 50 wakeups | forever, even when the shortcut is not dual-shift (`:674-679` just `continue`s) | Park the thread (Condvar/channel) when the setting is not dual-shift. When it is, poll at 33-50 ms and only clone the one field. Or replace with a filtered `WH_KEYBOARD_LL` hook (zero idle cost, but see privacy note in `desktop_signals.rs:1-4` and antivirus flags, section 2.4). |
| C | `system.rs:546` | 40 ms cursor hit-test: `GetCursorPos`, plus `outer_position()`, `outer_size()`, `scale_factor()` on the Tauri window each tick (`system.rs:566-570`) | 25 wakeups | forever | Cache window rect on move/resize events instead of querying per tick (those getters probably round-trip to the main thread [inferred]). Make the period adaptive: 40 ms while hovered or input in the last ~2 s, ~120-150 ms otherwise, using `GetLastInputInfo` (already used at `desktop_signals.rs:816-826`). The cost is up to ~150 ms extra on the first hover after idle; the 0.3 s open delay hides it. |
| D | `desktop_signals.rs:702` | Signals loop, `recv_timeout(250 ms)`: Caps Lock `GetKeyState` (`:774`), foreground keyboard layout (`:779-786`), Wi-Fi every 8th cycle = 2 s (`:788-799`), and `osd_flyout::tick()` (`:701`) | 4 | forever | Wi-Fi: `WlanRegisterNotification` instead of opening a WLAN handle every 2 s (`:619-653`) [inferred API choice]. Layout: re-read on the foreground WinEvent the lifecycle hub already has (`lifecycle.rs:632`). Caps Lock has no documented change event: keep, but only when input age is recent. `osd_flyout::tick` only needs a wake at `lookup_at`; use that as the `recv_timeout`. Brightness and volume are already event-driven (`:415-421`, `:512`) [seen]. |
| E | `telemetry.rs:391` | 1 s loop: `GetSystemTimes`, `GlobalMemoryStatusEx`, `sample_gpus()` and PDH GPU counters, then `emit("telemetry", snapshot)` with 60 history points | 1 + 1 IPC | forever, started unconditionally (`lib.rs:221`) | `sample_gpus()` does `LoadLibraryW("dxgi.dll")`, creates a factory, enumerates adapters and `FreeLibrary` **every second** (`telemetry.rs:280-379`); adapters are static, so cache them. Sample only while the Usage tab is visible (the page already guards on that, `live-ui.ts:336`) or at 5-10 s while closed. The pull command `telemetry_now` exists (`:84`), so the push is optional. |
| F | `extensions.rs:875` | 1 s loop: scans Downloads (`scan_downloads`, `:792-800`), reminders, notifications, then `emit("extension-state", snapshot())` (`:873`) | 1 + 1 IPC | forever | Emit only when the snapshot changed. Replace the directory scan with `ReadDirectoryChangesW`/`notify`, or scan every 5-10 s. |
| G | `usage.rs:808` | `emit("usage", current_snapshot())` every second; remote API only every 600 s, file every 60 s (`:758-770`) | 1 IPC | forever | Emit on change only. |
| H | `installed_usage.rs:437` | 1 s sleep, real work every 300 s (`:410-413`) | 1 | forever | Sleep until the next due time or a `REFRESH` signal (Condvar). |
| I | `clipboard_history.rs:655` | 350 ms sleep loop using the clipboard sequence number | ~3 | forever | `AddClipboardFormatListener` on the lifecycle window, then handle `WM_CLIPBOARDUPDATE` [inferred API choice]. |
| J | `media.rs:633`, `:740` | 30 s safety refresh, otherwise event-driven | 0.03 | - | Fine [seen]. |
| K | `lifecycle.rs` | All event-driven; the 1 s timer is armed only while hidden for fullscreen (`:189-206`) | ~0 | - | Fine [seen]. Model for the rest. |
| L | `main.ts:532` | `setInterval(tick, 200)` for the seek bar | 5 | forever | Run only while media is playing and the island is open. |
| M | `main.ts:869-873` | 1 s interval: `renderToday()`, `renderMediaHeader()`, `renderCompact()` | 1 | forever | Align to the minute unless seconds are on screen; skip while hidden. |
| N | `live-ui.ts:333-338`, `activities.ts:86`, `main.ts:1004`, `main.ts:1339` | 1 s, 5 s (sessions), activity ticker, 30 s, 60 s | ~1.4 | forever | Clear on collapse, restart on open. Chromium throttles timers in hidden pages, but only if the page is actually marked hidden, which is the same `IsVisible` question as in 1.2 [inferred]. |

Loaded models: Whisper is **not** resident. `WhisperContext::new_with_params` runs per recording (`dictation/audio.rs:48`), so idle RAM for dictation is zero and the cost is load latency on each press [seen]. Keep that for tiny/base. For larger models add Handy's pattern: a `model_unload_timeout` of never / immediately / N seconds [web] https://github.com/cjpais/Handy/issues/1841 (also reports a 70 s reload after idle unload on one NVIDIA Vulkan setup, so test any GPU build before turning unload on).

### 1.5 Ranking (expected saving vs effort)

| Rank | Change | Saving | Effort |
|---|---|---|---|
| 1 | Gate visualizer on island-open + playing (A) | Removes ~60 events/s and an FFT thread when enabled | Low |
| 2 | Emit-on-change for F, G, E; stop the per-second DXGI load (E) and downloads scan (F) | ~3 IPC/s plus a directory scan and a DLL load per second | Low |
| 3 | Park or slow the dual-shift thread (B) | 50 -> ~0 wakeups when unused | Low |
| 4 | Adaptive hit-test (C) | 25 -> ~7 wakeups at idle, and fewer cross-thread window calls | Medium |
| 5 | `MemoryUsageTargetLevel = Low` after 30 s collapsed | RAM only; size [unverified] | Medium |
| 6 | JS timers gated on open/playing (L, M, N) | Keeps the renderer asleep | Low |
| 7 | Event-driven Wi-Fi, clipboard, layout (D, I) | Small wakeup savings | Medium |
| 8 | `TrySuspend` on lock/display-off | RAM and CPU while locked | Medium |
| 9 | Browser flags | Unknown, unsupported | Not recommended |

### 1.6 How to measure (do this before and after each change)

Count the whole family: `tuck.exe` plus every `msedgewebview2.exe` descendant. The script walks the process tree (the WebView2 browser process is Tuck's child, the rest are its children) and sums working set, private bytes and CPU. Working set double-counts shared pages across the WebView2 processes, so use private bytes for RAM and treat working set as an upper bound.

```powershell
# footprint.ps1  -  powershell -File footprint.ps1 -Exe tuck -Seconds 120 -IntervalSec 5
param([string]$Exe = 'tuck', [int]$Seconds = 120, [int]$IntervalSec = 5)

function Get-Family([string]$Name) {
    $roots = @(Get-Process -Name $Name -ErrorAction Stop)
    $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId
    $ids = [System.Collections.Generic.HashSet[int]]::new()
    foreach ($r in $roots) { [void]$ids.Add($r.Id) }
    do {
        $grew = $false
        foreach ($p in $all) {
            if ($ids.Contains([int]$p.ParentProcessId) -and $ids.Add([int]$p.ProcessId)) { $grew = $true }
        }
    } while ($grew)
    Get-Process -Id @($ids) -ErrorAction SilentlyContinue
}

$prev = @{}; $rows = @()
$end = (Get-Date).AddSeconds($Seconds)
while ((Get-Date) -lt $end) {
    $family = Get-Family $Exe
    $cpuMs = 0.0
    foreach ($p in $family) {
        $t = $p.TotalProcessorTime.TotalMilliseconds
        if ($prev.ContainsKey($p.Id)) { $cpuMs += [Math]::Max(0, $t - $prev[$p.Id]) }
        $prev[$p.Id] = $t
    }
    $rows += [pscustomobject]@{
        Procs = @($family).Count
        WorkingSetMB = [Math]::Round((($family | Measure-Object WorkingSet64 -Sum).Sum) / 1MB, 1)
        PrivateMB = [Math]::Round((($family | Measure-Object PrivateMemorySize64 -Sum).Sum) / 1MB, 1)
        CpuPctOfOneCore = [Math]::Round($cpuMs / ($IntervalSec * 1000) * 100, 2)
    }
    Start-Sleep -Seconds $IntervalSec
}
$rows = $rows | Select-Object -Skip 1            # first sample has no CPU delta
$med = { param($v) $s = $v | Sort-Object; $s[[int][Math]::Floor(($s.Count - 1) / 2)] }
"samples=$($rows.Count)"
"median processes      : $(& $med ($rows.Procs))"
"median private MB     : $(& $med ($rows.PrivateMB))"
"median working set MB : $(& $med ($rows.WorkingSetMB))  (upper bound)"
"median CPU % of 1 core: $(& $med ($rows.CpuPctOfOneCore))  max: $(($rows.CpuPctOfOneCore | Measure-Object -Maximum).Maximum)"
```

I ran this script against `explorer` to confirm it parses and runs; I did not run it against Tuck. Protocol:

1. Release build, nothing else of note running, same power plan, same display.
2. Three states, 5 minutes each, discard the first minute: (a) collapsed, no media; (b) collapsed, media playing; (c) open on Media with lyrics and visualizer.
3. Record median processes, private MB, median and max CPU %. Compare against the Nusa figures (30 MB, ~0%) only as a direction, not a target Tuck can meet with a web engine.
4. For counts of wakeups, use Windows Performance Recorder with `WebView2.wprp` (named in the Microsoft performance page [web]) or Process Explorer's context-switch delta column. The Browser Task Manager (`OpenTaskManagerWindow`, same page) shows per-renderer CPU and memory.

---

## 2. Lyrics

### 2.1 What LRCLIB documents (https://lrclib.net/docs, fetched 2026-11-04) [web]

| Topic | Rule |
|---|---|
| Access | Free, no key, CORS-enabled. Identify your client with a `User-Agent` of app name, version and a link or email (`X-User-Agent` or `Lrclib-Client` if the header cannot be set). |
| Tuck today | `DEFAULT_USER_AGENT = "Tuck/0.1.0 (https://github.com/jojomensah89/tuck)"` meets this (`lyrics.rs:9`). |
| Rate limits (approximate, "may change at any time") | ~45 req/min per IP for generic clients; ~90/min for an identified `User-Agent`; ~180/min per IP for a registered app using `Lrclib-App-Id`. IPv6 counts per /64. |
| 429 | Must honour `Retry-After` and add jitter up to ~30%; ignoring it "may result in a temporary ban". Tuck honours it (clamped 2-3600 s, `lyrics.rs:336-345`) but adds no jitter. |
| Etiquette | Send requests sequentially; add 200-500 ms between requests in bulk. |
| `GET /api/get` | `track_name` and `artist_name` required; `album_name` recommended; `duration` (1-3600 s) "is crucial": it matches only when the duration is equal or within **±2 s**. 404 `TrackNotFound` otherwise. |
| `GET /api/search` | Needs `q` **or** `track_name`. `q` searches title, artist and album and takes precedence over the structured fields. Max 20 results, no pagination. Experimental. |
| `GET /api/v2/search` (beta) | Same inputs, but returns metadata and `hasSyncedLyrics`/`hasPlainLyrics` flags with no lyrics text; then `GET /api/get/{id}`. Not covered by v1 compatibility promises. |
| Record fields | `id`, `trackName`, `artistName`, `albumName`, `duration`, `instrumental`, `hasWordSync`, `plainLyrics`, `syncedLyrics`, `lyricsfile`. |
| Other | A missing track "can be picked up by the background lyrics fetching service and may become available in a later request" (so cache misses briefly). A flag endpoint exists for wrong lyrics or copyright reports. Registering an app is optional and raises limits. |

**The owner's case, reproduced** [web, 2026-11-04, via TinyFish fetch]:

- `GET /api/search?track_name=Big+Thug+Boys&artist_name=Babyboy+AV` returns three records, all titled `Big Thug Boys (feat. Jireel)`: artist `Babyboy AV` twice (albums `Big Thug Boys (feat. Jireel)` and `Big Thug Boys`) and one with artist `Babyboy AV feat. Jireel`. Durations 163, 164 and 163 s. `q=Big+Thug+Boys+Babyboy+AV` returns the same three.
- `GET /api/get?track_name=Big+Thug+Boys&artist_name=Babyboy+AV&duration=171` returned not found.
- Two things fail at once: the title lacks the `(feat. Jireel)` suffix, and 171 s is 7-8 s from 163-164 s, well outside ±2 s. Fixing only one of them would still 404.

### 2.2 Matching algorithm (specification)

Inputs: `title`, `artist` string, optional `album`, `duration_s` (0 or missing means unknown).

**Step 0 - cache.** Key on `(norm(title_core), qualifiers, norm(primary_artist), round(duration_s) or "unknown")`. Duration must be in the key: an artist can have several songs with one title (LRCLIB has `Intro` by `Drake` at 52, 37, 35, 34, 30 and 25 s), and a player can report the duration late. Negative results expire after 6-24 h, because LRCLIB can gain the track later [web]. Never store a negative result when the duration is unknown, or the correct request that follows it would be answered by that miss. User offsets are stored per track, separately from this cache.

**Step 1 - normalise.**

- `norm(s)`: Unicode NFKC, lowercase, strip combining accents, `&` to `and`, curly to straight quotes, drop punctuation except digits and letters, collapse spaces.
- Split the title into `core` and `qualifiers`. Treat as **ignorable noise** (remove from `core`): `feat.`/`featuring`, and `ft.` inside brackets (move X into the artist set), `remaster(ed) [year]`, `mono`/`stereo`, `explicit`/`clean`, `single`/`album`/`radio`/`original version`, `radio edit`, `deluxe`, `bonus track`, `from "..."`, in either `( )`, `[ ]` or a trailing ` - Remastered 2011` style suffix. `(with X)` and a bare `ft X` are only credits if X is in the artist credits of either record, because `Run (With Me)` and `Six Ft Under` are titles.
- Treat as **identity qualifiers** (keep as a set `Q`; the candidate must have the same `Q`): `live`, `acoustic`, `remix` (and `X remix`), `instrumental`, `karaoke`, `cover`, `sped up`, `slowed`, `reverb`, `demo`, `part N`/`pt. N`, and any other `<word> version` (`Spanish`, `Dub`, `Piano`). A live track must never get the studio lyrics, and vice versa.
- Artists: keep the full string and also split on `,` ` & ` ` and ` ` x ` ` feat. ` ` ft. ` ` with ` `;` `/`. `primary` is the first token. Keep the unsplit form too, because `Simon & Garfunkel`, `Earth, Wind & Fire` and `Tyler, The Creator` contain separators.

**Step 2 - lookup order** (sequential, stop at the first accepted candidate, at most 3 network calls per track, 200-500 ms apart, one shared 429 back-off):

1. `GET /api/get` with the raw title, raw artist, `album_name` if known and `duration=round(duration_s)`. Exact record, ±2 s. If 200, still run the sanity checks in step 3 and accept.
2. On 404, `GET /api/search?track_name={core}&artist_name={primary}` (20 results).
3. If nothing scores ≥ 55, `GET /api/search?q={core} {primary}`.
4. If the title had stripped noise and still no result, repeat 2 with the raw title.

Optional later: use `/api/v2/search` for step 2/3 to avoid downloading 20 lyric bodies, then `GET /api/get/{id}` for the winner. It is beta, so adopt it only if payload size matters.

**Step 3 - score** each candidate out of about 100:

| Signal | Points |
|---|---|
| Title: candidate's own `core` equals ours | 40 |
| Title: anything else | reject (word sets ignore order and repeats: `Money Money Money` is not `Money`) |
| Artist: same credit, ignoring order and guests (e.g. `Babyboy AV feat. Jireel`) | 30 |
| Artist: our `primary` equals any other candidate artist token, including a guest | 30, flagged approximate |
| Artist: candidate string contains a multi-word `primary` | 28, flagged approximate |
| Artist: only partial overlap | 10, flagged approximate |
| Artist: no overlap | reject |
| Duration: \|Δ\| ≤ 2 s | 20 |
| Duration: ≤ 5 s | 14 |
| Duration: ≤ 10 s | 8 |
| Duration: ≤ 15 s | 3 |
| Duration: > 15 s | -20 (reject above 30 s) |
| Duration unknown | 0, and cap the result at "low confidence" |
| Album equal | +5 |
| `Q` sets differ | -30 |
| Has `syncedLyrics` | +5 |
| Last synced timestamp > playing duration + 3 s | -25 |
| Candidate is `instrumental` while others have lyrics | -20 (unless our title says instrumental) |

Decision: **≥ 75 accept**, **55-74 low confidence** (show only if a "loose matching" setting is on, with a subtle marker), **< 55 not found**. Ties: an unflagged match, then smaller \|Δduration\|, then synced over plain.

Worked example (our title `Big Thug Boys`, artist `Babyboy AV`, 171 s) against `Big Thug Boys (feat. Jireel)` / `Babyboy AV` / 163 s: title 40 + artist 30 + duration Δ8 = 8 + synced 5 = **83, accepted**, flagged "approximate" because Δ > 2 s. The 164 s record scores the same at Δ7; the album-`Big Thug Boys` record wins the album point if Spotify reports that album.

**Wrong-song guards** (never accept on duration alone):

- Both the same title and an artist match are required.
- Reject candidates with `karaoke`, `tribute`, `made famous by`, `originally performed by` in the title or artist unless ours has them.
- Titles of one or two short words (`22`, `Home`) require an exact artist match and \|Δ\| ≤ 3 s.
- Reject a synced result whose final timestamp is far beyond the real track length.

**Sync drift when durations differ.**

- Δ ≤ 2 s: use as is.
- 2-15 s: use as is, **do not time-stretch**. Different durations usually mean a different edit (intro or outro trimmed, radio edit, silence), which shifts lines by a constant, not by a ratio. A uniform stretch would move the early lines to the wrong place [inferred; not tested on real audio].
- Give the user a per-track offset (±100 ms steps, stored with the cached `id`). This is the real fix for the 8 s case: the lyrics are right, the timing is shifted.
- Honour the LRC `[offset:]` tag. `parse_lrc` currently discards it (`lyrics.rs:107`) [seen].
- When playback passes the last lyric by more than ~20 s, stop highlighting.

### 2.3 Other sources

| Source | How it works | Account or cookie | Legal / ToS | Afrobeats / Nigerian coverage | Verdict |
|---|---|---|---|---|---|
| **LRCLIB** | Community database, open API | None. Optional app id for higher limits | Hosts user-submitted lyrics with a flag/takedown endpoint [web]. Copyright of the lyrics themselves is grey, as for every non-licensed source [inferred] | The test track exists, but only under a different title and 7-8 s off. No coverage statistics are published [unverified] | **Primary and only** |
| **Musixmatch official API** | REST with a developer key | Developer key (not a user's cookie) | The free tier returns about 30% of the lyric text, per a 3-year-old Stack Overflow answer [web, unverified]; full and synced lyrics are paid or enterprise (https://about.musixmatch.com/business/overview) | Marketed as "the world's largest lyrics catalog" [web, marketing] | Not viable free |
| **Spotify internal** (`spclient.wg.spotify.com/color-lyrics/v2/track/{id}`) | Needs a Spotify track id and a bearer token derived from the `sp_dc` cookie. Lyrics are Musixmatch's (https://support.spotify.com/us/artists/article/managing-your-lyrics-on-spotify/) | **Yes: `sp_dc` cookie** (https://github.com/akashrchandran/spotify-lyrics-api, https://stackoverflow.com/questions/73704499/get-lyrics-data-from-spotify) | That project's own README: "probably against Spotify TOS" [web] | Whatever Musixmatch has | **Excluded:** needs another app's credential, and Windows SMTC does not give Tuck the track id anyway [inferred] |
| **Musixmatch desktop token** (what Spicetify-style tools use) | Reuses a Musixmatch app token | Yes | Not licensed for this [inferred] | - | **Excluded** (credential reuse) |
| **NetEase Cloud Music** | Unofficial web endpoints, no key for search/lyrics | No | Undocumented API of a Chinese service; no public developer terms found [unverified] | Strong for CJK; African coverage unknown [unverified] | Optional later fallback, opt-in, disclosed. Low value for Afrobeats |
| **QQ Music** | Same class as NetEase | No | Same | Same | As above |
| **Kugou, TTPod, Gecimi, Syair** | Listed by LyricsKit | No | Same | Unknown | Skip |
| **Genius** | API returns metadata, not lyric text | Token required | search snippet of https://docs.genius.com/ reads "Genius API is not allowed without a license" (I did not read the surrounding context [unverified]); its ToS prohibits scraping and it sued Google/LyricFind over it (https://advertisinglaw.fkks.com/post/102hklg/how-does-that-song-go-again-lyrics-databases-claims-alleging-scraping-in-viol) | Unsynced only | **No**. Spicetify disabled it as a provider in 1.2.31 (https://github.com/spicetify/cli/blob/main/CustomApps/lyrics-plus/README.md) |
| **Apple Music** (SongSync) | Private API | Token | ToS-violating [inferred] | Unknown | Skip |

What open-source projects do [web]:

- **Spicetify lyrics-plus:** internal Spotify service, Netease, Musixmatch, Genius (disabled) (README above). An LRCLIB source was requested in https://github.com/spicetify/cli/issues/3027; whether it shipped is [unverified].
- **SongSync:** Spotify, Apple Music, Musixmatch, Netease, QQ Music, LRCLib (https://thedocumentation.org/songsync/advanced/lyrics_providers/).
- **LyricsX / LyricsKit:** NetEase, QQ, Kugou, TTPod, Gecimi, Syair (https://github.com/ddddxxx/LyricsKit).
- **SPlayer, foobar2000 lyric plugins, "Nugs":** not confirmed in this pass [unverified]. I could not identify what "Nugs" refers to; `nugs.net` is a live-concert streaming service, not a lyrics source I could find.
- **Nusa and several small Windows apps:** LRCLIB only (section 2.4; https://apps.microsoft.com/detail/9p6lnjcl8zcc lists "Fetches time-synced lyrics from LRCLib").

**Recommendation.** Stay on LRCLIB. The failure the owner saw is a matching bug, not a coverage gap. For Afrobeats, add a local counter of "not found" titles in the Lyrics settings so real coverage becomes visible before anyone adds a second source. If a second source is ever added, NetEase or QQ are the only ones that need no credential; both are grey-area, so make them off by default and say where the title goes.

### 2.4 How Nusa gets its lyrics

**Answer: LRCLIB, and from the binary, only the `/api/search` endpoint.**

- Public statements [web] https://rama2555.github.io/nusa/: "Lyrics come from LRCLIB"; "The only connection is to LRCLIB for song lyrics"; "Nusa sends the song title, artist and length to LRCLIB (lrclib.net)". Version 1.0.0, Rust, about 3 MB installer.
- Binary scan [seen]. `%LOCALAPPDATA%\Programs\Nusa\nusa.exe` (909,312 bytes) was read as bytes only; it was not run or modified. The script lives in the session scratchpad, not in the install folder (`scan_urls.py`: regex over ASCII and UTF-16LE for `http(s)://`, plus keyword counts). Results:
  - ASCII URLs: exactly one, `https://lrclib.net/api/search?`. UTF-16 URLs: none.
  - Adjacent strings: `track_name=`, `&artist_name=`, and `q=`, in the order a Rust `format!`-style query builder would leave them.
  - Zero hits for `api/get`, `syncedLyrics`, `plainLyrics`, `musixmatch`, `netease`, `genius`, `spotify`, `qq.com`, `kugou`, `apple`.
  - The `syncedLyrics`/`plainLyrics` field names being absent probably means the JSON is parsed through derive-generated code whose names the compiler stored differently [inferred]; I did not decompile it.
- Inference: Nusa searches by title and artist (with a `q=` variant, probably a fallback) and never calls the exact-duration `/api/get`. That would explain why it finds `(feat. …)` records that Tuck misses. How it ranks results, which duration tolerance it applies and whether it sends `duration` cannot be told from strings; **that cannot be determined from static strings alone**.
- No `User-Agent` string appears in the binary, but the HTTP library may build it elsewhere, so I draw no conclusion.

---

## 3. Speech models for offline dictation

### 3.1 How Handy does it

Sources: https://handy.computer/docs/models.html, https://github.com/cjpais/Handy, https://github.com/cjpais/transcribe-rs, https://github.com/handy-computer/transcribe.cpp, https://huggingface.co/handy-computer [web].

| Topic | Finding |
|---|---|
| Models offered | Whisper Small (~487 MB), Medium (~492 MB), Large (~1.1 GB), Turbo (~1.6 GB); Parakeet V3 (~478 MB, 25 European languages) and V2 (~473 MB, English); Cohere Transcribe (~1.8 GB); Moonshine (31-192 MB, English); SenseVoice (~160 MB); GigaAM v3 (~225 MB, Russian); Canary 180M Flash / 1B v2 (146 / 692 MB); Breeze ASR (~1.1 GB). The Medium (~492 MB) and Large (~1.1 GB) sizes look quantized, since the f16 files are 1.5 GB and 3.1 GB; the docs do not say which quant [unverified]. |
| Labels | A table per family with Size, Speed (Fastest / Very fast / Fast / Moderate / Slower) and Accuracy (Lower / Good / Better / High / Highest / Best), plus a "Speed vs. Accuracy" cheat sheet. |
| Recommendation | "Parakeet V3 is the recommended model for most users"; Cohere Transcribe for the highest English quality "best suited to machines with a capable GPU"; Whisper Small or Medium for multilingual. This is documentation text. I found no automatic hardware detection that picks a model [unverified]. |
| Default model | Not stated on the page [unverified]. |
| Download UX | Choose a model in a settings dropdown; "new models will be downloaded on first use". Mirrors: `blob.handy.computer` (pre-quantized GGML Whisper and int8 Parakeet) and Hugging Face (https://www.snackonai.com/p/handy-the-most-forkable-speech-to-text-app-is-a-better-design-goal-than-the-most-accurate-one, secondary [unverified]). |
| Storage | `%APPDATA%\com.pais.handy\models\` on Windows (https://github.com/cjpais/Handy/issues/1496). A custom Whisper `.bin` dropped in `models/` is discovered on next launch ("experimental"). |
| Engines | `transcribe-rs` (ONNX models plus whisper.cpp: Parakeet, Canary, Cohere, Moonshine, SenseVoice, GigaAM, Whisper, Whisperfile, OpenAI), moving to `transcribe.cpp`, a ggml/GGUF library (MIT, Rust bindings in `bindings/rust/transcribe-cpp`). GPU: Whisper and Cohere use Metal on macOS and Vulkan on Windows/Linux; "all other models currently run on CPU only" per the docs page. |
| Unloading | A `model_unload_timeout` setting: never / immediately / N seconds (https://github.com/cjpais/Handy/issues/1841, https://github.com/cjpais/Handy/issues/2000). Those issues also record a 70 s reload after unload on one NVIDIA Vulkan setup and memory growth on long audio, so unloading is not free. |

### 3.2 Whisper catalog for Tuck

Source of every byte count and SHA-256: the Hugging Face tree API for `ggerganov/whisper.cpp`, `lfs.oid` (the SHA-256 of the file) and `size`, fetched 2026-11-04 [web] https://huggingface.co/api/models/ggerganov/whisper.cpp/tree/main. As a cross-check, the `ggml-tiny.bin` row matches `MODEL_SHA256` in `dictation/model.rs:10` and the 77,691,713-byte size is under the 100 MiB cap at `model.rs:9`. whisper.cpp's `models/README.md` publishes SHA-1 only for a subset of files (no tiny/base/small/medium quantized builds), so SHA-1 is not used here; every row below has a SHA-256.

URL pattern: `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/<file>`. Pin to a commit hash instead of `main` before shipping so a re-upload cannot change the digest.

RAM is for the f16 model from whisper.cpp's README table (tiny ~273 MB, base ~388 MB, small ~852 MB, medium ~2.1 GB, large ~3.9 GB) [web, via a mirror of the README: https://github.com/ggerganov/whisper.cpp?ref=explainx]. Quantized files hold fewer weight bytes, so RAM falls roughly with file size [inferred]. The turbo figure and every "speed" cell are [inferred] estimates, scaled from one measurement: whisper.cpp `small` on CPU ran at 3.3x real time (RTF 0.31, Apple M5, 4 threads) per https://whispaste.de/en/engine-benchmarks/. Real numbers on the target laptop need a run of `whisper-bench` or Tuck's own timing.

| Tier | File | Bytes | SHA-256 | RAM (est.) | CPU speed (est., x real time) |
|---|---|---|---|---|---|
| tiny | `ggml-tiny.bin` | 77,691,713 | `be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21` | ~273 MB | ~20x |
| tiny.en | `ggml-tiny.en.bin` | 77,704,715 | `921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f` | ~273 MB | ~20x |
| tiny q5_1 | `ggml-tiny-q5_1.bin` | 32,152,673 | `818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7` | ~150 MB | ~20x |
| tiny.en q5_1 | `ggml-tiny.en-q5_1.bin` | 32,166,155 | `c77c5766f1cef09b6b7d47f21b546cbddd4157886b3b5d6d4f709e91e66c7c2b` | ~150 MB | ~20x |
| tiny q8_0 | `ggml-tiny-q8_0.bin` | 43,537,433 | `c2085835d3f50733e2ff6e4b41ae8a2b8d8110461e18821b09a15c40c42d1cca` | ~160 MB | ~20x |
| tiny.en q8_0 | `ggml-tiny.en-q8_0.bin` | 43,550,795 | `5bc2b3860aa151a4c6e7bb095e1fcce7cf12c7b020ca08dcec0c6d018bb7dd94` | ~160 MB | ~20x |
| base | `ggml-base.bin` | 147,951,465 | `60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe` | ~388 MB | ~10x |
| base.en | `ggml-base.en.bin` | 147,964,211 | `a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002` | ~388 MB | ~10x |
| base q5_1 | `ggml-base-q5_1.bin` | 59,707,625 | `422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898` | ~220 MB | ~10x |
| base.en q5_1 | `ggml-base.en-q5_1.bin` | 59,721,011 | `4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f` | ~220 MB | ~10x |
| base q8_0 | `ggml-base-q8_0.bin` | 81,768,585 | `c577b9a86e7e048a0b7eada054f4dd79a56bbfa911fbdacf900ac5b567cbb7d9` | ~250 MB | ~10x |
| base.en q8_0 | `ggml-base.en-q8_0.bin` | 81,781,811 | `a4d4a0768075e13cfd7e19df3ae2dbc4a68d37d36a7dad45e8410c9a34f8c87e` | ~250 MB | ~10x |
| small | `ggml-small.bin` | 487,601,967 | `1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b` | ~852 MB | ~3x |
| small.en | `ggml-small.en.bin` | 487,614,201 | `c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d` | ~852 MB | ~3x |
| small q5_1 | `ggml-small-q5_1.bin` | 190,085,487 | `ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb` | ~400 MB | ~3x |
| small.en q5_1 | `ggml-small.en-q5_1.bin` | 190,098,681 | `bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30` | ~400 MB | ~3x |
| small q8_0 | `ggml-small-q8_0.bin` | 264,464,607 | `49c8fb02b65e6049d5fa6c04f81f53b867b5ec9540406812c643f177317f779f` | ~500 MB | ~3x |
| small.en q8_0 | `ggml-small.en-q8_0.bin` | 264,477,561 | `67a179f608ea6114bd3fdb9060e762b588a3fb3bd00c4387971be4d177958067` | ~500 MB | ~3x |
| medium | `ggml-medium.bin` | 1,533,763,059 | `6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208` | ~2.1 GB | ~1x |
| medium.en | `ggml-medium.en.bin` | 1,533,774,781 | `cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356` | ~2.1 GB | ~1x |
| medium q5_0 | `ggml-medium-q5_0.bin` | 539,212,467 | `19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f` | ~900 MB | ~1x |
| medium.en q5_0 | `ggml-medium.en-q5_0.bin` | 539,225,533 | `76733e26ad8fe1c7a5bf7531a9d41917b2adc0f20f2e4f5531688a8c6cd88eb0` | ~900 MB | ~1x |
| medium q8_0 | `ggml-medium-q8_0.bin` | 823,369,779 | `42a1ffcbe4167d224232443396968db4d02d4e8e87e213d3ee2e03095dea6502` | ~1.2 GB | ~1x |
| medium.en q8_0 | `ggml-medium.en-q8_0.bin` | 823,382,461 | `43fa2cd084de5a04399a896a9a7a786064e221365c01700cea4666005218f11c` | ~1.2 GB | ~1x |
| large-v3-turbo | `ggml-large-v3-turbo.bin` | 1,624,555,275 | `1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69` | ~2.2 GB [unverified] | ~0.7-1x |
| large-v3-turbo q5_0 | `ggml-large-v3-turbo-q5_0.bin` | 574,041,195 | `394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2` | ~900 MB [unverified] | ~0.7-1x |
| large-v3-turbo q8_0 | `ggml-large-v3-turbo-q8_0.bin` | 874,188,075 | `317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1` | ~1.3 GB [unverified] | ~0.7-1x |

There is no `.en` build of `large-v3-turbo`, and quantized `.en` builds exist only for tiny, base, small and medium, as above.

For a 5-15 s dictation clip, ~3x real time means a 2-5 s wait for `small` and 10 s or more for `medium` on CPU alone, so offering `medium` and above without a GPU path is a trap [inferred].

Accuracy for context, WER on LibriSpeech test-clean for Q8_0 from transcribe.cpp's whisper doc [web] https://github.com/handy-computer/transcribe.cpp/blob/main/docs/models/whisper.md: tiny 7.52%, tiny.en 5.72%, base 5.12%, base.en 4.16%, small 3.33%, small.en 3.09%, medium 2.64%, large-v3-turbo 2.01%. Clean read speech; accented or noisy dictation will be worse. Accuracy on Nigerian English is [unverified]: test with a handful of the owner's own recordings before picking the default.

**Suggested picker (four entries, not twenty-seven):**

| Label | Pick | Size | Notes |
|---|---|---|---|
| Fastest | `base.en` q5_1 | 60 MB | English only |
| Balanced (default candidate) | `small` q5_1 | 190 MB | Multilingual, ~3-5 s on CPU |
| Accurate | `medium` q5_0 | 539 MB | Needs ≥ 8 GB free RAM; slow without a GPU |
| Best | `large-v3-turbo` q5_0 | 574 MB | Needs ≥ 8 GB free RAM; GPU strongly advised |

Gate the heavier two on `GlobalMemoryStatusEx` free RAM (already used in `telemetry.rs:246-278`) and show a warning rather than hide them.

Code changes this implies (not made here): `model.rs` is single-model, with a constant filename, URL, 100 MiB cap and digest (`model.rs:7-10`, `:27`, `:86`, `:110`, `:167`). It needs a `{id, file, url, bytes, sha256}` table, a size cap taken from `bytes`, a free-disk check, resume support for the 500 MB+ files (the current 600 s total timeout, `:68`, will fail on slow links), and `.part` cleanup, which the `TemporaryModelFile` guard already does.

### 3.3 Parakeet (ONNX) feasibility

| Question | Finding |
|---|---|
| Runtimes | (a) `transcribe-rs` with the `onnx` feature (ONNX Runtime; Parakeet int8 directory of `encoder-model.int8.onnx`, `decoder_joint-model.int8.onnx`, `nemo128.onnx`, `vocab.txt`); (b) `sherpa-onnx`; (c) `transcribe.cpp` GGUF via its Rust binding, no ONNX Runtime. Sources: https://github.com/cjpais/transcribe-rs, https://github.com/handy-computer/transcribe.cpp [web]. |
| Model size | Parakeet V3 ~478 MB and V2 ~473 MB per Handy's docs [web]. Another project lists the V3 int8 ONNX at ~640 MB (https://github.com/automataIA/grunt-bone-rs) [unverified discrepancy]. A smaller `parakeet-tdt_ctc-110m` exists: 134 MB on disk per one blog (https://greffier.app/blog/beyond-whisper/) [unverified], 563 MB peak RSS and RTFx 81 on a 20-core x86 host (parakeet.cpp benchmark, https://localai.io/blog/parakeet-cpp-asr-on-cpu/) [web]. It is English; it is the best candidate to replace `tiny` if a spike is run. |
| Installer size added | **Models are downloaded, not bundled**, so the installer grows only by the runtime. ONNX Runtime's DLL size is [unverified] here (I did not find a sourced figure); treat the `ort` route as "tens of MB" until measured. The `transcribe.cpp` route reuses ggml and adds no ONNX Runtime [inferred]. |
| Runtime cost (RAM) | The 0.6B models peaked at about 2.5 GB RSS at f32 in the parakeet.cpp benchmark (tdt-0.6b-v2: 2,545 MB), halved by quantization [web, f32 figure only; quantized RSS [unverified]]. Same order as Whisper `medium`. |
| CPU speed vs Whisper | transcribe-rs README: Parakeet int8 ~30x real time on an M4 Max, ~20x on a Ryzen 5700X, ~5x on a 2015 Core i5-6500 [web]. On the same Apple M5 CPU, Parakeet V3 ran 34.7x real time versus whisper.cpp `small` at 3.3x (https://whispaste.de/en/engine-benchmarks/) [web; different thread counts: 8 vs 4]. parakeet.cpp reports about 27x whisper.cpp turbo on CPU [web; project's own claim]. Net: roughly 7-10x faster than Whisper `small` per second of audio, at better stated accuracy (1.9% vs 3.4% WER on that small test). |
| Limits | V3 covers 25 European languages with automatic language detection that cannot be overridden; it writes numbers as words ("one", "two"), not digits [web, Handy docs]. It does **not** cover Yoruba, Hausa or Igbo. Whisper's 99 languages do list Yoruba and Hausa [inferred from general Whisper knowledge, not on a fetched page]. |
| Verdict | Worth a one-week spike behind a feature flag, not a swap. The cleanest route is `transcribe.cpp`'s Rust binding if its Windows build is acceptable to this project's CI (it documents a Vulkan SDK build guide); otherwise `transcribe-rs`. Measure installer delta, idle RAM, and word error on the owner's own recordings first. |

### 3.4 Language and accent note

The owner is likely to dictate Nigerian English, possibly with Yoruba or Pidgin. Whisper multilingual models can switch language per clip; Parakeet V3 cannot be told a language and does not list those languages. If English only is the goal, `.en` Whisper builds are slightly better and cheaper at the same size [web, transcribe.cpp docs]. That points to `base.en` or `small.en` as the default for English-only users and a multilingual `small` for everyone else. Whether `.en` handles Nigerian-accented English better than multilingual is [unverified].

### 3.5 Can whisper.cpp load the `handy-computer` GGUF files?

**No, not with `whisper-rs` / whisper.cpp as Tuck uses them.**

- The whisper.cpp maintainer's answer in a discussion (Nov 21, 2024): "`whisper.cpp` does not support the GGUF file format yet", and the models on the Hugging Face page "are not really in GGUF format" [web] https://github.com/ggml-org/whisper.cpp/discussions/2578. I did not find a later announcement that changed this, but a search that returned nothing is not proof; check the `whisper.cpp` release notes when the catalog work starts.
- The `handy-computer` GGUFs (for example `handy-computer/whisper-tiny-gguf`; the org card says they are "officially supported by transcribe.cpp") are built for `transcribe.cpp`, a separate engine that also runs Whisper from GGUF [web].
- `whisper-rs = "0.16"` (`Cargo.toml:33`) wraps whisper.cpp, so it needs `ggml-*.bin`. Use the ggerganov files in 3.2. The only way to use `handy-computer` GGUFs is to move to `transcribe.cpp` (MIT, Rust bindings) and take on a younger engine.

### 3.6 Where other Windows apps keep ggml models (for detection)

| App | Location | Evidence |
|---|---|---|
| Handy | `%APPDATA%\com.pais.handy\models\` (custom `.bin` auto-discovered) | [web] https://github.com/cjpais/Handy/issues/1496, https://handy.computer/docs/models.html |
| Buzz | `%USERPROFILE%\AppData\Local\Buzz\Buzz\Cache` (models under it) | [web] https://chidiwilliams.github.io/buzz/docs/faq. Subfolder layout [unverified]; scan recursively. |
| OpenWhispr | `%USERPROFILE%\.cache\openwhispr\whisper-models\` (may fall back to `%ProgramData%\OpenWhispr\cache` if the path has spaces or non-ASCII) | [web] https://docs.openwhispr.com/platform/where-your-files-live |
| whisper.cpp itself | `models\ggml-*.bin` inside the clone, from `download-ggml-model.sh`; user-chosen otherwise | [web] https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/models/README.md |
| Hugging Face cache | `%USERPROFILE%\.cache\huggingface\hub\models--ggerganov--whisper.cpp\snapshots\<rev>\` | [inferred] from the standard hub layout; not checked |
| Vibe and others | Not confirmed in this pass | [unverified] |

Rules for detection: scan these folders read-only for `ggml-*.bin`; accept a file only if its size and SHA-256 equal a row in the catalog (the same check Tuck already runs on its own download, `model.rs:20-49`); never write to, move or delete another app's files; copy or hard-link into Tuck's folder if the user agrees. Skip `.gguf` files: they are not loadable here (3.5). The first bytes tell them apart: ggml files start with the magic `lmgg` (0x67676d6c little-endian) and GGUF files start with `GGUF` [inferred from the formats; check before relying on it].

---

## Open items I could not settle

- Real idle numbers for Tuck (process count, private MB, CPU %) were not measured; section 1.6 gives the method.
- Whether Tauri's `win.hide()` marks the WebView2 controller invisible, and therefore whether timers throttle and `TrySuspend` is allowed, needs a runtime check.
- Nusa's ranking and tolerance logic cannot be read from strings.
- Afrobeats coverage on LRCLIB or any other source is unquantified.
- Installer size of the ONNX Runtime route, Parakeet quantized RAM, and per-model CPU speed on the target laptop.
- Which of Handy's models is its first-run default, and whether it recommends by hardware.
