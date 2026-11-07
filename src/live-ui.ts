import { clipCategory, CLIP_CATEGORIES } from './clipboard-categories.js';
import { listenToEvent, mediaSeek, isTauri, openSettingsCmd, triggerQuickAction } from './ipc.js';
import { SVG_ICONS } from './icons.js';
import { islandModelLine } from './dictation-models.js';
import { DictationUpdates, nativeAction, nativeService, normalizeClipboard, normalizeDictation, normalizeTelemetry, type ClipboardSnapshot, type DictationSnapshot, type LyricsSnapshot, type MediaSession, type TelemetrySnapshot } from './live-services.js';
import { historyPath, heldLyric, lyricIndex, lyricOffsetFor, matchesLyricRequest, MAX_LYRIC_OFFSET_MS, measuredPercent, newRequestId, parseLrc, readPreferences, TelemetryGate, withLyricOffset, writePreferences, type LyricLine } from './live-state.js';
import { createRing, usageTone, type Ring } from './rings.js';
import type { MediaState } from './types.js';

const node = <K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text = '') => {
  const el = document.createElement(tag); el.className = className; el.textContent = text; return el;
};
function button(label: string, action: () => void): HTMLButtonElement {
  const b = node('button', '', label); b.type = 'button'; b.onclick = e => { e.stopPropagation(); action(); }; return b;
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
const friendlyError = (error: unknown) => error instanceof Error ? error.message : typeof error === 'string' ? error : 'Action failed, try again';

export function mountLivePages(onLayout: () => void): {
  media: (state: MediaState, position: number) => void;
  position: (position: number) => void;
  compactLyric: () => string;
} {
  const mediaPage = document.querySelector<HTMLElement>('.page.media')!;
  mediaPage.classList.add('no-lyrics');
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
    if (lyricToggle.getAttribute('aria-disabled') === 'true') return;
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
    currentLyric = heldLyric(lines, index);
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
  /** Without lyrics the player takes the full width; the toggle goes quiet and says why */
  function syncLyricsAvailable(available: boolean): void {
    mediaPage.classList.toggle('no-lyrics', !available);
    lyricToggle.setAttribute('aria-disabled', String(!available));
    lyricToggle.title = available ? 'Show or hide lyrics' : lyricsStatus.textContent || 'Lyrics unavailable';
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
    syncLyricsAvailable(baseLines.length > 0 || !!snapshot.plainLyrics);
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
    syncLyricsAvailable(false);
    if (!state.active) return;
    try {
      const data = await nativeService<LyricsSnapshot>('lyrics_lookup', {
        trackId: state.trackId, requestId, title: state.title, artist: state.artist, durationMs: state.durationMs,
      });
      if (request === generation) applyLyrics(data);
    } catch {
      if (request !== generation) return;
      lyricsStatus.textContent = isTauri() ? 'Lyrics unavailable' : 'Online lyrics need the Windows app';
      syncLyricsAvailable(false);
    }
  }

  // System stats: one ring per meter with the value inside, its name and a sparkline under it
  const stats = makePage('stats', 'System usage');
  const statRows = node('div', 'stat-rings'); stats.append(statRows);
  const rates = node('div', 'network-rates');
  const down = node('span', 'network-rate'); const up = node('span', 'network-rate');
  const rateValue = (parent: HTMLElement, upload: boolean) => {
    parent.innerHTML = upload ? SVG_ICONS.upload : SVG_ICONS.download;
    const value = node('span', '', '\u2013 MB/s'); parent.append(value); return value;
  };
  const downValue = rateValue(down, false); const upValue = rateValue(up, true);
  rates.append(down, up); stats.append(rates);
  function setRates(d: number | null | undefined, u: number | null | undefined): void {
    for (const [value, parent, number, label] of [[downValue, down, d, 'Download'], [upValue, up, u, 'Upload']] as const) {
      const valid = typeof number === 'number' && Number.isFinite(number) && number >= 0;
      value.textContent = valid ? `${(number / 1000000).toFixed(1)} MB/s` : '\u2013 MB/s';
      parent.dataset.unavailable = String(!valid); parent.setAttribute('aria-label', `${label} ${value.textContent}`);
    }
  }
  const histories = new Map<string, (number | null)[]>();
  const telemetryGate = new TelemetryGate();
  const rowNodes = new Map<string, { row: HTMLElement; face: HTMLElement; ring: Ring; value: HTMLElement; path: SVGPathElement; name: string; detail: string }>();
  function metric(id: string, label: string, pct: number | null, detail = '', fullName = label): void {
    let view = rowNodes.get(id);
    if (!view) {
      const row = node('div', 'stat-cell');
      const face = node('div', 'ring-face stat-face'); face.tabIndex = 0; face.setAttribute('role', 'img');
      const ring = createRing(72, 6); const value = node('span', 'ring-glyph stat-value');
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
    setRates(data.networkDownBps, data.networkUpBps);
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
    setRates(null, null);
    stats.title = label;
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

  // Dictation: status on top, the transcript takes the room left, one large record control below
  const speech = makePage('dictation', 'Dictation');
  const speechHead = node('div', 'dictation-status');
  const speechDot = statusDot(); speechDot.set('off', 'Offline, set up a model in Settings');
  const speechStatus = node('p', 'dictation-note', 'Dictation is starting');
  const speechModel = node('p', 'dictation-model', islandModelLine(null));
  const modelSettings = button('Model settings', () => { speechAct(openSettingsCmd('tools')); });
  modelSettings.className = 'dictation-link'; modelSettings.hidden = true;
  speechHead.append(speechDot.el, speechStatus, speechModel, modelSettings);
  const transcript = node('div', 'transcript placeholder', 'Your transcript appears here'); transcript.setAttribute('role', 'status');
  transcript.dataset.noSwipe = ''; // dragging here selects text
  let dictationPhase = 'disabled';
  const dictationUpdates = new DictationUpdates();
  let previewsPaused = false;
  let dictationLine = 'Dictation is starting';
  /** Status text: what just failed, or else what the dictation state says */
  const paintSpeechStatus = (actionError = '') => {
    speechStatus.textContent = actionError || dictationLine;
    speechStatus.title = speechStatus.textContent;
  };
  const speechAct = (action: Promise<unknown>) => { action.then(() => paintSpeechStatus(), error => paintSpeechStatus(friendlyError(error))); };
  const record = iconButton('Record', SVG_ICONS.mic, () => {
    speechAct(nativeAction(dictationPhase === 'recording' ? 'dictation_stop' : 'dictation_toggle'));
  }, 'primary dictation-record');
  const cancel = iconButton('Cancel', SVG_ICONS.close, () => { speechAct(nativeAction('dictation_cancel')); });
  const insert = iconButton('Insert', SVG_ICONS.insert, () => { speechAct(nativeAction('dictation_insert')); }); insert.disabled = true;
  const speechControls = node('div', 'dictation-controls');
  speechControls.append(cancel, record, insert);
  speech.append(speechHead, transcript, speechControls);
  const DICTATION_PHASES: Record<string, string> = {
    disabled: 'Dictation is off', idle: 'Ready', starting: 'Starting\u2026', recording: 'Recording', review: 'Review, then insert', inserting: 'Inserting\u2026',
    error: 'Something went wrong', modelMissing: 'No speech model yet', downloadingModel: 'Downloading the model\u2026',
  };
  function dictation(data: DictationSnapshot): void {
    if (!data || typeof data.status !== 'string' || !dictationUpdates.accept(data)) return;
    dictationPhase = data.status;
    previewsPaused = data.previewsPaused === true;
    // Explain engine work and a paused preview without implying the microphone is live.
    const modelMissing = data.modelReady === false || data.status === 'modelMissing';
    dictationLine = data.error || (modelMissing && !['cancelling', 'processing', 'transcribing'].includes(data.status) ? DICTATION_PHASES.modelMissing : data.status === 'cancelling' ? 'Cancelling\u2026' : data.status === 'processing' ? 'Microphone closed. Waiting for the speech engine\u2026' : data.status === 'transcribing' ? 'Transcribing\u2026' : previewsPaused && data.status === 'recording' ? 'Live transcript paused for this recording' : data.modelNotice || DICTATION_PHASES[data.status] || '');
    paintSpeechStatus();
    speechModel.textContent = islandModelLine(data.activeModelLabel);
    modelSettings.hidden = !modelMissing;
    speechModel.hidden = modelMissing;
    const dotState = data.error || data.status === 'error' ? 'warn' : data.status === 'recording' ? 'live' : ['starting', 'transcribing', 'processing', 'cancelling', 'inserting', 'review'].includes(data.status) ? 'idle' : 'off';
    speechDot.set(dotState, `Offline, ${data.status}`);
    const provisional = ['recording', 'transcribing', 'processing'].includes(data.status) && !previewsPaused ? data.preview || '' : '';
    const text = data.transcript || provisional;
    transcript.textContent = text.slice(0, 32000) || (data.status === 'recording' ? 'Listening\u2026' : data.status === 'transcribing' ? 'Transcribing\u2026' : 'Your transcript appears here');
    transcript.classList.toggle('placeholder', !text);
    transcript.setAttribute('aria-label', provisional ? 'Live preview, words may change' : 'Transcript');
    // Live words follow the newest line
    if (provisional) transcript.scrollTop = transcript.scrollHeight;
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
      audioLevel: data.audioLevel || 0,
      active,
      title: 'Dictation',
      text: text.slice(-65),
      priority: 99,
    } }));
  }

  const clipboard = makePage('clipboard', 'Clipboard history');
  const clipboardEmpty = node('p', 'empty-line', 'Enable clipboard history in Settings');
  const clipboardStatus = node('p', 'live-note'); clipboardStatus.setAttribute('role', 'status');
  const clearClipboard = iconButton('Clear unpinned history', SVG_ICONS.trash, () => { void act(nativeAction('clipboard_clear').then(refreshClipboard), clipboardStatus); });
  clearClipboard.dataset.sectionActions = 'clipboard';
  // Shared by both parts of Tools. The tab rail moves it to the right end of the row
  const snip = iconButton('Screen snip', SVG_ICONS.capture, () => {
    triggerQuickAction('capture').catch(error => window.dispatchEvent(new CustomEvent('tuck-preview', { detail: { title: 'Screen snip', text: friendlyError(error) } })));
  });
  snip.dataset.sectionActions = 'extensions clipboard';
  const filters = node('div', 'clipboard-filters');
  filters.setAttribute('role', 'group'); filters.setAttribute('aria-label', 'Clip categories');
  const clipList = node('div', 'clipboard-list'); clipList.dataset.horizontalScroll = ''; clipList.dataset.noSwipe = '';
  filters.dataset.horizontalScroll = ''; filters.dataset.noSwipe = '';
  clipboard.append(clearClipboard, snip, filters, clipboardEmpty, clipList, clipboardStatus);
  let category: string = 'all';
  let latestClipboard: ClipboardSnapshot = { enabled: false, items: [] };
  let clipboardSignature = '';
  function renderClipboard(data: ClipboardSnapshot): void {
    if (!data || !Array.isArray(data.items)) return;
    latestClipboard = data;
    if (!data.enabled) clipList.replaceChildren();
    if (document.getElementById('isl')?.dataset.state !== 'open' || clipboard.inert || !clipboard.closest('.tab-panel.on')) return;
    const signature = category + data.enabled + JSON.stringify(data.items);
    if (signature === clipboardSignature) return; clipboardSignature = signature;
    const visible = data.items.filter(item => category === 'all' || clipCategory(item) === category);
    filters.hidden = !data.enabled;
    filters.replaceChildren();
    for (const [key, label] of (filters.hidden ? [] : CLIP_CATEGORIES)) {
      if (key !== 'all' && !data.items.some(item => clipCategory(item) === key)) continue;
      const pill = button(label, () => { category = key; clipboardStatus.textContent = ''; clipboardSignature = ''; renderClipboard(latestClipboard); filters.querySelector<HTMLButtonElement>(`[data-category="${key}"]`)?.focus(); });
      pill.dataset.category = key; pill.setAttribute('aria-pressed', String(category === key)); filters.append(pill);
    }
    clipboardEmpty.textContent = data.enabled ? (category === 'all' ? 'Nothing copied yet' : `No ${CLIP_CATEGORIES.find(([key]) => key === category)?.[1].toLowerCase()} copied yet`) : 'Enable clipboard history in Settings';
    clipboardEmpty.hidden = data.enabled && visible.length > 0;
    clearClipboard.disabled = !data.enabled || !data.items.length;
    const focus = (document.activeElement as HTMLElement)?.dataset.focus;
    clipList.replaceChildren();
    for (const item of [...visible].sort((a, b) => b.createdAt - a.createdAt).slice(0, 100)) {
      const row = node('div', 'clipboard-row');
      const kind = clipCategory(item); row.dataset.kind = kind;
      const copy = button(item.text.slice(0, 160), () => { void act(nativeAction('clipboard_copy', { id: item.id }).then(() => { clipboardStatus.textContent = 'Copied'; window.setTimeout(() => { if (clipboardStatus.textContent === 'Copied') clipboardStatus.textContent = ''; }, 1200); }), clipboardStatus, true); });
      if (kind === 'image' && item.thumbnail?.startsWith('data:image/png;base64,') && item.thumbnail.length < 132000) {
        const image = document.createElement('img'); image.src = item.thumbnail; image.alt = 'Copied image'; image.draggable = false;
        copy.replaceChildren(image); copy.setAttribute('aria-label', 'Copy image');
      } else if (kind === 'file' || kind === 'video') {
        const icon = node('span', 'clipboard-file-icon'); icon.innerHTML = kind === 'video' ? SVG_ICONS.video : SVG_ICONS.folder; icon.setAttribute('aria-hidden', 'true');
        const name = node('span', 'clipboard-file-name', item.text); copy.replaceChildren(icon, name);
        if (item.missing) copy.append(node('span', 'clipboard-missing', 'File no longer exists'));
      }
      copy.className = 'clipboard-text'; copy.dataset.focus = `copy-${item.id}`; copy.title = `Copy: ${item.text.slice(0, 500)}`;
      const pin = iconButton(item.pinned ? 'Unpin' : 'Pin', SVG_ICONS.pin, () => { void act(nativeAction('clipboard_pin', { id: item.id, pinned: !item.pinned }).then(refreshClipboard), clipboardStatus); }, 'row-action pin-btn');
      pin.setAttribute('aria-pressed', String(item.pinned)); pin.dataset.focus = `pin-${item.id}`;
      const remove = iconButton(`Remove ${kind}`, SVG_ICONS.close, () => { void act(nativeAction('clipboard_remove', { id: item.id }).then(refreshClipboard), clipboardStatus); }, 'row-action'); remove.dataset.focus = `remove-${item.id}`;
      row.dataset.pinned = String(item.pinned);
      const label = node('span', 'clipboard-type', kind === 'link' ? 'Link' : kind.charAt(0).toUpperCase() + kind.slice(1));
      row.append(copy, label, pin, remove); clipList.append(row);
    }
    if (focus) [...clipList.querySelectorAll<HTMLElement>('[data-focus]')].find(el => el.dataset.focus === focus)?.focus();
  }
  async function refreshClipboard(): Promise<void> { renderClipboard(await nativeService<ClipboardSnapshot>('clipboard_snapshot')); }

  async function act(action: Promise<unknown>, status: HTMLElement, keepStatus = false): Promise<void> {
    try { await action; if (!keepStatus) status.textContent = ''; } catch (error) { status.textContent = friendlyError(error); }
  }
  const unlisteners: (() => void)[] = [];
  for (const [event, handler] of [
    ['lyrics', applyLyrics], ['telemetry', (data: unknown) => telemetry(normalizeTelemetry(data))], ['dictation_state', (data: unknown) => dictation(normalizeDictation(data))], ['clipboard_history_changed', (data: unknown) => renderClipboard(normalizeClipboard(data))],
  ] as const) void listenToEvent(event, handler as (data: unknown) => void).then(unlisten => unlisteners.push(unlisten));
  void listenToEvent<DictationSnapshot>('dictation_partial', data => dictation(normalizeDictation(data))).then(unlisten => unlisteners.push(unlisten));
  void listenToEvent<{ enabled: boolean }>('lyrics-settings', data => {
    if (!data.enabled) { generation++; currentRequestId = ''; baseLines = []; lines = []; currentLyric = ''; lyricList.replaceChildren(); hideTiming(); lyricsStatus.textContent = 'Enable online lyrics in Settings'; syncLyricsAvailable(false); }
    else if (latestMedia?.active) void requestLyrics(latestMedia);
  }).then(unlisten => unlisteners.push(unlisten));
  void sessions();
  void nativeService<DictationSnapshot>('dictation_snapshot').then(dictation).catch(() => { dictationLine = 'Dictation service unavailable, see Settings'; paintSpeechStatus(); });
  void refreshClipboard().catch(() => { clipboardEmpty.textContent = 'Clipboard service unavailable, see Settings'; clipboardEmpty.hidden = false; });
  let interval: number | undefined;
  let sessionInterval: number | undefined;
  const visible = () => document.getElementById('isl')?.dataset.state === 'open';
  const syncTimers = () => {
    window.clearInterval(interval); window.clearInterval(sessionInterval);
    interval = undefined; sessionInterval = undefined;
    if (!visible()) return;
    if (!clipboard.inert && clipboard.closest('.tab-panel.on')) renderClipboard(latestClipboard);
    if (mediaPage.closest('.tab-panel.on')) { void sessions(); sessionInterval = window.setInterval(() => { void sessions(); }, 5000); }
    if (stats.inert || !stats.closest('.tab-panel.on')) return;
    void refreshTelemetry();
    interval = window.setInterval(() => {
      if (visible() && !stats.inert && stats.closest('.tab-panel.on')) void refreshTelemetry();
    }, 1000);
  };
  const sectionObserver = new MutationObserver(syncTimers);
  sectionObserver.observe(stats, {attributes: true, attributeFilter: ['inert']});
  sectionObserver.observe(clipboard, {attributes: true, attributeFilter: ['inert']});
  window.addEventListener('tuck-visibility', syncTimers);
  window.addEventListener('pagehide', () => {
    sectionObserver.disconnect();
    clearInterval(interval); clearInterval(sessionInterval);
    window.removeEventListener('tuck-visibility', syncTimers);
    unlisteners.forEach(unlisten => unlisten());
  }, { once: true });
  return {
    media(state, position) { latestMedia = state; updatePosition(position); if (state.trackId !== currentTrack || !state.active) { currentTrack = state.trackId; void requestLyrics(state); } },
    position: updatePosition, compactLyric: () => currentLyric,
  };
}
