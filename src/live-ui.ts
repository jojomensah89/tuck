import { listenToEvent, mediaSeek, isTauri } from './ipc.js';
import { nativeAction, nativeService, normalizeClipboard, normalizeDictation, normalizeTelemetry, type ClipboardSnapshot, type DictationSnapshot, type LyricsSnapshot, type MediaSession, type TelemetrySnapshot } from './live-services.js';
import { historyPath, lyricIndex, matchesLyricRequest, measuredPercent, newRequestId, parseLrc, readPreferences, TelemetryGate, type LyricLine } from './live-state.js';
import type { MediaState } from './types.js';

const node = <K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text = '') => {
  const el = document.createElement(tag); el.className = className; el.textContent = text; return el;
};
function button(label: string, action: () => void): HTMLButtonElement {
  const b = node('button', 'live-button', label); b.type = 'button'; b.onclick = e => { e.stopPropagation(); action(); }; return b;
}
function makePage(id: string, title: string): HTMLElement {
  const page = node('section', `page live-page ${id}`); page.dataset.page = id; page.setAttribute('aria-label', title);
  page.inert = true; page.setAttribute('aria-hidden', 'true'); document.querySelector('.pages-container')!.append(page); return page;
}
const friendlyError = (error: unknown) => error instanceof Error ? error.message : 'Action failed. Please try again.';

