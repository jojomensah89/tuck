import { UsageSnapshotGate } from './usage-snapshot.js';
import {mountExtensions} from './extensions.js';
import { mountLivePages } from './live-ui.js';
import { LIVE_SETTINGS_KEY, readPreferences, writePreferences } from './live-state.js';
import { mountTabRail } from './tab-rail.js';
import './live.css';
import {
  isTauri,
  listenToEvent,
  mediaToggle,
  mediaNext,
  mediaPrev,
  mediaSeek,
  setHit,
  fetchUsageNow,
  openSettingsCmd,
  triggerQuickAction,
  dispatchMockEvent,
  invokeCommand,
} from './ipc.js';
import { currentPositionMs, retainedArtwork, hitGeometry } from './runtime.js';
import { extractAccentColor } from './accent.js';
import { batteryGlyph, SVG_ICONS } from './icons.js';
import { activitiesManager } from './activities.js';
import { renderUsageRings } from './usage-rings.js';
import { activityProgress, batteryTone, createRing, formatSpan } from './rings.js';
import { compactScrollMs, compactShellWidth, selectCompactActivity, type CompactActivityCandidate } from './compact-slots.js';
import { formatClockTime } from './time-format.js';
import {
  AnnouncementQueue,
  HUD_ANNOUNCE_QUIET_MS,
  HudModel,
  hudLabel,
  levelEventShowsHud,
  levelInputApplies,
  peekReturnTarget,
  readHudPayload,
  stepVolume,
  volumeGlyph,
  wheelVolumeDelta,
  type HudReading,
} from './hud.js';
import {
  STORAGE_KEYS,
  mergeMeters,
  connectedUsageMeters,
  checkResets,
  logChatGPTMessage,
} from './usage.js';
import type {
  IslandState,
  ShapeStyle,
  MediaState,
  PowerState,
  Meter,
  Activity,
} from './types.js';

/* ================= Geometry & Config ================= */

const SIZES: Record<IslandState, [number, number]> = {
  idle: [212, 32],
  peek: [300, 38],
  open: [440, 210],
};

/** The width the idle pill last asked for; it changes with what owns the slots */
let idleWidth = SIZES.idle[0];

/** The strip of the open island that keeps acting like the pill it would close to */
let pillStrip = { width: SIZES.idle[0], height: SIZES.idle[1] };

let livePreferences = readPreferences();

function $(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`Element #${id} not found`);
  return el;
}

const isl = $('isl');
isl.style.setProperty('--pill-w', `${pillStrip.width}px`);
isl.style.setProperty('--pill-h', `${pillStrip.height}px`);
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
// Set once the live pages mount; renderCompact can run before that during startup
let compactLyric: () => string = () => '';
// The compact title is measured mid-morph on state changes; refit once the shell settles
isl.addEventListener('transitionend', e => {
  if (e.target === isl && (e.propertyName === 'width' || e.propertyName === 'height')) fitCompactTitle();
});

/* ================= Diffed DOM writes ================= */

// Live content is rewritten on 200ms/1s ticks. Writing only on change keeps
// CSS animations (eq bars, marquee) running and preserves hover state.
const slotMarkup = new WeakMap<Element, string>();

function setText(el: HTMLElement, text: string): void {
  if (!slotMarkup.has(el) && el.textContent === text) return;
  slotMarkup.delete(el);
  el.textContent = text;
}

function setMarkup(el: HTMLElement, html: string): void {
  if (slotMarkup.get(el) === html) return;
  slotMarkup.set(el, html);
  el.innerHTML = html;
}

interface CompactSlotRenderState {
  key: string;
  signature: string;
  layer: HTMLElement;
}

const compactSlotState = new WeakMap<HTMLElement, CompactSlotRenderState>();

function setCompactSlot(
  slot: HTMLElement,
  key: string,
  signature: string,
  build: () => HTMLElement,
): boolean {
  const previous = compactSlotState.get(slot);
  if (previous?.key === key) {
    if (previous.signature !== signature) {
      previous.layer.replaceChildren(build());
      previous.signature = signature;
      return true;
    }
    return false;
  }

  const layer = document.createElement('span');
  layer.className = 'compact-slot current';
  layer.append(build());

  if (previous) {
    previous.layer.classList.remove('current');
    previous.layer.classList.add('leaving');
    layer.classList.add('entering');
    previous.layer.addEventListener('transitionend', () => previous.layer.remove(), { once: true });
    window.setTimeout(() => previous.layer.remove(), 320);
    requestAnimationFrame(() => layer.classList.remove('entering'));
  }

  slot.append(layer);
  compactSlotState.set(slot, { key, signature, layer });
  return true;
}

function compactText(text: string, className = ''): HTMLElement {
  const span = document.createElement('span');
  span.className = className;
  span.textContent = text;
  return span;
}

function compactMarkup(markup: string, className = ''): HTMLElement {
  const span = document.createElement('span');
  span.className = className;
  span.innerHTML = markup;
  return span;
}

function compactIndicator(
  markup: string,
  ariaLabel: string,
  hasBackgroundActivity: boolean,
): HTMLElement {
  const indicator = compactMarkup(markup, 'compact-indicator');
  indicator.setAttribute('role', 'img');
  indicator.setAttribute(
    'aria-label',
    hasBackgroundActivity ? `${ariaLabel}, another activity is running` : ariaLabel,
  );
  if (hasBackgroundActivity) {
    const dot = document.createElement('span');
    dot.className = 'live-activity-dot';
    dot.setAttribute('aria-hidden', 'true');
    indicator.append(dot);
  }
  return indicator;
}

function runAction(action: Promise<unknown>, label: string): void {
  void action.catch(() => {
    const message = `${label} failed, try again`;
    $('statusMessage').textContent = message;
    setText($('cc'), message);
    if (state === 'open') $('artist').textContent = message;
  });
}

/* ================= Shape Setup ================= */

const storedShape = (() => {
  try {
    return localStorage.getItem(STORAGE_KEYS.SHAPE) as ShapeStyle | null;
  } catch {
    return null;
  }
})();

let shape: ShapeStyle = storedShape === 'float' ? 'float' : 'notch';
let topOffset = shape === 'float' ? 10 : 0;

function applyShape(newShape: ShapeStyle, refresh = true): void {
  shape = newShape === 'float' ? 'float' : 'notch';
  topOffset = shape === 'float' ? 10 : 0;
  isl.dataset.shape = shape;
  isl.style.setProperty('--top', `${topOffset}px`);
  if (refresh) {
    setState(state);
  }
}

applyShape(shape, false);

window.addEventListener('storage', (e) => {
  if (e.key === STORAGE_KEYS.SHAPE && e.newValue) {
    applyShape(e.newValue as ShapeStyle);
  }
});

/* ================= State Machine ================= */

