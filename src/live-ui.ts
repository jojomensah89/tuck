import { listenToEvent, mediaSeek, isTauri } from './ipc.js';
import { SVG_ICONS } from './icons.js';
import { islandModelLine } from './dictation-models.js';
import { nativeAction, nativeService, normalizeClipboard, normalizeDictation, normalizeTelemetry, type ClipboardSnapshot, type DictationSnapshot, type LyricsSnapshot, type MediaSession, type TelemetrySnapshot } from './live-services.js';
import { historyPath, lyricIndex, lyricOffsetFor, matchesLyricRequest, MAX_LYRIC_OFFSET_MS, measuredPercent, newRequestId, parseLrc, readPreferences, TelemetryGate, withLyricOffset, writePreferences, type LyricLine } from './live-state.js';
import { formatClockTime } from './time-format.js';
import { createRing, usageTone, type Ring } from './rings.js';
import type { MediaState } from './types.js';

const node = <K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text = '') => {
  const el = document.createElement(tag); el.className = className; el.textContent = text; return el;
};
function button(label: string, action: () => void): HTMLButtonElement {
  const b = node('button', 'live-button', label); b.type = 'button'; b.onclick = e => { e.stopPropagation(); action(); }; return b;
}
/** An icon from the line family with its name as tooltip and accessible name */
function iconButton(label: string, icon: string, action: () => void, className = ''): HTMLButtonElement {
  const b = node('button', `icon-btn${className ? ` ${className}` : ''}`); b.type = 'button';
  setIconLabel(b, label, icon);
  b.onclick = e => { e.stopPropagation(); action(); }; return b;
}
function setIconLabel(b: HTMLElement, label: string, icon: string): void {
  if (b.getAttribute('aria-label') === label) return;
  b.setAttribute('aria-label', label); b.title = label; b.innerHTML = icon;
}
/** A 6px status dot: its tone carries the state, its name says it */
function statusDot(): { el: HTMLElement; set(state: 'live' | 'idle' | 'warn' | 'off', label: string): void } {
  const el = node('span', 'status-dot'); el.setAttribute('role', 'img');
  return { el, set(state, label) { el.dataset.status = state; el.setAttribute('aria-label', label); el.title = label; } };
}
function makePage(id: string, title: string): HTMLElement {
  const page = node('section', `page live-page ${id}`); page.dataset.page = id; page.setAttribute('aria-label', title);
  document.querySelector('.pages-container')!.append(page); return page;
}
const friendlyError = (error: unknown) => error instanceof Error ? error.message : 'Action failed, try again';