export function mountLivePages(onLayout: () => void): {
  media: (state: MediaState, position: number) => void;
  position: (position: number) => void;
  compactLyric: () => string;
} {
  const mediaPage = document.querySelector<HTMLElement>('.page.media')!;
  const toolbar = node('div', 'media-tools');
  const select = node('select', 'player-select'); select.setAttribute('aria-label', 'Media player');
  select.append(new Option('Current Windows player', ''));
  const message = node('p', 'live-note'); message.setAttribute('role', 'status');
  const lyricsPane = node('div', 'lyrics-pane'); lyricsPane.setAttribute('aria-label', 'Lyrics');
  const lyricList = node('div', 'lyric-lines');
  const lyricsStatus = node('p', 'lyrics-status', 'Enable online lyrics in Settings.');
  const lyricToggle = button('Lyrics', () => {
    mediaPage.classList.toggle('with-lyrics'); lyricToggle.setAttribute('aria-pressed', String(mediaPage.classList.contains('with-lyrics'))); onLayout();
  });
  lyricToggle.title = 'Show or hide lyrics';
  toolbar.append(select, lyricToggle); mediaPage.append(toolbar, lyricsPane, message);
  lyricsPane.append(lyricsStatus, lyricList);
  mediaPage.classList.toggle('with-lyrics', readPreferences().lyricsOpen);
  lyricToggle.setAttribute('aria-pressed', String(readPreferences().lyricsOpen));
  select.onchange = () => { void act(nativeAction('media_select_session', { id: select.value }), message); };
  let currentTrack = '', generation = 0, lines: LyricLine[] = [], activeLine = -2;
  let currentRequestId = '';
  let latestMedia: MediaState | null = null;
  let currentLyric = '', lastPosition = 0, manualScrollUntil = 0;
  lyricList.addEventListener('wheel', e => { e.stopPropagation(); manualScrollUntil = Date.now() + 4000; }, { passive: true });
  async function sessions(): Promise<void> {
    try {
      const list = await nativeService<MediaSession[]>('media_sessions');
      if (!Array.isArray(list)) return;
      const focused = document.activeElement === select;
      if (focused) return;
      select.replaceChildren(new Option('Automatic Windows player', ''));
      select.disabled = false;
      for (const session of list) {
        if (typeof session.id !== 'string' || typeof session.name !== 'string') continue;
        const option = new Option(session.name, session.id); option.selected = session.selected; select.append(option);
      }
    } catch { select.disabled = true; select.title = 'Player selection is unavailable in this build'; }
  }
  const updatePosition = (position: number) => {
    lastPosition = position;
    const index = lyricIndex(lines, position);
    currentLyric = index >= 0 ? lines[index].text : '';
    if (index === activeLine) return;
    activeLine = index;
    [...lyricList.children].forEach((el, i) => {
      el.classList.toggle('current', i === index);
      if (i === index) el.setAttribute('aria-current', 'true'); else el.removeAttribute('aria-current');
    });
    if (index >= 0 && Date.now() > manualScrollUntil) {
      const target = lyricList.children[index] as HTMLElement;
      lyricList.scrollTop = Math.max(0, target.offsetTop - lyricList.offsetTop - lyricList.clientHeight / 2 + target.offsetHeight / 2);
    }
  };
  function applyLyrics(snapshot: LyricsSnapshot): void {
    if (!snapshot || !matchesLyricRequest({ trackId: currentTrack, requestId: currentRequestId }, snapshot)) return;
    lines = snapshot.lines || parseLrc(snapshot.syncedLyrics || ''); activeLine = -2; currentLyric = ''; lyricList.replaceChildren();
    const statuses: Record<LyricsSnapshot['status'], string> = {
      disabled: 'Enable online lyrics in Settings.', loading: 'Finding lyrics…', synced: '', plain: 'Lyrics · not synchronized',
      instrumental: 'Instrumental', unavailable: 'No lyrics found for this track.', error: 'Lyrics unavailable. Check your connection and try another track.',
    };
    lyricsStatus.textContent = statuses[snapshot.status] ?? 'Lyrics unavailable.';
    if (lines.length) {
      for (const line of lines) {
        const b = button(line.text || '♪', () => { void act(mediaSeek(line.atMs), message); });
        b.className = 'lyric-line'; b.title = 'Seek to this line'; lyricList.append(b);
      }
    } else if (snapshot.plainLyrics) {
      const plain = node('p', 'plain-lyrics', snapshot.plainLyrics.slice(0, 250000)); lyricList.append(plain);
    }
    updatePosition(lastPosition);
  }
  async function requestLyrics(state: MediaState): Promise<void> {
    const request = ++generation;
    const requestId = newRequestId(); currentRequestId = state.active ? requestId : '';
    lines = []; currentLyric = ''; activeLine = -2; lyricList.replaceChildren();
    lyricsStatus.textContent = state.active ? 'Finding lyrics…' : 'Play music to see lyrics.';
    if (!state.active) return;
    try {
      const data = await nativeService<LyricsSnapshot>('lyrics_lookup', {
        trackId: state.trackId, requestId, title: state.title, artist: state.artist, durationMs: state.durationMs,
      });
      if (request === generation) applyLyrics(data);
    } catch { if (request === generation) lyricsStatus.textContent = isTauri() ? 'Lyrics unavailable. Enable lyrics in Settings.' : 'Online lyrics require the Windows app.'; }
  }

  const stats = makePage('stats', 'System usage');
  const statHead = node('div', 'live-heading'); statHead.append(node('h2', '', 'System usage'));
  const statsStatus = node('span', 'live-note', 'Waiting for native samples'); statHead.append(statsStatus); stats.append(statHead);
  const statRows = node('div', 'stat-rows'); stats.append(statRows);
  const histories = new Map<string, (number | null)[]>();
  const telemetryGate = new TelemetryGate();
  const rowNodes = new Map<string, { row: HTMLElement; value: HTMLElement; path: SVGPathElement }>();
  function metric(id: string, label: string, pct: number | null, detail = ''): void {
    let view = rowNodes.get(id);
    if (!view) {
      const row = node('div', 'stat-row'); const name = node('span', 'stat-name', label); const value = node('span', 'stat-value');
      const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); svg.setAttribute('viewBox', '0 0 180 40'); svg.setAttribute('aria-hidden', 'true');
      const path = document.createElementNS(svg.namespaceURI, 'path') as SVGPathElement; svg.append(path); row.append(name, value, svg); statRows.append(row);
      view = { row, value, path }; rowNodes.set(id, view);
    }
    const history = histories.get(id) || []; history.push(pct); if (history.length > 45) history.shift(); histories.set(id, history);
    view.value.textContent = pct === null ? 'Unavailable' : `${Math.round(pct)}%${detail ? ` · ${detail}` : ''}`;
    view.path.setAttribute('d', historyPath(history));
  }
  const bytes = (value: number) => `${(value / 1073741824).toFixed(1)} GB`;
  function telemetry(data: TelemetrySnapshot): void {
    if (!data || !Array.isArray(data.gpus)) return;
    const sampled = Number.isFinite(data.sampledAt) ? data.sampledAt : 0;
    if (!telemetryGate.accept(sampled)) return;
    const stale = data.stale || !sampled || Date.now() - sampled > 10000;
    if (stale) { invalidateTelemetry('Stale / warming up'); return; }
    statsStatus.textContent = stale ? 'Stale / warming up' : `Live · ${new Date(sampled).toLocaleTimeString()}`;
    metric('cpu', 'CPU', measuredPercent(data.cpuPct));
    const validRam = typeof data.ramUsedBytes === 'number' && typeof data.ramTotalBytes === 'number' && data.ramTotalBytes > 0 && data.ramUsedBytes >= 0;
    metric('ram', 'Memory', validRam ? measuredPercent(data.ramUsedBytes! / data.ramTotalBytes! * 100) : null, validRam ? `${bytes(data.ramUsedBytes!)} / ${bytes(data.ramTotalBytes!)}` : '');
    const ids = new Set(['cpu', 'ram']);
    for (const gpu of data.gpus) { ids.add(gpu.id); metric(gpu.id, gpu.name, measuredPercent(gpu.usedPct)); }
    if (!data.gpus.length) { ids.add('gpu-unknown'); metric('gpu-unknown', 'GPU', null); }
    for (const [id, view] of rowNodes) if (!ids.has(id)) { view.row.remove(); rowNodes.delete(id); histories.delete(id); }
  }
  metric('cpu', 'CPU', null); metric('ram', 'Memory', null); metric('gpu-unknown', 'GPU', null);
  function invalidateTelemetry(label: string): void {
    statsStatus.textContent = label;
    for (const [id, view] of rowNodes) {
      view.value.textContent = 'Unavailable';
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
      if (telemetryGate.sampledAt === startedAtSample) invalidateTelemetry('Native telemetry unavailable');
    } finally { telemetryGate.finish(); }
  }

  const speech = makePage('dictation', 'Dictation');
  const speechTitle = node('h2', '', 'Dictation'); const speechStatus = node('p', 'live-note', 'Offline · set up a model in Settings');
  const transcript = node('div', 'transcript', 'Your transcript appears here.'); transcript.setAttribute('role', 'status');
  const speechActions = node('div', 'live-actions');
  const record = button('Record', () => { void act(nativeAction('dictation_toggle'), speechStatus); });
  let dictationPhase = 'disabled';
  const stop = button('Finish', () => { void act(nativeAction('dictation_stop'), speechStatus); });
  const cancel = button('Cancel', () => { void act(nativeAction('dictation_cancel'), speechStatus); });
  const insert = button('Insert', () => { void act(nativeAction('dictation_insert'), speechStatus); }); insert.disabled = true;
  speechActions.append(record, stop, cancel, insert); speech.append(speechTitle, speechStatus, transcript, speechActions);
  function dictation(data: DictationSnapshot): void {
    if (!data || typeof data.status !== 'string') return;
    dictationPhase = data.status;
    speechStatus.textContent = data.error || `Offline · ${data.status}`;
    transcript.textContent = data.transcript?.slice(0, 32000) || (data.status === 'recording' ? 'Listening…' : 'Your transcript appears here.');
    insert.disabled = data.status !== 'review' || data.canInsert === false; stop.disabled = data.status !== 'recording';
    record.disabled = !['idle', 'recording', 'review', 'error'].includes(data.status) || data.modelReady === false;
    record.textContent = data.status === 'recording' ? 'Finish recording' : 'Record';
    document.getElementById('isl')!.dataset.dictation = data.status;
    if (['recording', 'transcribing', 'review'].includes(data.status)) {
      window.dispatchEvent(new CustomEvent('tuck-activity', { detail: { id: 'dictation', active: true, title: 'Dictation', text: data.transcript?.slice(-65) || data.status, priority: 99 } }));
    } else window.dispatchEvent(new CustomEvent('tuck-activity', { detail: { id: 'dictation', active: false, title: '', text: '', priority: 99 } }));
  }

  const clipboard = makePage('clipboard', 'Clipboard history');
  const clipboardHead = node('div', 'live-heading'); const clipboardStatus = node('p', 'live-note', 'Enable clipboard history in Settings.');
  clipboardHead.append(node('h2', '', 'Clipboard'), button('Clear', () => { void act(nativeAction('clipboard_clear').then(refreshClipboard), clipboardStatus); }));
  const clipList = node('div', 'clipboard-list'); clipboard.append(clipboardHead, clipboardStatus, clipList);
  let clipboardSignature = '';
  function renderClipboard(data: ClipboardSnapshot): void {
    if (!data || !Array.isArray(data.items)) return;
    clipboardStatus.textContent = data.enabled ? (data.items.length ? 'Select to copy · pins stay at the top' : 'Nothing copied yet.') : 'Enable clipboard history in Settings.';
    const signature = JSON.stringify(data.items);
    if (signature === clipboardSignature) return; clipboardSignature = signature;
    const focus = (document.activeElement as HTMLElement)?.dataset.focus;
    clipList.replaceChildren();
    for (const item of [...data.items].sort((a, b) => Number(b.pinned) - Number(a.pinned)).slice(0, 100)) {
      const row = node('div', 'clipboard-row');
      const copy = button(item.text.slice(0, 160), () => { void act(nativeAction('clipboard_copy', { id: item.id }), clipboardStatus); });
      copy.classList.add('clipboard-text'); copy.dataset.focus = `copy-${item.id}`; copy.title = item.text.slice(0, 500);
      const pin = button(item.pinned ? 'Unpin' : 'Pin', () => { void act(nativeAction('clipboard_pin', { id: item.id, pinned: !item.pinned }).then(refreshClipboard), clipboardStatus); });
      pin.setAttribute('aria-pressed', String(item.pinned)); pin.dataset.focus = `pin-${item.id}`;
      const remove = button('Remove', () => { void act(nativeAction('clipboard_remove', { id: item.id }).then(refreshClipboard), clipboardStatus); }); remove.dataset.focus = `remove-${item.id}`;
      row.append(copy, pin, remove); clipList.append(row);
    }
    if (focus) [...clipList.querySelectorAll<HTMLElement>('[data-focus]')].find(el => el.dataset.focus === focus)?.focus();
  }
  async function refreshClipboard(): Promise<void> { renderClipboard(await nativeService<ClipboardSnapshot>('clipboard_snapshot')); }

  // Frequency bars are driven exclusively by actual native loopback measurements.
  const spectrum = node('div', 'live-spectrum'); spectrum.setAttribute('aria-label', 'System audio spectrum');
  const bars = Array.from({ length: 32 }, () => { const bar = node('i'); spectrum.append(bar); return bar; }); mediaPage.append(spectrum);
  const audioPage = makePage('visualizer', 'Audio visualizer');
  const audioTitle = node('div', 'live-heading'); const audioStatus = node('p', 'live-note', 'Playback analysis is off until enabled in Settings.');
  audioTitle.append(node('h2', '', 'System audio'), button('Enable', () => { void act(nativeAction('visualizer_set_enabled', { enabled: true }), audioStatus); }), button('Stop', () => { void act(nativeAction('visualizer_set_enabled', { enabled: false }), audioStatus); }));
  const wideSpectrum = node('div', 'wide-spectrum'); wideSpectrum.setAttribute('aria-label', 'Measured playback frequency bands');
  const wideBars = Array.from({ length: 48 }, () => { const bar = node('i'); wideSpectrum.append(bar); return bar; });
  audioPage.append(audioTitle, wideSpectrum, audioStatus);
  let lastAudio = 0;
  function audio(data: { bands: number[]; active?: boolean }): void {
    if (!data || !Array.isArray(data.bands)) return;
    lastAudio = Date.now();
    audioStatus.textContent = data.active === false ? 'Silence · no audio stored' : 'Live playback spectrum · no audio stored';
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
    if (dictationPhase !== 'recording' || typeof data?.text !== 'string') return;
    transcript.textContent = data.text.slice(0, 32000);
    window.dispatchEvent(new CustomEvent('tuck-activity', { detail: { id: 'dictation', active: true, title: 'Dictation', text: data.text.slice(-65), priority: 99 } }));
  }).then(unlisten => unlisteners.push(unlisten));
  void listenToEvent<{ enabled: boolean }>('lyrics-settings', data => {
    if (!data.enabled) { generation++; currentRequestId = ''; lines = []; currentLyric = ''; lyricList.replaceChildren(); lyricsStatus.textContent = 'Enable online lyrics in Settings.'; }
    else if (latestMedia?.active) void requestLyrics(latestMedia);
  }).then(unlisten => unlisteners.push(unlisten));
  void listenToEvent<string>('visualizer-error', () => { audioStatus.textContent = 'Playback analysis unavailable. Check the output device.'; }).then(unlisten => unlisteners.push(unlisten));
  void sessions();
  void refreshTelemetry();
  void nativeService<DictationSnapshot>('dictation_snapshot').then(dictation).catch(() => { speechStatus.textContent = 'Dictation service unavailable. See Settings.'; });
  void refreshClipboard().catch(() => { clipboardStatus.textContent = 'Clipboard service unavailable. See Settings.'; });
  const interval = window.setInterval(() => {
    if (Date.now() - lastAudio > 1000) [...bars, ...wideBars].forEach(bar => bar.style.setProperty('--level', '0'));
    if (telemetryGate.sampledAt && Date.now() - telemetryGate.sampledAt > 10000) invalidateTelemetry('Stale / warming up');
    if (document.querySelector('.page.on') === stats) void refreshTelemetry();
  }, 1000);
  const sessionInterval = window.setInterval(() => { void sessions(); }, 5000);
  window.addEventListener('pagehide', () => { clearInterval(interval); clearInterval(sessionInterval); unlisteners.forEach(unlisten => unlisten()); }, { once: true });
  return {
    media(state, position) { latestMedia = state; updatePosition(position); if (state.trackId !== currentTrack || !state.active) { currentTrack = state.trackId; void requestLyrics(state); } },
    position: updatePosition, compactLyric: () => currentLyric,
  };
}