let state: IslandState = 'idle';
const usageSnapshots = new UsageSnapshotGate();
let seekInterval: number | undefined;
let openInterval: number | undefined;
let compactClockTimeout: number | undefined;
function syncIdleTimers(): void {
  window.clearInterval(seekInterval);
  window.clearInterval(openInterval);
  window.clearTimeout(compactClockTimeout);
  seekInterval = undefined;
  openInterval = undefined;
  if (mediaState.active && mediaState.playing) seekInterval = window.setInterval(tick, 200);
  if (state === 'open') {
    tick(); renderToday(); renderMediaHeader(); renderUsage();
    void refreshUsageSnapshot();
    openInterval = window.setInterval(() => { renderToday(); renderMediaHeader(); renderUsage(); }, 1000);
  }
  if (state !== 'open' && !mediaState.playing) {
    compactClockTimeout = window.setTimeout(() => {
      renderCompact();
      syncIdleTimers();
    }, 60000 - Date.now() % 60000 + 10);
  }
  window.dispatchEvent(new Event('tuck-visibility'));
}

let closeTimer = 0;
let openTimer = 0;
let hoverInside = false;
let nativeDragActive = false;
let keyboardMode = false;
let announcement: { text: string; until: number } | null = null;
const hud = new HudModel();
/** Level feedback while the island is open; it has no shell to morph */
const openLevel = new HudModel();
const levelStrip = $('levelStrip');
let openLevelTimer = 0;
/** The Windows flyout is minimized right now, so the island is the only level feedback */
let flyoutHidden = false;
/** The last hand-back request sent to native */
let flyoutPause = '';
const pendingAnnouncements = new AnnouncementQueue();
/** The current peek interrupted the open island, which comes back when the peek ends */
let returnOpenAfterPeek = false;
let lastChargerChange: number | null = null;
let hudTimer = 0;
let hudAnnounceTimer = 0;
interface ExternalActivityState {
  audioLevel: number;
  title: string;
  text: string;
  priority: number;
  type: string;
  phase: string;
}
const externalActivities = new Map<string, ExternalActivityState>();

function setState(next: IslandState): void {
  if (state === 'open' && next !== 'open' && isTauri()) runAction(invokeCommand('deactivate_island'), 'Restore desktop interaction');
  if (next !== 'open') keyboardMode = false;
  if (next !== 'peek') returnOpenAfterPeek = false;
  // The value still reaches the status region; only the painted strip goes
  if (next !== 'open') delete levelStrip.dataset.on;
  state = next;
  syncFlyoutPause();
  if (next === 'idle') idleWidth = compactShellWidth(selectCompactActivity(compactCandidates()).primary, livePreferences.compactContent);
  const [w, h] = next === 'open'
    ? [livePreferences.openWidth, 210]
    : next === 'idle'
      ? [idleWidth, SIZES.idle[1]]
      : SIZES.peek;
  isl.style.setProperty('--w', `${w}px`);
  isl.style.setProperty('--h', `${h}px`);
  isl.style.setProperty('--open-w', `${livePreferences.openWidth}px`);
  isl.style.width = `${w}px`;
  isl.style.height = `${h}px`;
  isl.dataset.state = next;
  if (isTauri()) void invokeCommand('island_activity', { active: next !== 'idle' || hoverInside });
  syncIdleTimers();
  if (next === 'peek' && hud.visible(Date.now())) isl.dataset.hud = hud.reading!.kind;
  else delete isl.dataset.hud;

  const full = document.querySelector<HTMLElement>('.full')!;
  const compact = document.querySelector<HTMLElement>('.compact')!;
  if (next !== 'open' && full.contains(document.activeElement)) isl.focus();
  full.inert = next !== 'open';
  full.setAttribute('aria-hidden', String(next !== 'open'));
  compact.setAttribute('aria-hidden', String(next === 'open'));
  isl.setAttribute('aria-expanded', String(next === 'open'));

  const hit = hitGeometry(w, h, topOffset);
  runAction(setHit(hit.w, hit.h), 'Update island bounds');

  renderCompact();
  if (next === 'open') {
    fitTitle();
    renderMediaHeader();
    tick();
  }
}

/**
 * The Windows flyout comes back while the HUD switch is off (the island shows nothing)
 * or the island is pinned open (it stays in use, and the in-strip level is easy to miss
 * there, so both show). A peek over a pinned island is only a pause in being open, so
 * it does not count as leaving.
 */
function syncFlyoutPause(): void {
  if (!isTauri()) return;
  const hudOff = !livePreferences.levelHud;
  const pinnedOpen = livePreferences.pinned && (state === 'open' || returnOpenAfterPeek);
  const key = `${hudOff}:${pinnedOpen}`;
  if (key === flyoutPause) return;
  flyoutPause = key;
  void invokeCommand('set_flyout_island_pause', { hudOff, pinnedOpen }).catch(() => { flyoutPause = ''; });
}

function onHover(inside: boolean): void {
  hoverInside = inside;
  if (isTauri()) void invokeCommand('island_activity', { active: inside || state !== 'idle' });
  window.clearTimeout(closeTimer);
  window.clearTimeout(openTimer);
  // The HUD owns the shell until it times out; hover is re-read when it leaves
  if (state !== 'open' && hud.visible(Date.now())) return;
  if (nativeDragActive || (!inside && (keyboardMode || livePreferences.pinned) && state === 'open')) return;

  if (inside) {
    if (livePreferences.hoverExpand) openTimer = window.setTimeout(() => setState('open'), livePreferences.openDelayMs);
    else if (state === 'idle') setState('peek');
  } else {
    const delay = state === 'open' ? livePreferences.closeDelayMs : 0;
    if (delay === 0) setState('idle');
    else closeTimer = window.setTimeout(() => setState('idle'), delay);
  }
}

// Mouse events inside browser preview
if (!isTauri()) {
  isl.addEventListener('mouseenter', () => onHover(true));
  isl.addEventListener('mouseleave', () => onHover(false));
}

isl.addEventListener('click', (e) => {
  if ((e.target as HTMLElement).closest('button, .track')) return;
  if (state !== 'open') {
    setState('open');
  }
});

mountExtensions();
const live = mountLivePages(() => setState(state));
compactLyric = live.compactLyric;
const rail = mountTabRail({
  initial: livePreferences.lastTab,
  isOpen: () => state === 'open',
  onChange: id => {
    window.dispatchEvent(new Event('tuck-visibility'));
    if (id !== 'media') return;
    fitTitle();
    renderMediaHeader();
  },
  onUserChange: id => { livePreferences = writePreferences({ lastTab: id }); },
  openSettings: () => runAction(openSettingsCmd(), 'Open settings'),
});

window.addEventListener('storage', event => {
  if (event.key !== LIVE_SETTINGS_KEY) return;
  livePreferences = readPreferences();
  if (isTauri()) void invokeCommand('set_fullscreen_auto_hide', { enabled: livePreferences.fullscreenAutoHide });
  document.querySelector('.page.media')?.classList.toggle('with-lyrics', livePreferences.lyricsOpen);
  document.querySelector<HTMLButtonElement>('button[aria-label="Toggle lyrics"], .media-tools button')?.setAttribute('aria-pressed', String(livePreferences.lyricsOpen));
  setState(state);
});