export function mountLivePages(onLayout: () => void): {
  media: (state: MediaState, position: number) => void;
  position: (position: number) => void;
  compactLyric: () => string;
} {
  const mediaPage = document.querySelector<HTMLElement>('.page.media')!;
  const ctl = mediaPage.querySelector<HTMLElement>('.ctl')!;
  const toolbar = node('div', 'media-tools');
  const message = node('p', 'live-note'); message.setAttribute('role', 'status');
  const lyricsPane = node('div', 'lyrics-pane'); lyricsPane.setAttribute('aria-label', 'Lyrics');
  const lyricList = node('div', 'lyric-lines');
  const lyricsHeader = node('div', 'lyrics-header');
  const lyricsStatus = node('p', 'lyrics-status', 'Enable online lyrics in Settings');
  const offsetControls = node('div', 'lyrics-offset-controls'); offsetControls.setAttribute('role', 'group'); offsetControls.setAttribute('aria-label', 'Lyrics timing');
  const lyricToggle = node('button', 'lyrics-toggle-btn');
  lyricToggle.type = 'button';
  lyricToggle.setAttribute('aria-label', 'Toggle lyrics');
  lyricToggle.title = 'Show or hide lyrics';
  lyricToggle.innerHTML = SVG_ICONS.lyrics;
  lyricToggle.onclick = e => {
    e.stopPropagation();
    mediaPage.classList.toggle('with-lyrics');
    lyricToggle.setAttribute('aria-pressed', String(mediaPage.classList.contains('with-lyrics')));
    onLayout();
  };
  toolbar.append(lyricToggle);
  ctl.prepend(toolbar);
  mediaPage.append(lyricsPane, message);
  lyricsHeader.append(lyricsStatus, offsetControls);
  const lyricsNote = node('p', 'lyrics-note', 'Another edit? Timing may be off'); lyricsNote.hidden = true;
  lyricsPane.append(lyricsHeader, lyricsNote, lyricList);
  mediaPage.classList.toggle('with-lyrics', readPreferences().lyricsOpen);
  lyricToggle.setAttribute('aria-pressed', String(readPreferences().lyricsOpen));
  let currentTrack = '', generation = 0, baseLines: LyricLine[] = [], lines: LyricLine[] = [], activeLine = -2;
  let currentRequestId = '';
  let latestMedia: MediaState | null = null;
  let currentLyric = '', lastPosition = 0, manualScrollUntil = 0, userOffsetMs = 0;
  // Manual scrolling pauses auto-follow; the tab rail decides whether the wheel scrolls or switches tabs
  lyricList.addEventListener('wheel', () => { manualScrollUntil = Date.now() + 4000; }, { passive: true });
  async function sessions(): Promise<void> {
    try {
      await nativeService<MediaSession[]>('media_sessions');
    } catch { /* native player sessions unavailable */ }
  }
  const updatePosition = (position: number) => {
    lastPosition = position;
    const pastLyrics = lines.length > 0 && position > lines[lines.length - 1].atMs + 20000;
    const index = pastLyrics ? -1 : lyricIndex(lines, position);
    currentLyric = index >= 0 ? lines[index].text : '';
    if (index === activeLine) return;
    activeLine = index;
    [...lyricList.children].forEach((el, i) => {
      el.classList.toggle('current', i === index);
      if (i === index) el.setAttribute('aria-current', 'true'); else el.removeAttribute('aria-current');
    });
    if (index >= 0 && Date.now() > manualScrollUntil) {
      const target = lyricList.children[index] as HTMLElement;
      // The list is positioned, so offsetTop is already measured from it
      lyricList.scrollTop = Math.max(0, target.offsetTop - lyricList.clientHeight / 2 + target.offsetHeight / 2);
    }
  };
  const offsetValue = node('span', 'lyrics-offset-value');
  offsetValue.setAttribute('aria-live', 'polite');
  offsetValue.title = 'How far the lyrics are shifted. Plus shows them later';
  /** Hides the timing row without dropping keyboard focus onto the page. */
  function hideTiming(): void {
    if (offsetControls.contains(document.activeElement)) lyricToggle.focus();
    offsetControls.hidden = true; lyricsNote.hidden = true;
  }
  /** Shifts the existing lines by a constant; a different edit moves lyrics by that, not by a ratio. */
  function retimeLyrics(): void {
    lines = baseLines.map(line => ({ ...line, atMs: Math.max(0, line.atMs + userOffsetMs) }));
    const sign = userOffsetMs > 0 ? '+' : userOffsetMs < 0 ? '−' : '';
    offsetValue.textContent = `${sign}${(Math.abs(userOffsetMs) / 1000).toFixed(1)} s`;
    activeLine = -2; updatePosition(lastPosition);
  }
  function applyLyrics(snapshot: LyricsSnapshot): void {
    if (!snapshot || !matchesLyricRequest({ trackId: currentTrack, requestId: currentRequestId }, snapshot)) return;
    baseLines = snapshot.lines || parseLrc(snapshot.syncedLyrics || '');
    userOffsetMs = baseLines.length ? lyricOffsetFor(readPreferences(), currentTrack) : 0;
    currentLyric = ''; lyricList.replaceChildren();
    const statuses: Record<LyricsSnapshot['status'], string> = {
      disabled: 'Enable online lyrics in Settings', loading: 'Finding lyrics…', synced: '', plain: 'Lyrics · not synchronized',
      instrumental: 'Instrumental', unavailable: 'No lyrics for this track', error: 'Lyrics unavailable, check your connection',
    };
    const approximate = !!snapshot.approximate && (snapshot.status === 'synced' || snapshot.status === 'plain');
    lyricsStatus.textContent = approximate ? (snapshot.status === 'plain' ? 'Approximate · not synchronized' : 'Approximate') : statuses[snapshot.status] ?? 'Lyrics unavailable';
    lyricsStatus.title = approximate ? 'The closest lyrics found. They may be from another edit of this song, so the timing can be off' : '';
    lyricsPane.title = snapshot.creditDiffers ? 'The lyrics found credit this song a little differently' : '';
    // Only a timing mismatch warrants a warning or the stepper; a saved correction stays reachable
    if (baseLines.length && (approximate || userOffsetMs !== 0)) offsetControls.hidden = false; else hideTiming();
    lyricsNote.hidden = !(approximate && baseLines.length);
    if (baseLines.length) {
      baseLines.forEach((line, index) => {
        const b = button(line.text || '♪', () => { void act(mediaSeek(lines[index].atMs), message); });
        b.className = 'lyric-line'; b.title = 'Seek to this line'; lyricList.append(b);
      });
    } else if (snapshot.plainLyrics) {
      const plain = node('p', 'plain-lyrics', snapshot.plainLyrics.slice(0, 250000)); lyricList.append(plain);
    }
    retimeLyrics();
  }
  function adjustLyrics(deltaMs: number): void {
    if (!baseLines.length) return;
    userOffsetMs = Math.max(-MAX_LYRIC_OFFSET_MS, Math.min(MAX_LYRIC_OFFSET_MS, userOffsetMs + deltaMs));
    writePreferences({ lyricsOffsets: withLyricOffset(readPreferences(), currentTrack, userOffsetMs) });
    retimeLyrics();
  }
  /** One press is 100 ms; holding repeats, because a different edit is often seconds out. */
  function offsetStepper(label: string, icon: string, deltaMs: number): HTMLButtonElement {
    // The track a press began on: a correction made for one song must never land on the next
    let timer = 0, repeated = false, pressedTrack: string | null = null;
    const stop = () => window.clearTimeout(timer);
    const reset = () => { stop(); repeated = false; pressedTrack = null; };
    const b = iconButton(label, icon, () => {}, 'lyrics-offset-btn');
    b.onclick = e => {
      e.stopPropagation();
      // A keyboard press has no pointer history. A pointer's click only counts if its press
      // began on this track, was not cancelled and has not already repeated.
      if (e.detail === 0 || (pressedTrack === currentTrack && !repeated)) adjustLyrics(deltaMs);
      reset();
    };
    const tick = () => {
      if (pressedTrack !== currentTrack) return;
      repeated = true; adjustLyrics(deltaMs); timer = window.setTimeout(tick, 60);
    };
    b.onpointerdown = e => {
      reset();
      if (e.button !== 0) return;
      pressedTrack = currentTrack; timer = window.setTimeout(tick, 400);
    };
    b.onpointerup = stop;
    // No click follows these, so the next keyboard press must still count
    b.onpointercancel = b.onpointerleave = reset;
    return b;
  }
  offsetControls.append(
    offsetStepper('Show lyrics 0.1 s earlier, hold to repeat', SVG_ICONS.minus, -100), offsetValue,
    offsetStepper('Show lyrics 0.1 s later, hold to repeat', SVG_ICONS.plus, 100),
  );
  offsetControls.hidden = true;
  async function requestLyrics(state: MediaState): Promise<void> {
    const request = ++generation;
    const requestId = newRequestId(); currentRequestId = state.active ? requestId : '';
    baseLines = []; lines = []; currentLyric = ''; activeLine = -2; lyricList.replaceChildren(); hideTiming(); lyricsPane.title = '';
    lyricsStatus.textContent = state.active ? 'Finding lyrics\u2026' : 'Play music to see lyrics';
    if (!state.active) return;
    try {
      const data = await nativeService<LyricsSnapshot>('lyrics_lookup', {
        trackId: state.trackId, requestId, title: state.title, artist: state.artist, durationMs: state.durationMs,
      });
      if (request === generation) applyLyrics(data);
    } catch { if (request === generation) lyricsStatus.textContent = isTauri() ? 'Lyrics unavailable' : 'Online lyrics need the Windows app'; }
  }

  // System stats: one ring per meter with the value inside, its name and a sparkline under it
  const stats = makePage('stats', 'System usage');
  const statHead = node('div', 'live-heading'); const statTitle = node('h2', '', 'System');
  const statsStatus = statusDot(); statsStatus.set('idle', 'Waiting for native samples');
  const statTitleRow = node('div', 'live-title'); statTitleRow.append(statTitle, statsStatus.el); statHead.append(statTitleRow); stats.append(statHead);
  const statRows = node('div', 'stat-rings'); stats.append(statRows);
  const histories = new Map<string, (number | null)[]>();
  const telemetryGate = new TelemetryGate();
  const rowNodes = new Map<string, { row: HTMLElement; face: HTMLElement; ring: Ring; value: HTMLElement; path: SVGPathElement; name: string; detail: string }>();
  function metric(id: string, label: string, pct: number | null, detail = '', fullName = label): void {
    let view = rowNodes.get(id);
    if (!view) {
      const row = node('div', 'stat-cell');
      const face = node('div', 'ring-face stat-face'); face.tabIndex = 0; face.setAttribute('role', 'img');
      const ring = createRing(56, 6); const value = node('span', 'ring-glyph stat-value');
      face.append(ring.root, value);
      const name = node('span', 'stat-name', label);
      const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); svg.setAttribute('viewBox', '0 0 180 40'); svg.setAttribute('preserveAspectRatio', 'none'); svg.setAttribute('aria-hidden', 'true'); svg.setAttribute('class', 'spark');
      const path = document.createElementNS(svg.namespaceURI, 'path') as SVGPathElement; svg.append(path); row.append(face, name, svg); statRows.append(row);
      view = { row, face, ring, value, path, name: fullName, detail }; rowNodes.set(id, view);
    }
    view.name = fullName; view.detail = detail;
    const history = histories.get(id) || []; history.push(pct); if (history.length > 45) history.shift(); histories.set(id, history);
    paintMetric(view, pct);
    view.path.setAttribute('d', historyPath(history));
  }
  function paintMetric(view: { row: HTMLElement; face: HTMLElement; ring: Ring; value: HTMLElement; name: string; detail: string }, pct: number | null): void {
    // Load reads in ink until it is high; green would make an idle machine the loudest thing here
    const tone = usageTone(pct);
    view.ring.set(pct, tone === 'ok' ? 'ink' : tone);
    const text = pct === null ? '\u2013' : `${Math.round(pct)}%`;
    if (view.value.textContent !== text) view.value.textContent = text;
    view.row.dataset.unavailable = String(pct === null);
    const label = pct === null ? `${view.name} unavailable` : `${view.name} ${Math.round(pct)}%${view.detail ? `, ${view.detail}` : ''}`;
    view.face.setAttribute('aria-label', label); view.face.title = label;
  }
  const gb = (value: number) => (value / 1073741824).toFixed(1);
  function telemetry(data: TelemetrySnapshot): void {
    if (!data || !Array.isArray(data.gpus)) return;
    const sampled = Number.isFinite(data.sampledAt) ? data.sampledAt : 0;
    if (!telemetryGate.accept(sampled)) return;
    const stale = data.stale || !sampled || Date.now() - sampled > 10000;
    if (stale) { invalidateTelemetry('Stale, warming up'); return; }
    statsStatus.set('live', `Live, sampled ${formatClockTime(sampled)}`);
    metric('cpu', 'CPU', measuredPercent(data.cpuPct));
    const validRam = typeof data.ramUsedBytes === 'number' && typeof data.ramTotalBytes === 'number' && data.ramTotalBytes > 0 && data.ramUsedBytes >= 0;
    metric('ram', 'Memory', validRam ? measuredPercent(data.ramUsedBytes! / data.ramTotalBytes! * 100) : null, validRam ? `${gb(data.ramUsedBytes!)} of ${gb(data.ramTotalBytes!)} GB` : '');
    const ids = new Set(['cpu', 'ram']);
    data.gpus.forEach((gpu, i) => { ids.add(gpu.id); metric(gpu.id, data.gpus.length > 1 ? `GPU ${i + 1}` : 'GPU', measuredPercent(gpu.usedPct), '', gpu.name || 'GPU'); });
    if (!data.gpus.length) { ids.add('gpu-unknown'); metric('gpu-unknown', 'GPU', null); }
    for (const [id, view] of rowNodes) if (!ids.has(id)) { view.row.remove(); rowNodes.delete(id); histories.delete(id); }
  }
  metric('cpu', 'CPU', null); metric('ram', 'Memory', null); metric('gpu-unknown', 'GPU', null);
  function invalidateTelemetry(label: string): void {
    statsStatus.set(label.startsWith('Stale') ? 'idle' : 'warn', label);
    for (const [id, view] of rowNodes) {
      paintMetric(view, null);
      const history = histories.get(id) || []; history.push(null); if (history.length > 45) history.shift();
      histories.set(id, history); view.path.setAttribute('d', historyPath(history));
    }
  }
  async function refreshTelemetry(): Promise<void> {
    if (!telemetryGate.begin()) return;
    const startedAtSample = telemetryGate.sampledAt;
    try { telemetry(await nativeService<TelemetrySnapshot>('telemetry_snapshot')); }
    catch {
      // An older failed request must not invalidate a newer event received in flight.
      if (telemetryGate.sampledAt === startedAtSample && telemetryGate.failed()) invalidateTelemetry('Native telemetry unavailable');
    } finally { telemetryGate.finish(); }
  }

  // Dictation: the transcript is the content; record (stop while recording), cancel and insert are icons
  const speech = makePage('dictation', 'Dictation');
  const speechHead = node('div', 'live-heading'); const speechTitleRow = node('div', 'live-title');
  const speechDot = statusDot(); speechDot.set('off', 'Offline, set up a model in Settings');
  speechTitleRow.append(node('h2', '', 'Dictation'), speechDot.el);
  const speechStatus = node('p', 'live-note');
  const speechModel = node('p', 'live-note', islandModelLine(null));
  const transcript = node('div', 'transcript placeholder', 'Your transcript appears here'); transcript.setAttribute('role', 'status');
  const speechActions = node('div', 'live-actions');
  let dictationPhase = 'disabled';
  let provisional = '';
  let previewsPaused = false;
  const record = iconButton('Record', SVG_ICONS.mic, () => {
    void act(nativeAction(dictationPhase === 'recording' ? 'dictation_stop' : 'dictation_toggle'), speechStatus);
  }, 'primary');
  const cancel = iconButton('Cancel', SVG_ICONS.close, () => { void act(nativeAction('dictation_cancel'), speechStatus); });
  const insert = iconButton('Insert', SVG_ICONS.insert, () => { void act(nativeAction('dictation_insert'), speechStatus); }); insert.disabled = true;
  speechActions.append(record, cancel, insert); speechHead.append(speechTitleRow, speechActions);
  speech.append(speechHead, transcript, speechModel, speechStatus);
  function dictation(data: DictationSnapshot): void {
    if (!data || typeof data.status !== 'string') return;
    if (data.status === 'starting' || !['recording', 'transcribing'].includes(data.status)) provisional = '';
    dictationPhase = data.status;
    previewsPaused = data.previewsPaused === true;
    // Explain engine work and a paused preview without implying the microphone is live.
    speechStatus.textContent = data.error || (data.status === 'cancelling' ? 'Cancelling\u2026' : data.status === 'processing' ? 'Microphone closed. Waiting for the speech engine\u2026' : data.status === 'transcribing' ? 'Transcribing\u2026' : previewsPaused && data.status === 'recording' ? 'Live transcript paused for this recording.' : data.modelNotice || '');
    speechModel.textContent = islandModelLine(data.activeModelLabel);
    const dotState = data.error || data.status === 'error' ? 'warn' : data.status === 'recording' ? 'live' : ['starting', 'transcribing', 'processing', 'cancelling', 'inserting', 'review'].includes(data.status) ? 'idle' : 'off';
    speechDot.set(dotState, `Offline, ${data.status}`);
    transcript.textContent = data.transcript?.slice(0, 32000) || provisional || (data.status === 'recording' ? 'Listening\u2026' : 'Your transcript appears here');
    transcript.classList.toggle('placeholder', !data.transcript && !provisional);
    insert.disabled = data.status !== 'review' || data.canInsert === false;
    cancel.disabled = !['starting', 'recording', 'transcribing', 'processing', 'inserting', 'review'].includes(data.status);
    record.disabled = !['idle', 'recording', 'review', 'error'].includes(data.status) || data.modelReady === false;
    if (data.status === 'recording') setIconLabel(record, 'Finish recording', SVG_ICONS.stop);
    else setIconLabel(record, 'Record', SVG_ICONS.mic);
    record.setAttribute('aria-pressed', String(data.status === 'recording'));
    document.getElementById('isl')!.dataset.dictation = data.status;
    const active = ['starting', 'recording', 'transcribing', 'processing', 'cancelling', 'inserting', 'review'].includes(data.status);
    window.dispatchEvent(new CustomEvent('tuck-activity', { detail: {
      id: 'dictation',
      type: 'dictation',
      phase: data.status,
      active,
      title: 'Dictation',
      text: data.transcript?.slice(-65) || data.status,
      priority: 99,
    } }));
  }

  const clipboard = makePage('clipboard', 'Clipboard history');
  const clipboardHead = node('div', 'live-heading'); const clipboardStatus = node('p', 'live-note', 'Enable clipboard history in Settings');
  const clearClipboard = iconButton('Clear unpinned history', SVG_ICONS.trash, () => { void act(nativeAction('clipboard_clear').then(refreshClipboard), clipboardStatus); });
  clipboardHead.append(node('h2', '', 'Clipboard'), clearClipboard);
  const clipList = node('div', 'clipboard-list'); clipboard.append(clipboardHead, clipboardStatus, clipList);
  let clipboardSignature = '';
  function renderClipboard(data: ClipboardSnapshot): void {
    if (!data || !Array.isArray(data.items)) return;
    // A note only when there is nothing to show; a row explains itself (select to copy, pin to keep)
    clipboardStatus.textContent = data.enabled ? (data.items.length ? '' : 'Nothing copied yet') : 'Enable clipboard history in Settings';
    clipboardStatus.hidden = !clipboardStatus.textContent;
    clearClipboard.disabled = !data.enabled || !data.items.length;
    const signature = JSON.stringify(data.items);
    if (signature === clipboardSignature) return; clipboardSignature = signature;
    const focus = (document.activeElement as HTMLElement)?.dataset.focus;
    clipList.replaceChildren();
    for (const item of [...data.items].sort((a, b) => Number(b.pinned) - Number(a.pinned)).slice(0, 100)) {
      const row = node('div', 'clipboard-row');
      const copy = button(item.text.slice(0, 160), () => { void act(nativeAction('clipboard_copy', { id: item.id }), clipboardStatus); });
      copy.className = 'clipboard-text'; copy.dataset.focus = `copy-${item.id}`; copy.title = `Copy: ${item.text.slice(0, 500)}`;
      const pin = iconButton(item.pinned ? 'Unpin' : 'Pin', SVG_ICONS.pin, () => { void act(nativeAction('clipboard_pin', { id: item.id, pinned: !item.pinned }).then(refreshClipboard), clipboardStatus); }, 'row-action pin-btn');
      pin.setAttribute('aria-pressed', String(item.pinned)); pin.dataset.focus = `pin-${item.id}`;
      const remove = iconButton('Remove', SVG_ICONS.close, () => { void act(nativeAction('clipboard_remove', { id: item.id }).then(refreshClipboard), clipboardStatus); }, 'row-action'); remove.dataset.focus = `remove-${item.id}`;
      row.dataset.pinned = String(item.pinned);
      row.append(copy, pin, remove); clipList.append(row);
    }
    if (focus) [...clipList.querySelectorAll<HTMLElement>('[data-focus]')].find(el => el.dataset.focus === focus)?.focus();
  }
  async function refreshClipboard(): Promise<void> { renderClipboard(await nativeService<ClipboardSnapshot>('clipboard_snapshot')); }

  // Frequency bars are driven exclusively by actual native loopback measurements.
  const spectrum = node('div', 'live-spectrum'); spectrum.setAttribute('aria-label', 'System audio spectrum');
  const bars = Array.from({ length: 32 }, () => { const bar = node('i'); spectrum.append(bar); return bar; }); mediaPage.append(spectrum);
  const audioPage = makePage('visualizer', 'Audio visualizer');
  const audioTitle = node('div', 'live-heading'); const audioStatus = node('p', 'live-note', 'Analysis off, no audio is stored');
  // One toggle: the spectrum itself shows whether analysis runs
  let visualizerOn = false;
  // Frames still in flight (and the capture thread's final empty frame) arrive just after Stop
  let stoppedAt = 0;
  const visualizerToggle = iconButton('Start playback analysis', SVG_ICONS.power, () => {
    const next = !visualizerOn;
    void act(nativeAction('visualizer_set_enabled', { enabled: next }).then(() => { setVisualizer(next); }), audioStatus);
  });
  function setVisualizer(on: boolean): void {
    if (visualizerOn && !on) stoppedAt = Date.now();
    visualizerOn = on;
    visualizerToggle.setAttribute('aria-pressed', String(on));
    setIconLabel(visualizerToggle, on ? 'Stop playback analysis' : 'Start playback analysis', SVG_ICONS.power);
    if (!on) audioStatus.textContent = 'Analysis off, no audio is stored';
  }
  setVisualizer(false);
  void nativeService<{ enabled: boolean }>('visualizer_settings').then(status => setVisualizer(status.enabled === true)).catch(() => {});
  audioTitle.append(node('h2', '', 'System audio'), visualizerToggle);
  const wideSpectrum = node('div', 'wide-spectrum'); wideSpectrum.setAttribute('aria-label', 'Measured playback frequency bands');
  const wideBars = Array.from({ length: 48 }, () => { const bar = node('i'); wideSpectrum.append(bar); return bar; });
  audioPage.append(audioTitle, wideSpectrum, audioStatus);
  let lastAudio = 0;
  function audio(data: { bands: number[]; active?: boolean }): void {
    if (!data || !Array.isArray(data.bands)) return;
    if (!visualizerOn) {
      // Ignore stragglers after a local Stop; later frames mean analysis was started elsewhere (Settings)
      if (Date.now() - stoppedAt < 1500) return;
      setVisualizer(true);
    }
    lastAudio = Date.now();
    audioStatus.textContent = data.active === false ? 'Silence, no audio is stored' : 'Live spectrum, no audio is stored';
    for (const group of [bars, wideBars]) group.forEach((bar, i) => {
      const source = i * Math.max(0, data.bands.length - 1) / (group.length - 1);
      const low = data.bands[Math.floor(source)] || 0, high = data.bands[Math.ceil(source)] || 0;
      const sample = data.active === false ? 0 : low + (high - low) * (source % 1);
      bar.style.setProperty('--level', String(Number.isFinite(sample) ? Math.max(0, Math.min(1, sample)) : 0));
    });
  }
  async function act(action: Promise<unknown>, status: HTMLElement): Promise<void> {
    try { await action; status.textContent = ''; } catch (error) { status.textContent = friendlyError(error); }
  }
  const unlisteners: (() => void)[] = [];
  for (const [event, handler] of [
    ['lyrics', applyLyrics], ['telemetry', (data: unknown) => telemetry(normalizeTelemetry(data))], ['dictation_state', (data: unknown) => dictation(normalizeDictation(data))], ['clipboard_history_changed', (data: unknown) => renderClipboard(normalizeClipboard(data))], ['visualizer', (data: {bins?: number[]; bands?: number[]; isSilent?: boolean}) => audio({ bands: data.bins || data.bands || [], active: !data.isSilent })],
  ] as const) void listenToEvent(event, handler as (data: unknown) => void).then(unlisten => unlisteners.push(unlisten));
  void listenToEvent<{ text: string }>('dictation_partial', data => {
    if (dictationPhase !== 'recording' || previewsPaused || typeof data?.text !== 'string') return;
    provisional = data.text.slice(0, 32000);
    transcript.textContent = data.text.slice(0, 32000);
    transcript.classList.toggle('placeholder', !data.text);
    window.dispatchEvent(new CustomEvent('tuck-activity', { detail: { id: 'dictation', type: 'dictation', phase: 'recording', active: true, title: 'Dictation', text: data.text.slice(-65), priority: 99 } }));
  }).then(unlisten => unlisteners.push(unlisten));
  void listenToEvent<{ enabled: boolean }>('lyrics-settings', data => {
    if (!data.enabled) { generation++; currentRequestId = ''; baseLines = []; lines = []; currentLyric = ''; lyricList.replaceChildren(); hideTiming(); lyricsStatus.textContent = 'Enable online lyrics in Settings'; }
    else if (latestMedia?.active) void requestLyrics(latestMedia);
  }).then(unlisten => unlisteners.push(unlisten));
  void listenToEvent<string>('visualizer-error', () => { audioStatus.textContent = 'Playback analysis unavailable, check the output device'; }).then(unlisten => unlisteners.push(unlisten));
  void sessions();
  void refreshTelemetry();
  void nativeService<DictationSnapshot>('dictation_snapshot').then(dictation).catch(() => { speechStatus.textContent = 'Dictation service unavailable, see Settings'; });
  void refreshClipboard().catch(() => { clipboardStatus.textContent = 'Clipboard service unavailable, see Settings'; });
  const interval = window.setInterval(() => {
    if (Date.now() - lastAudio > 1000) [...bars, ...wideBars].forEach(bar => bar.style.setProperty('--level', '0'));
    if (telemetryGate.sampledAt && Date.now() - telemetryGate.sampledAt > 10000) invalidateTelemetry('Stale, warming up');
    if (stats.closest('.tab-panel.on')) void refreshTelemetry();
  }, 1000);
  const sessionInterval = window.setInterval(() => { void sessions(); }, 5000);
  window.addEventListener('pagehide', () => { clearInterval(interval); clearInterval(sessionInterval); unlisteners.forEach(unlisten => unlisten()); }, { once: true });
  return {
    media(state, position) { latestMedia = state; updatePosition(position); if (state.trackId !== currentTrack || !state.active) { currentTrack = state.trackId; void requestLyrics(state); } },
    position: updatePosition, compactLyric: () => currentLyric,
  };
}