isl.addEventListener('keydown', (e) => {
  const target = e.target as HTMLElement | null;
  if (target && target.closest('input, textarea, select, [contenteditable=true], [role=slider]')) {
    return;
  }

  if (e.key === 'Enter' && state !== 'open') {
    e.preventDefault();
    keyboardMode = true;
    setState('open');
  } else if (e.key === 'Escape') {
    e.preventDefault();
    keyboardMode = false;
    setState('idle');
  } else if (state === 'open' && (e.key === 'ArrowRight' || e.key === 'ArrowLeft')) {
    // Anywhere in the open island; the rail itself handles its own roving keys
    e.preventDefault();
    rail.step(e.key === 'ArrowRight' ? 1 : -1, 'key');
  }
});

/* ================= Media ================= */

let mediaState: MediaState = {
  active: false,
  trackId: '',
  title: '',
  artist: '',
  playing: false,
  positionMs: 0,
  durationMs: 0,
};
let mediaReceivedAt = 0;
/** How long a paused track keeps the pill, so a pause between two tracks does not resize it */
const PAUSE_HOLD_MS = 1000;
let pauseHold = false;
let pauseHoldTimer = 0;
let currentArtwork: string | null = null;

function formatMs(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const m = Math.floor(s / 60);
  const sec = s % 60;
  return `${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}`;
}

function batteryAriaLabel(): string {
  if (!powerState.hasBattery && powerState.statusKnown !== false) return 'No system battery';
  if (powerState.statusKnown === false || powerState.percentKnown === false || !Number.isFinite(powerState.percent)) {
    return 'Battery level unavailable';
  }
  return `Battery ${Math.max(0, Math.min(100, Math.round(powerState.percent)))} percent${powerState.charging ? ', charging' : ''}`;
}

function buildBatteryStatus(): HTMLElement {
  const status = compactMarkup(batteryGlyph(
    powerState.percent,
    powerState.charging,
    powerState.percentKnown !== false && powerState.statusKnown !== false,
    powerState.hasBattery,
  ), 'battery-status');
  status.setAttribute('role', 'img');
  status.setAttribute('aria-label', batteryAriaLabel());
  return status;
}

function renderMediaHeader(): void {
  const badgeEl = document.getElementById('mediaBattBadge');
  if (!badgeEl) return;
  setMarkup(badgeEl, batteryGlyph(
    powerState.percent,
    powerState.charging,
    powerState.percentKnown !== false && powerState.statusKnown !== false,
    powerState.hasBattery,
  ));
  badgeEl.setAttribute('role', 'img');
  badgeEl.setAttribute('aria-label', batteryAriaLabel());
}

function setAccentColor(rgb: { r: number; g: number; b: number } | null): void {
  if (rgb) {
    isl.style.setProperty('--accent', `rgb(${rgb.r}, ${rgb.g}, ${rgb.b})`);
    isl.style.setProperty('--art-wash', `rgba(${rgb.r}, ${rgb.g}, ${rgb.b}, 0.65)`);
  } else {
    isl.style.setProperty('--accent', '#ffffff');
    isl.style.setProperty('--art-wash', 'transparent');
  }
}

function applyArtwork(): void {
  const artEl = $('art');
  if (currentArtwork) {
    artEl.replaceChildren();
    artEl.style.backgroundImage = `url("${currentArtwork}")`;
    const sampledArtwork = currentArtwork;
    extractAccentColor(currentArtwork, (color) => {
      if (currentArtwork === sampledArtwork) setAccentColor(color);
    });
  } else {
    artEl.style.backgroundImage = '';
    artEl.innerHTML = SVG_ICONS.note;
    setAccentColor(null);
  }
}

function fitTitle(): void {
  requestAnimationFrame(() => {
    const titleEl = $('title');
    const span = titleEl.firstElementChild as HTMLElement | null;
    if (!span) return;
    const overflow = span.scrollWidth - titleEl.clientWidth;
    titleEl.classList.toggle('scroll', overflow > 2);
    titleEl.style.setProperty('--shift', `-${overflow}px`);
  });
}

function renderMedia(): void {
  const titleSpan = $('title').firstElementChild;
  if (titleSpan) {
    titleSpan.textContent = mediaState.active ? mediaState.title : 'Nothing playing';
  }
  $('artist').textContent = mediaState.active ? mediaState.artist : 'Play something in any app';
  renderMediaHeader();
  applyArtwork();
  fitTitle();
}

function syncTransportButtons(): void {
  const playBtn = $('play');
  setMarkup(playBtn, mediaState.playing ? SVG_ICONS.pauseBig : SVG_ICONS.playBig);
  playBtn.setAttribute('aria-label', mediaState.playing ? 'Pause' : 'Play');
}

function syncPauseHold(wasPlaying: boolean, next: Pick<MediaState, 'active' | 'playing'>): void {
  if (next.playing || !next.active) {
    window.clearTimeout(pauseHoldTimer);
    pauseHold = false;
  } else if (wasPlaying) {
    window.clearTimeout(pauseHoldTimer);
    pauseHold = true;
    pauseHoldTimer = window.setTimeout(() => {
      pauseHold = false;
      renderCompact();
    }, PAUSE_HOLD_MS);
  }
}

function onMediaUpdate(payload: MediaState): void {
  const { artwork, ...rest } = payload;
  syncPauseHold(mediaState.playing, rest);
  const trackChanged = rest.trackId !== mediaState.trackId || rest.active !== mediaState.active;

  mediaState = { ...rest };
  mediaReceivedAt = performance.now();

  // "null artwork means unchanged unless trackId changed"
  currentArtwork = retainedArtwork(currentArtwork, trackChanged, artwork);

  if (trackChanged || artwork) {
    renderMedia();
  }

  syncTransportButtons();
  ($('play') as HTMLButtonElement).disabled = !mediaState.active || mediaState.canPlayPause === false;
  ($('prev') as HTMLButtonElement).disabled = !mediaState.active || mediaState.canSkipPrev === false;
  ($('next') as HTMLButtonElement).disabled = !mediaState.active || mediaState.canSkipNext === false;
  live.media(mediaState, currentPosition());
  syncIdleTimers();
  renderCompact();
}

function currentPosition(): number {
  if (!mediaState.active) return 0;
  return currentPositionMs(mediaState, mediaReceivedAt, performance.now());
}

function tick(): void {
  live.position(currentPosition());
  if (state !== 'open') renderCompact();
  if (state !== 'open' || rail.active() !== 'media') return;
  const pos = currentPosition();
  setText($('cur'), formatMs(pos));
  setText($('left'), mediaState.durationMs > 0 ? `-${formatMs(mediaState.durationMs - pos)}` : 'Live');
  const pct = mediaState.durationMs > 0 ? (pos / mediaState.durationMs) * 100 : 0;
  // The fill is a full-width bar translated into place (transform only, no layout)
  $('fill').style.setProperty('--p', String(Math.min(100, Math.max(0, pct)) / 100));

  const trackSlider = $('track');
  trackSlider.setAttribute('aria-valuenow', String(Math.round(pct)));
  trackSlider.setAttribute('aria-valuetext', `${formatMs(pos)} of ${formatMs(mediaState.durationMs)}`);
  trackSlider.setAttribute('aria-disabled', String(!mediaState.active || mediaState.canSeek === false || mediaState.durationMs <= 0));
}



$('play').onclick = (e) => {
  e.stopPropagation();
  runAction(mediaToggle(), 'Change playback');
};

$('next').onclick = (e) => {
  e.stopPropagation();
  runAction(mediaNext(), 'Next track');
};

$('prev').onclick = (e) => {
  e.stopPropagation();
  if (currentPosition() > 3000) {
    seekTo(0);
  } else {
    runAction(mediaPrev(), 'Previous track');
  }
};

function seekTo(ms: number): void {
  if (!mediaState.active || mediaState.canSeek === false || mediaState.durationMs <= 0) return;
  mediaState.positionMs = Math.max(0, Math.min(mediaState.durationMs, ms));
  mediaReceivedAt = performance.now();
  tick();
  runAction(mediaSeek(mediaState.positionMs), 'Seek track');
}
const slider = $('track');
let scrubbing = false;
function seekFromPointer(e: PointerEvent): void {
  const rect = slider.getBoundingClientRect();
  if (rect.width > 0) seekTo((e.clientX - rect.left) / rect.width * mediaState.durationMs);
}
slider.addEventListener('pointerdown', (e) => {
  e.stopPropagation();
  if (!mediaState.active || mediaState.durationMs <= 0) return;
  scrubbing = true;
  slider.setPointerCapture(e.pointerId);
  seekFromPointer(e);
});
slider.addEventListener('pointermove', (e) => { if (scrubbing) seekFromPointer(e); });
slider.addEventListener('pointerup', (e) => {
  if (!scrubbing) return;
  seekFromPointer(e);
  scrubbing = false;
  if (slider.hasPointerCapture(e.pointerId)) slider.releasePointerCapture(e.pointerId);
});
slider.addEventListener('pointercancel', () => { scrubbing = false; });
slider.addEventListener('keydown', (e) => {
  if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
  e.preventDefault();
  e.stopPropagation();
  seekTo(e.key === 'Home' ? 0 : e.key === 'End' ? mediaState.durationMs : currentPosition() + (e.key === 'ArrowRight' ? 5000 : -5000));
});
new ResizeObserver(fitTitle).observe($('title'));

/* ================= Compact View Arbitration ================= */

function compactTitle(text: string): HTMLElement {
  const title = document.createElement('span');
  title.className = 'compact-title';
  const track = compactText(text, 'compact-title-track');
  title.append(track);
  return title;
}

/** Writes an accessible label only when it changes, so the 200ms tick doesn't churn the tree. */
function setLabel(el: Element, label: string): void {
  if (el.getAttribute('aria-label') !== label) el.setAttribute('aria-label', label);
}

/** The collapsed line that already had its one scroll; only a new line re-arms it */
let scrolledCompactLine = '';

function fitCompactTitle(): void {
  requestAnimationFrame(() => {
    // The leaving layer is still in the DOM during the cross-fade; measure the current one
    const title = document.querySelector<HTMLElement>('#cc .compact-slot.current .compact-title');
    const track = title?.querySelector<HTMLElement>('.compact-title-track');
    if (!title || !track || state === 'open' || title.classList.contains('dictation-preview')) return;
    // Mid-morph the column is the wrong width; transitionend refits once the shell settles
    if (Math.abs(isl.offsetWidth - parseFloat(isl.style.width)) > 1) return;
    const line = track.textContent ?? '';
    const overflow = Math.max(0, track.offsetWidth - title.clientWidth);
    const overflows = overflow > 2 && !reducedMotion.matches;
    title.classList.toggle('scroll', overflows && state === 'peek');
    if (!overflows) scrolledCompactLine = '';
    if (!overflows || state === 'peek') {
      title.classList.remove('scroll-once');
      title.style.setProperty('--compact-shift', `-${overflow}px`);
      return;
    }
    // Idle stops after one pass, on the ellipsis, so nothing runs for the rest of the track
    if (line === scrolledCompactLine) return;
    scrolledCompactLine = line;
    const lyric = livePreferences.compactContent === 'lyrics' && compactLyric() === line;
    title.style.setProperty('--compact-shift', `-${overflow}px`);
    title.style.setProperty('--compact-scroll', `${compactScrollMs(overflow, lyric)}ms`);
    title.classList.add('scroll-once');
    track.addEventListener('animationend', () => title.classList.remove('scroll-once'), { once: true });
  });
}

function compactTextIndicator(
  text: string,
  ariaLabel: string,
  hasBackgroundActivity: boolean,
): HTMLElement {
  const indicator = document.createElement('span');
  indicator.className = 'compact-indicator';
  indicator.setAttribute('role', 'img');
  indicator.setAttribute(
    'aria-label',
    hasBackgroundActivity ? `${ariaLabel}, another activity is running` : ariaLabel,
  );
  indicator.append(compactText(text, 'compact-metric'));
  if (hasBackgroundActivity) {
    const dot = document.createElement('span');
    dot.className = 'live-activity-dot';
    dot.setAttribute('aria-hidden', 'true');
    indicator.append(dot);
  }
  return indicator;
}

function compactCandidates(): CompactActivityCandidate[] {
  return [
    ...[...externalActivities.entries()].map(([id, activity]) => ({
      id: `external:${id}`,
      kind: activity.type === 'dictation' ? 'dictation' as const : 'external' as const,
      title: activity.title,
      priority: activity.priority,
      active: true,
      running: activity.type !== 'dictation' || ['starting', 'recording', 'transcribing', 'processing', 'cancelling', 'inserting'].includes(activity.phase || ''),
    })),
    {
      id: 'media',
      kind: 'media',
      title: mediaState.title || 'Media',
      // Playing media outranks timers and downloads; paused media yields to everything running
      priority: mediaState.playing || pauseHold ? 85 : 40,
      // A paused track gives the idle pill back to time and battery; a peek can still name it
      active: mediaState.active && (mediaState.playing || pauseHold || state === 'peek'),
      running: mediaState.playing,
    },
    ...activitiesManager.getAll().filter(activity => activity.active).map(activity => ({
      id: `timer:${activity.id}`,
      kind: 'timer' as const,
      title: activity.title,
      priority: activity.paused ? activity.priority - 30 : activity.priority,
      active: true,
      running: !activity.paused,
    })),
  ];
}

/** Morphs the idle pill and moves the native hit area with it, so the whole pill stays clickable */
function syncIdleWidth(width: number): void {
  if (width === idleWidth) return;
  idleWidth = width;
  isl.style.setProperty('--w', `${width}px`);
  isl.style.width = `${width}px`;
  const hit = hitGeometry(width, SIZES.idle[1], topOffset);
  runAction(setHit(hit.w, hit.h), 'Update island bounds');
}

function renderCompact(): void {
  const cl = $('cl');
  const cc = $('cc');
  const cr = $('cr');
  const compact = document.querySelector<HTMLElement>('.compact')!;

  if (state === 'peek' && hud.visible(Date.now())) {
    renderHud(hud.reading!, compact);
    return;
  }

  if (announcement && announcement.until > Date.now() && state === 'peek') {
    setCompactSlot(cl, 'announcement:leading', 'note', () => compactMarkup(SVG_ICONS.note));
    const changed = setCompactSlot(cc, 'announcement:title', announcement.text, () => compactTitle(announcement!.text));
    setCompactSlot(cr, 'announcement:trailing', 'notification', () => compactIndicator(
      '<span class="peek-indicator"></span>',
      'Notification',
      false,
    ));
    setLabel(compact, announcement.text);
    if (changed) fitCompactTitle();
    return;
  }

  const timerActivities = activitiesManager.getAll().filter(activity => activity.active);
  const selection = selectCompactActivity(compactCandidates());
  const primary = selection.primary;

  const mediaText = primary?.kind === 'media' && livePreferences.compactContent !== 'artwork';
  isl.toggleAttribute('data-compact-text', mediaText || primary?.kind === 'dictation');
  const closedWidth = compactShellWidth(primary, livePreferences.compactContent);
  if (state === 'idle') syncIdleWidth(closedWidth);
  if (closedWidth !== pillStrip.width) {
    pillStrip = { ...pillStrip, width: closedWidth };
    isl.style.setProperty('--pill-w', `${closedWidth}px`);
  }

  if (!primary) {
    const time = formatClockTime();
    if (state === 'peek') {
      setCompactSlot(cl, 'idle-peek:leading', 'clock', () => compactMarkup(SVG_ICONS.clock));
      const changed = setCompactSlot(cc, 'idle-peek:title', time, () => compactTitle(time));
      if (changed) fitCompactTitle();
    } else {
      setCompactSlot(cl, 'idle:leading', time, () => compactText(time, 'compact-time'));
      setCompactSlot(cc, 'idle:center', '', () => compactText(''));
    }
    const batteryLabel = batteryAriaLabel();
    // Desktops without a battery show only the time
    if (!powerState.hasBattery && powerState.statusKnown !== false) {
      setCompactSlot(cr, 'idle:trailing-empty', '', () => compactText(''));
      setLabel(compact, `Time ${time}`);
    } else {
      setCompactSlot(cr, 'idle:trailing', `${batteryLabel}:${powerState.percent}`, buildBatteryStatus);
      setLabel(compact, `Time ${time}. ${batteryLabel}`);
    }
    return;
  }

  // Media can show the current lyric line instead of the track title (Settings > Collapsed content)
  const lyric = primary.kind === 'media' && livePreferences.compactContent === 'lyrics' ? compactLyric() : '';
  const title = lyric || primary.title;
  if (primary.kind === 'dictation') {
    // Its center is rendered below from provisional text rather than the activity title.
  } else if (state === 'peek' || mediaText) {
    const changed = setCompactSlot(cc, `${primary.id}:title`, title, () => compactTitle(title));
    if (changed) fitCompactTitle();
  } else {
    setCompactSlot(cc, 'activity:center', '', () => compactText(''));
  }

  if (primary.kind === 'media') {
    const artwork = currentArtwork;
    setCompactSlot(cl, 'media:leading', artwork || 'note', () => {
      if (!artwork) return compactMarkup(SVG_ICONS.note);
      const miniArt = document.createElement('span');
      miniArt.className = 'mini-art';
      miniArt.style.backgroundImage = `url("${artwork}")`;
      miniArt.setAttribute('role', 'img');
      miniArt.setAttribute('aria-label', 'Album artwork');
      return miniArt;
    });
    const mediaLabel = mediaState.playing ? 'Media playing' : 'Media paused';
    setCompactSlot(
      cr,
      'media:trailing',
      `${mediaState.playing}:${selection.hasBackgroundActivity}`,
      () => compactIndicator(
        mediaState.playing ? SVG_ICONS.eqPlaying : SVG_ICONS.eq,
        mediaLabel,
        selection.hasBackgroundActivity,
      ),
    );
    setLabel(compact, `${mediaLabel}: ${title}`);
    return;
  }

  if (primary.kind === 'timer') {
    const activity = timerActivities.find(item => `timer:${item.id}` === primary.id)!;
    const icon = activity.type === 'stopwatch'
      ? SVG_ICONS.stopwatch
      : activity.type === 'pomodoro'
        ? SVG_ICONS.pomodoro
        : SVG_ICONS.timer;
    const remaining = activitiesManager.formatRemaining(activity);
    setCompactSlot(cl, `${primary.id}:leading`, activity.type, () => compactMarkup(icon));
    setCompactSlot(
      cr,
      `${primary.id}:trailing`,
      `${remaining}:${selection.hasBackgroundActivity}`,
      () => compactTextIndicator(
        remaining,
        `${title}, ${remaining}${activity.paused ? ', paused' : ' remaining'}`,
        selection.hasBackgroundActivity,
      ),
    );
    setLabel(compact, `${title}, ${remaining}${activity.paused ? ', paused' : ' remaining'}`);
    return;
  }

  const externalId = primary.id.slice('external:'.length);
  const external = externalActivities.get(externalId)!;
  if (primary.kind === 'dictation') {
    const recording = external.phase === 'recording';
    const phaseLabel = recording ? 'Dictation recording' : external.phase === 'transcribing' ? 'Transcribing' : external.phase === 'processing' ? 'Processing' : external.phase === 'cancelling' ? 'Cancelling' : external.phase === 'starting' ? 'Starting' : external.phase === 'inserting' ? 'Inserting' : 'Review transcript';
    const line = external.text || (recording ? '' : `${phaseLabel}\u2026`);
    if (recording && !external.text) {
      const level = external.audioLevel;
      setCompactSlot(cc, 'dictation:waveform', String(level), () => compactMarkup(
        `<span class="dictation-waveform" aria-label="Listening"><i style="height:${2 + level * 8}px"></i><i style="height:${2 + level * 16}px"></i><i style="height:${2 + level * 12}px"></i><i style="height:${2 + level * 16}px"></i><i style="height:${2 + level * 8}px"></i></span>`));
    } else {
    // Revised previews use a stable width and no marquee.
    setCompactSlot(cc, 'dictation:preview', line, () => {
      const preview = compactTitle(line);
      preview.classList.add('dictation-preview');
      return preview;
    });
    }
    setCompactSlot(cl, 'dictation:leading', 'mic', () => compactMarkup(SVG_ICONS.mic));
    setCompactSlot(
      cr,
      'dictation:trailing',
      `${recording}:${selection.hasBackgroundActivity}`,
      () => compactIndicator(
        `<span class="dictation-level" aria-hidden="true"><i></i><i></i><i></i></span>`,
        recording ? 'Dictation recording' : 'Dictation active',
        selection.hasBackgroundActivity,
      ),
    );
    setLabel(compact, external.text ? `${phaseLabel}: ${external.text}` : phaseLabel);
    return;
  }

  setCompactSlot(cl, `${primary.id}:leading`, external.type, () => compactMarkup(SVG_ICONS.external));
  setCompactSlot(
    cr,
    `${primary.id}:trailing`,
    `${external.text}:${selection.hasBackgroundActivity}`,
    () => compactTextIndicator(
      external.text,
      `${external.title}, ${external.text}`,
      selection.hasBackgroundActivity,
    ),
  );
  setLabel(compact, `${external.title}, ${external.text}`);
}

/* ================= Today & Battery ================= */

let powerState: PowerState = {
  percent: 100,
  charging: false,
  hasBattery: false,
  minutesLeft: null,
};

function renderToday(): void {
  const now = new Date();
  $('clock').textContent = formatClockTime(now);
  $('date').textContent = now.toLocaleDateString([], {
    weekday: 'long',
    day: 'numeric',
    month: 'long',
    year: 'numeric',
  });
}

const battRing = createRing(72, 6, 'batt-ring');
$('battFace').prepend(battRing.root);
$('battBolt').innerHTML = SVG_ICONS.bolt;

function renderPower(): void {
  const pctEl = $('pct');
  const sub = $('battSub');
  const bolt = $('battBolt');
  const batt = $('batt');
  let label: string;
  let subText = '';
  bolt.hidden = true;
  if (!powerState.hasBattery && powerState.statusKnown !== false) {
    // A desktop on mains power: a plug in an empty track
    battRing.set(null);
    setMarkup(pctEl, SVG_ICONS.plug);
    label = 'No system battery, on AC power';
  } else if (powerState.statusKnown === false || powerState.percentKnown === false || !Number.isFinite(powerState.percent)) {
    battRing.set(null);
    setText(pctEl, '–');
    label = 'Battery level unavailable';
  } else {
    const pct = Math.max(0, Math.min(100, Math.round(powerState.percent)));
    battRing.set(pct, batteryTone(pct, powerState.charging));
    setText(pctEl, `${pct}%`);
    bolt.hidden = !powerState.charging;
    label = `Battery ${pct}%`;
    if (powerState.charging) label += ', charging';
    else if (powerState.minutesLeft != null && powerState.minutesLeft > 0) {
      subText = `${formatSpan(powerState.minutesLeft * 60)} left`;
      label += `, ${subText}`;
    } else if (powerState.acConnected === true) label += ', plugged in';
  }
  // Status text only when it adds information: the time left
  if (sub.textContent !== subText) sub.textContent = subText;
  batt.setAttribute('aria-label', label);
  batt.title = label;

  renderCompact();
}



/* ================= Live Activities ================= */

const actRing = createRing(128, 6, 'act-ring');
$('actFace').prepend(actRing.root);
/** The preset the play button starts when nothing is running */
let actPreset = 5;

function startPreset(minutes: number): void {
  actPreset = minutes;
  if (minutes === 25) activitiesManager.startPomodoro(25);
  else activitiesManager.startTimer(minutes * 60, `${minutes}m timer`);
}

function setupActivitiesUI(): void {
  $('actStopwatch').innerHTML = SVG_ICONS.stopwatch;
  $('actReset').innerHTML = SVG_ICONS.refresh;

  activitiesManager.subscribe((_list, top) => {
    // A timer started anywhere (chips, compact, native) becomes what play restarts
    const started = presetOf(top);
    if (started === 5 || started === 15 || started === 25) actPreset = started;
    renderCompact();
    renderActivitiesPage(top);
  });

  document.querySelectorAll<HTMLButtonElement>('.act-chip').forEach((chip) => {
    chip.onclick = (e) => {
      e.stopPropagation();
      startPreset(Number(chip.dataset.minutes) || 5);
    };
  });

  $('actStopwatch').onclick = (e) => {
    e.stopPropagation();
    activitiesManager.startStopwatch();
  };

  $('actPause').onclick = (e) => {
    e.stopPropagation();
    const top = activitiesManager.getTopActivity();
    if (!top) { startPreset(actPreset); return; }
    if (top.type === 'stopwatch') {
      if (top.paused) activitiesManager.startStopwatch();
      else activitiesManager.pauseStopwatch();
    } else {
      if (top.paused) activitiesManager.resumeTimer(top.id);
      else activitiesManager.pauseTimer(top.id);
    }
  };

  $('actReset').onclick = (e) => {
    e.stopPropagation();
    const top = activitiesManager.getTopActivity();
    if (!top) return;
    if (top.type === 'stopwatch') activitiesManager.resetStopwatch();
    else if (top.type === 'pomodoro') activitiesManager.resetPomodoro();
    else activitiesManager.resetTimer(top.id);
  };
}

function presetOf(top: Activity | null): number | null {
  if (!top || top.type === 'stopwatch') return null;
  if (top.type === 'pomodoro') return 25;
  return Math.round((top.totalDurationMs || 0) / 60000);
}

function setControl(btn: HTMLElement, label: string, icon: string): void {
  if (btn.getAttribute('aria-label') === label) return;
  btn.setAttribute('aria-label', label);
  btn.title = label;
  btn.innerHTML = icon;
}

function renderActivitiesPage(top: Activity | null): void {
  const status = $('actStatus');
  const pauseBtn = $('actPause');
  const face = $('actFace');
  const running = !!top && !top.paused;
  const phase = !top ? 'ready' : top.paused ? 'paused' : 'running';
  const phaseLabel = { ready: 'Ready', paused: 'Paused', running: 'Running' }[phase];

  setText($('actDigits'), top ? activitiesManager.formatRemaining(top) : `${String(actPreset).padStart(2, '0')}:00`);
  setText($('actTitle'), top ? top.title : 'Timer');
  if (status.dataset.status !== phase) {
    status.dataset.status = phase;
    status.setAttribute('aria-label', phaseLabel);
    status.title = phaseLabel;
  }
  face.dataset.status = phase;
  face.dataset.kind = top?.type ?? 'timer';
  actRing.set(top ? (activityProgress(top) ?? 0) * 100 : null, running ? 'ink' : 'muted');

  if (!top) setControl(pauseBtn, actPreset === 25 ? 'Start 25m pomodoro' : `Start ${actPreset}m timer`, SVG_ICONS.play);
  else if (running) setControl(pauseBtn, 'Pause', SVG_ICONS.pause);
  else setControl(pauseBtn, 'Resume', SVG_ICONS.play);
  ($('actReset') as HTMLButtonElement).disabled = !top;

  const preset = presetOf(top);
  document.querySelectorAll<HTMLButtonElement>('.act-chip').forEach((chip) => {
    const on = String(Number(chip.dataset.minutes) === preset);
    if (chip.getAttribute('aria-pressed') !== on) chip.setAttribute('aria-pressed', on);
  });
  const sw = $('actStopwatch');
  const swOn = String(top?.type === 'stopwatch');
  if (sw.getAttribute('aria-pressed') !== swOn) sw.setAttribute('aria-pressed', swOn);
}

/* ================= AI Usage ================= */

let rawMeters: Meter[] = [];
let prevPcts: Record<string, number> = {};

function announceReset(meter: Meter): void { showAnnouncement(`${meter.label} reset`); }

function renderUsage(): void {
  const meters = connectedUsageMeters(mergeMeters(rawMeters));
  const providers = renderUsageRings($('uList'), meters, () => {
    logChatGPTMessage();
    renderUsage();
  });
  rail.setSectionVisible('usage', providers > 0);
}

async function refreshUsageSnapshot(): Promise<void> {
  const ticket = usageSnapshots.begin();
  const meters = await fetchUsageNow();
  if (usageSnapshots.accepts(ticket)) onUsageUpdate(meters);
}

function onUsageUpdate(meters: Meter[]): void {
  usageSnapshots.changed();
  rawMeters = meters || [];
  const merged = connectedUsageMeters(mergeMeters(rawMeters));
  prevPcts = checkResets(merged, prevPcts, announceReset);
  renderUsage();
}

window.addEventListener('storage', () => { renderUsage(); });


/* ================= Quick Actions ================= */

function setupQuickActions(): void {
  $('icoWifi').innerHTML = SVG_ICONS.wifi;
  $('icoBt').innerHTML = SVG_ICONS.bluetooth;
  $('icoFocus').innerHTML = SVG_ICONS.focus;
  $('icoCapture').innerHTML = SVG_ICONS.capture;
  $('prev').innerHTML = SVG_ICONS.prev;
  $('next').innerHTML = SVG_ICONS.next;

  renderMediaHeader();

  document.querySelectorAll<HTMLButtonElement>('.chip').forEach((chip) => {
    chip.onclick = (e) => {
      e.stopPropagation();
      const action = chip.dataset.action;
      if (action) {
        runAction(triggerQuickAction(action), 'Open Windows action');
      }
    };
  });

}

/** A peek ended: next queued announcement first, then back to where the island belongs */
function finishPeek(): void {
  const next = pendingAnnouncements.next(Date.now());
  if (next) { showAnnouncement(next, false); return; }
  if (state === 'open') return;
  setState(peekReturnTarget({ returnOpen: returnOpenAfterPeek, hover: hoverInside, pinned: livePreferences.pinned, dragging: nativeDragActive }));
}

function showAnnouncement(text:string, speak=true):void {
  // Screen readers hear it right away, even when the text has to wait for the HUD
  if(speak)$('statusMessage').textContent=text;
  const wasOpen = state === 'open';
  if(wasOpen && (rail.active() !== 'media' || keyboardMode))return;
  if(wasOpen)returnOpenAfterPeek=true;
  // The HUD owns the shell until it times out; the text follows it
  if(hud.visible(Date.now())){pendingAnnouncements.add(text,Date.now());return;}
  announcement={text:text.slice(0,180),until:Date.now()+3500};
  window.clearTimeout(closeTimer);setState('peek');
  closeTimer=window.setTimeout(()=>{announcement=null;finishPeek();},3500);
}

/* ================= Volume & Brightness HUD ================= */

function buildHudBar(): HTMLElement {
  const bar = document.createElement('span');
  bar.className = 'hud-bar';
  bar.setAttribute('aria-hidden', 'true');
  const fill = document.createElement('span');
  fill.className = 'hud-fill';
  bar.append(fill);
  return bar;
}

function hudGlyph(reading: HudReading): 'brightness' | ReturnType<typeof volumeGlyph> {
  return reading.kind === 'brightness' ? 'brightness' : volumeGlyph(reading.level, reading.muted);
}

/** Glyph, level bar and tabular value; the bar is built once per kind so its fill eases between values */
function renderHud(reading: HudReading, compact: HTMLElement): void {
  const glyph = hudGlyph(reading);
  setCompactSlot($('cl'), 'hud:leading', glyph, () => compactMarkup(SVG_ICONS[glyph], 'hud-glyph'));
  setCompactSlot($('cc'), `hud:bar:${reading.kind}`, 'bar', buildHudBar);
  const bar = $('cc').querySelector<HTMLElement>('.compact-slot.current .hud-bar');
  if (bar) {
    bar.classList.toggle('muted', reading.muted);
    bar.style.setProperty('--level', String(reading.level / 100));
  }
  const value = `${reading.level}%`;
  setCompactSlot($('cr'), 'hud:value', `${value}:${reading.muted}`, () =>
    compactText(value, reading.muted ? 'compact-metric hud-value muted' : 'compact-metric hud-value'));
  setLabel(compact, hudLabel(reading));
}

function settleHud(): void {
  hud.dismiss();
  // An island opened meanwhile owns the announcement timer for its in-strip level
  if (state !== 'open') window.clearTimeout(hudAnnounceTimer);
  finishPeek();
}

/** Screen readers hear the value once input pauses; a newer change replaces the wait */
function announceWhenSettled(model: HudModel): void {
  window.clearTimeout(hudAnnounceTimer);
  hudAnnounceTimer = window.setTimeout(() => {
    const text = model.dueAnnouncement(Date.now());
    if (text) $('statusMessage').textContent = text;
  }, HUD_ANNOUNCE_QUIET_MS + 20);
}

/** The open island keeps its size and content; the level paints over the strip for a moment */
function showOpenLevel(reading: HudReading, now: number): void {
  openLevel.show(reading, now);
  announceWhenSettled(openLevel);
  if (!levelStrip.firstChild) {
    levelStrip.append(compactMarkup('', 'hud-glyph'), buildHudBar(), compactText('', 'compact-metric hud-value'));
  }
  const [glyph, bar, value] = levelStrip.children as HTMLCollectionOf<HTMLElement>;
  setMarkup(glyph, SVG_ICONS[hudGlyph(reading)]);
  bar.classList.toggle('muted', reading.muted);
  bar.style.setProperty('--level', String(reading.level / 100));
  value.textContent = `${reading.level}%`;
  value.classList.toggle('muted', reading.muted);
  levelStrip.dataset.on = reading.kind;
  window.clearTimeout(openLevelTimer);
  openLevelTimer = window.setTimeout(() => {
    openLevel.dismiss();
    delete levelStrip.dataset.on;
  }, openLevel.remaining(now));
}

function showHud(reading: HudReading): void {
  const now = Date.now();
  if (state === 'open') {
    showOpenLevel(reading, now);
    return;
  }
  // An interrupted announcement gets its full time back once the HUD settles
  if (announcement && announcement.until > now) pendingAnnouncements.addFirst(announcement.text, now);
  announcement = null;
  hud.show(reading, now);
  window.clearTimeout(openTimer);
  window.clearTimeout(closeTimer);
  window.clearTimeout(hudTimer);
  hudTimer = window.setTimeout(settleHud, hud.remaining(now));
  announceWhenSettled(hud);
  if (state !== 'peek') setState('peek');
  else {
    isl.dataset.hud = reading.kind;
    renderCompact();
  }
}

// Browser demo only: a local level so the HUD can be tried without the desktop app
let demoVolume = { level: 50, muted: false };
function applyVolume(command: 'volume_step' | 'volume_toggle_mute', args?: Record<string, unknown>): void {
  if (!isTauri()) {
    demoVolume = command === 'volume_step'
      ? { level: stepVolume(demoVolume.level, Number(args?.delta) || 0), muted: Number(args?.delta) > 0 ? false : demoVolume.muted }
      : { ...demoVolume, muted: !demoVolume.muted };
    showHud({ kind: 'volume', ...demoVolume });
    return;
  }
  void invokeCommand<unknown>(command, args).then(result => {
    const reading = readHudPayload(result);
    if (reading) showHud(reading);
  }).catch(() => { /* no output device; the HUD simply does not appear */ });
}

function overLevelInput(event: MouseEvent): boolean {
  return levelInputApplies(state, isl.getBoundingClientRect(), pillStrip, event.clientX, event.clientY);
}

// The pill-sized strip always means volume, in every state. These listeners capture, so
// whatever sits under the strip on the open island (a scroller, the rail's tab switching)
// never sees the event. Only wheel and middle click are taken: left clicks go through.
let wheelRest = 0;
isl.addEventListener('wheel', event => {
  if (!overLevelInput(event)) return;
  event.preventDefault();
  event.stopPropagation();
  const result = wheelVolumeDelta(wheelRest, event);
  wheelRest = result.rest;
  if (result.delta !== 0) applyVolume('volume_step', { delta: result.delta });
}, { passive: false, capture: true });

// Middle click toggles mute over the strip; stopping the press stops the browser's autoscroll everywhere
isl.addEventListener('mousedown', event => { if (event.button === 1) event.preventDefault(); }, { capture: true });
isl.addEventListener('auxclick', event => {
  if (event.button !== 1 || !overLevelInput(event)) return;
  event.preventDefault();
  event.stopPropagation();
  applyVolume('volume_toggle_mute');
}, { capture: true });

window.addEventListener('tuck-activity',event=>{
  const detail=(event as CustomEvent).detail;
  if(!detail||typeof detail.id!=='string'||typeof detail.title!=='string'||typeof detail.text!=='string'||!Number.isFinite(detail.priority))return;
  if(detail.active)externalActivities.set(detail.id,{
    title:detail.title.slice(0,80),
    text:detail.text.slice(0,80),
    priority:Math.max(0,Math.min(100,detail.priority)),
    type:typeof detail.type==='string'?detail.type:'external',
    audioLevel: Number.isFinite(detail.audioLevel) ? Math.max(0, Math.min(1, detail.audioLevel)) : 0,
    phase:typeof detail.phase==='string'?detail.phase:'active',
  });else externalActivities.delete(detail.id);
  renderCompact();
});
window.addEventListener('tuck-preview',event=>{
  const detail=(event as CustomEvent).detail;
  if(detail&&typeof detail.title==='string'&&typeof detail.text==='string')showAnnouncement(`${detail.title}: ${detail.text}`);
});
function holdDrag(active:boolean):void {
  nativeDragActive=active;window.clearTimeout(closeTimer);
  if(active){setState('open');rail.reveal('extensions');}
  else onHover(hoverInside);
}
window.addEventListener('tuck-drop-active',event=>holdDrag((event as CustomEvent).detail===true));
window.addEventListener('tuck-drag-active',event=>holdDrag((event as CustomEvent).detail===true));
window.addEventListener('blur',()=>{keyboardMode=false;if(!nativeDragActive)onHover(hoverInside);});

/* ================= Subscriptions & Native Events ================= */

void listenToEvent<{reason?: string}>('dictation_transcript', data => { if (data.reason === 'Copied') showAnnouncement('Copied'); });
void listenToEvent<null>('open-island', () => { keyboardMode=true;window.clearTimeout(closeTimer);setState('open');isl.focus(); });
void listenToEvent<boolean>('hover', (inside) => onHover(inside));
void listenToEvent<boolean>('fullscreen', (hidden) => {
  document.body.classList.toggle('fullscreen-hidden', hidden);
});
void listenToEvent<MediaState>('media', (payload) => onMediaUpdate(payload));
void listenToEvent<PowerState>('power', (payload) => {
  const previous = powerState;
  powerState = payload;
  renderPower();
  if (previous.statusKnown && payload.statusKnown && previous.acConnected !== payload.acConnected) lastChargerChange = Date.now();
  if (livePreferences.desktopPeeks && previous.statusKnown && payload.statusKnown && previous.acConnected !== payload.acConnected) showAnnouncement(payload.acConnected ? 'Charger connected' : 'Charger disconnected');
});
void listenToEvent<{title: string; text: string}>('desktop-peek', payload => {
  if (livePreferences.desktopPeeks && typeof payload?.title === 'string' && typeof payload.text === 'string') showAnnouncement(`${payload.title}: ${payload.text}`);
});
void listenToEvent<unknown>('system-level', payload => {
  const reading = readHudPayload(payload);
  if (reading && levelEventShowsHud(reading, livePreferences.levelHud, Date.now(), lastChargerChange, flyoutHidden)) showHud(reading);
});
interface FlyoutStatus { state?: string; message?: string | null }
function onFlyoutStatus(status: FlyoutStatus | null | undefined): void {
  flyoutHidden = status?.state === 'hidden';
  if (status?.state === 'unavailable' && typeof status.message === 'string') showAnnouncement(status.message);
}
void listenToEvent<FlyoutStatus>('system-flyout', onFlyoutStatus);
void listenToEvent<Meter[]>('usage', (payload) => onUsageUpdate(payload));

/* ================= First Run & Browser Preview ================= */

if (!storedShape && isTauri()) {
  runAction(openSettingsCmd(), 'Open settings');
}

if (!isTauri()) {
  document.body.classList.add('dev-mode');

  // Add demo badge
  const demoBadge = document.createElement('div');
  demoBadge.className = 'demo-banner';
  demoBadge.textContent = 'Browser Demo \u00b7 Open Settings';
  demoBadge.onclick = () => runAction(openSettingsCmd(), 'Open settings');
  document.body.appendChild(demoBadge);

  // Dispatch initial realistic mock data for preview/development
  setTimeout(() => {
    dispatchMockEvent<MediaState>('media', {
      active: true,
      trackId: 'demo-track',
      title: 'How I Use Grok Bot Every Single Day',
      artist: 'Ras Mic',
      playing: true,
      positionMs: 401000,
      durationMs: 1128000,
      artwork: null,
    });

    dispatchMockEvent<PowerState>('power', {
      percent: 82,
      charging: true,
      hasBattery: true,
      minutesLeft: null,
    });

    dispatchMockEvent<Meter[]>('usage', [
      {
        id: 'claude-5h',
        provider: 'claude',
        label: 'Claude session (5 h)',
        usedPct: 62,
        spentUsd: null,
        resetsAt: Math.round(Date.now() / 1000 + 8100),
        source: 'official',
        stale: false,
      },
      {
        id: 'claude-7d',
        provider: 'claude',
        label: 'Claude weekly (7 d)',
        usedPct: 41,
        spentUsd: null,
        resetsAt: Math.round(Date.now() / 1000 + 86400 * 3),
        source: 'official',
        stale: false,
      },
      {
        id: 'codex-5h',
        provider: 'openai',
        label: 'Codex 5-hour window',
        usedPct: 15,
        spentUsd: null,
        resetsAt: Math.round(Date.now() / 1000 + 3600 * 4),
        source: 'official',
        stale: false,
      },
    ]);
  }, 100);
}

if (isTauri()) {
  runAction(invokeCommand('set_fullscreen_auto_hide', { enabled: livePreferences.fullscreenAutoHide }), 'Apply fullscreen preference');
  runAction(invokeCommand<MediaState>('media_now').then(onMediaUpdate), 'Read media session');
  runAction(invokeCommand<boolean>('fullscreen_now').then(hidden => document.body.classList.toggle('fullscreen-hidden', hidden)), 'Read fullscreen state');
  // A failure before this listener existed is still pending natively
  runAction(invokeCommand<FlyoutStatus>('get_hide_system_flyout').then(onFlyoutStatus), 'Read flyout state');
}

// Fetch initial usage if running in native app
void refreshUsageSnapshot();

/* ================= Initialization ================= */

setupActivitiesUI();
setupQuickActions();
renderToday();
renderPower();
renderUsage();
renderMedia();
renderCompact();
setState('idle');
