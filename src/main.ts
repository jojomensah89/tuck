import {mountExtensions} from './extensions.js';
import { mountLivePages } from './live-ui.js';
import { LIVE_SETTINGS_KEY, readPreferences } from './live-state.js';
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
import { SVG_ICONS } from './icons.js';
import { activitiesManager } from './activities.js';
import {
  STORAGE_KEYS,
  mergeMeters,
  checkResets,
  logChatGPTMessage,
  formatUntil,
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

let livePreferences = readPreferences();

function $(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`Element #${id} not found`);
  return el;
}

const isl = $('isl');

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

function setNode(el: HTMLElement, key: string, build: () => Node): void {
  const marker = `node:${key}`;
  if (slotMarkup.get(el) === marker) return;
  slotMarkup.set(el, marker);
  el.replaceChildren(build());
}

function runAction(action: Promise<unknown>, label: string): void {
  void action.catch(() => {
    const message = `${label} failed. Please try again.`;
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
let pageIndex = 0;
let closeTimer = 0;
let openTimer = 0;
let wheelLock = 0;
let hoverInside = false;
let nativeDragActive = false;
let keyboardMode = false;
let announcement: { text: string; until: number } | null = null;
const externalActivities = new Map<string, { title: string; text: string; priority: number }>();

function setState(next: IslandState): void {
  if (state === 'open' && next !== 'open' && isTauri()) runAction(invokeCommand('deactivate_island'), 'Restore desktop interaction');
  if (next !== 'open') keyboardMode = false;
  state = next;
  const [w, h] = next === 'open' ? [livePreferences.openWidth, 210] : SIZES[next];
  isl.style.setProperty('--w', `${w}px`);
  isl.style.setProperty('--h', `${h}px`);
  isl.style.setProperty('--open-w', `${livePreferences.openWidth}px`);
  isl.style.width = `${w}px`;
  isl.style.height = `${h}px`;
  isl.dataset.state = next;

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

function onHover(inside: boolean): void {
  hoverInside = inside;
  window.clearTimeout(closeTimer);
  window.clearTimeout(openTimer);
  if (nativeDragActive || (!inside && (keyboardMode || livePreferences.pinned) && state === 'open')) return;

  if (inside) {
    if (livePreferences.hoverExpand) openTimer = window.setTimeout(() => setState('open'), livePreferences.openDelayMs);
    else if (state === 'idle') setState('peek');
  } else {
    closeTimer = window.setTimeout(
      () => setState('idle'),
      state === 'open' ? livePreferences.closeDelayMs : 120
    );
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
const pages = Array.from(document.querySelectorAll<HTMLElement>('.page'));

window.addEventListener('storage', event => {
  if (event.key !== LIVE_SETTINGS_KEY) return;
  livePreferences = readPreferences();
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
  } else if (state === 'open') {
    if (e.key === 'ArrowRight') {
      e.preventDefault();
      showPage(pageIndex + 1, 1);
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      showPage(pageIndex - 1, -1);
    }
  }
});

isl.addEventListener(
  'wheel',
  (e) => {
    if ((e.target as HTMLElement).closest('.usage, .live-page, .lyrics-pane, input, select') || state !== 'open' || Date.now() < wheelLock) return;
    wheelLock = Date.now() + 350;
    const delta = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
    if (delta !== 0) {
      showPage(pageIndex + (delta > 0 ? 1 : -1), delta > 0 ? 1 : -1);
    }
  },
  { passive: true }
);

/* ================= Pages Navigation ================= */

const dotButtons: HTMLButtonElement[] = [];

// Dots are built once and only their state changes, so the width transition plays.
function buildDots(): void {
  const dotsContainer = $('dots');
  dotsContainer.replaceChildren();
  dotButtons.length = 0;
  pages.forEach((page, i) => {
    const btn = document.createElement('button');
    btn.setAttribute('aria-label', `Go to ${page.dataset.page} page`);
    btn.title = page.getAttribute('aria-label') || page.dataset.page || 'Page';
    btn.onclick = (e) => {
      e.stopPropagation();
      showPage(i, i > pageIndex ? 1 : -1);
    };
    dotButtons.push(btn);
    dotsContainer.appendChild(btn);
  });
}

function clearLeaving(page: HTMLElement): void {
  page.classList.remove('leaving');
  if (!page.classList.contains('on')) page.classList.remove('fwd', 'back');
}

function showPage(n: number, dir = 1): void {
  if (pages.length === 0) return;
  if (dotButtons.length !== pages.length) buildDots();
  const focused = document.activeElement;
  const relocateFocus = state === 'open' && focused instanceof HTMLElement &&
    (pages[pageIndex]?.contains(focused) || $('dots').contains(focused));
  const previous = pageIndex;
  pageIndex = (n + pages.length) % pages.length;
  const changed = previous !== pageIndex;

  pages.forEach((p, i) => {
    const active = i === pageIndex;
    p.inert = !active;
    p.setAttribute('aria-hidden', String(!active));
    if (active) {
      clearLeaving(p);
      if (changed || !p.classList.contains('on')) {
        p.classList.remove('on', 'fwd', 'back');
        void p.offsetWidth; // restart the enter animation, even when re-entering mid-exit
        p.classList.add('on', dir > 0 ? 'fwd' : 'back');
      }
    } else if (i === previous && changed && p.classList.contains('on')) {
      // The outgoing page stays painted while it fades out in the travel direction
      p.classList.remove('on', 'fwd', 'back');
      p.classList.add('leaving', dir > 0 ? 'fwd' : 'back');
      p.addEventListener('animationend', () => clearLeaving(p), { once: true });
    } else {
      p.classList.remove('on');
      clearLeaving(p);
    }
  });

  dotButtons.forEach((btn, i) => {
    btn.classList.toggle('on', i === pageIndex);
    btn.setAttribute('aria-current', String(i === pageIndex));
  });

  if (pageIndex === 0) {
    fitTitle();
    renderMediaHeader();
  }
  if (relocateFocus) {
    const page = pages[pageIndex];
    const control = page.querySelector<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex="0"]');
    if (control) control.focus({ preventScroll: true });
    else { page.tabIndex = -1; page.focus({ preventScroll: true }); }
  }
}

$('nextPage').onclick = (e) => {
  e.stopPropagation();
  showPage(pageIndex + 1, 1);
};
$('prevPage').onclick = (e) => {
  e.stopPropagation();
  showPage(pageIndex - 1, -1);
};

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
let currentArtwork: string | null = null;

function formatMs(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const m = Math.floor(s / 60);
  const sec = s % 60;
  return `${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}`;
}

function formatMediaDate(d = new Date()): string {
  const weekday = d.toLocaleDateString('en-GB', { weekday: 'short' });
  const day = d.getDate();
  const month = d.toLocaleDateString('en-GB', { month: 'short' });
  const year = d.getFullYear();
  return `${weekday} ${day} ${month} ${year}`;
}

function formatMediaTime(d = new Date()): string {
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });
}

function renderMediaHeader(): void {
  const dateEl = document.getElementById('mediaDate');
  const timeEl = document.getElementById('mediaTime');
  const badgeEl = document.getElementById('mediaBattBadge');
  const wavesEl = document.getElementById('mediaWaves');
  const now = new Date();
  if (dateEl) setText(dateEl, formatMediaDate(now));
  if (timeEl) setText(timeEl, formatMediaTime(now));
  if (badgeEl && !badgeEl.innerHTML) badgeEl.innerHTML = SVG_ICONS.bolt;
  if (wavesEl && !wavesEl.innerHTML) wavesEl.innerHTML = SVG_ICONS.eq;
}

function formatIdleTime(now = new Date()): string {
  return now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });
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

function onMediaUpdate(payload: MediaState): void {
  const { artwork, ...rest } = payload;
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
  renderCompact();
}

function currentPosition(): number {
  if (!mediaState.active) return 0;
  return currentPositionMs(mediaState, mediaReceivedAt, performance.now());
}

function tick(): void {
  live.position(currentPosition());
  if (state !== 'open') renderCompact();
  if (state !== 'open' || pageIndex !== 0) return;
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

setInterval(tick, 200);

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

function renderCompact(): void {
  const cl = $('cl');
  const cc = $('cc');
  const cr = $('cr');

  if (announcement && announcement.until > Date.now() && state === 'peek') {
    setMarkup(cl, SVG_ICONS.note);
    setText(cc, announcement.text);
    setText(cr, '');
    return;
  }
  const external = [...externalActivities.values()].sort((a, b) => b.priority - a.priority)[0];
  const actCompact = activitiesManager.getCompactView();

  if (external && external.priority > (actCompact?.priority ?? 0)) {
    setMarkup(cl, SVG_ICONS.timer);
    setText(cc, state === 'peek' ? external.title : '');
    setText(cr, external.text);
    return;
  }
  // Priority stack: Active timer/stopwatch/pomodoro overrides media playback
  if (actCompact && (!mediaState.active || actCompact.priority > 50)) {
    setMarkup(cl, actCompact.leftHtml);
    setText(cc, state === 'peek' ? actCompact.centerText : '');
    setMarkup(cr, actCompact.rightHtml);
    return;
  }

  const showLyrics = livePreferences.compactContent === 'lyrics' && livePreferences.lyricsOpen && mediaState.active;

  if (state === 'idle' && !showLyrics) {
    setMarkup(cl, `<span class="battery-badge">${SVG_ICONS.bolt}</span>`);
    setText(cc, formatIdleTime());
    setMarkup(cr, SVG_ICONS.eq);
    return;
  }

  if (mediaState.active) {
    const artwork = currentArtwork;
    if (artwork) {
      setNode(cl, `art:${artwork}`, () => {
        const miniArt = document.createElement('span');
        miniArt.className = 'mini-art';
        miniArt.style.backgroundImage = `url("${artwork}")`;
        return miniArt;
      });
    } else {
      setMarkup(cl, SVG_ICONS.note);
    }
    setText(cc, livePreferences.compactContent === 'lyrics' ? live.compactLyric() || mediaState.title : mediaState.title);
    setMarkup(cr, mediaState.playing ? SVG_ICONS.eq : SVG_ICONS.pause);
  } else {
    setMarkup(cl, `<span class="battery-badge">${SVG_ICONS.bolt}</span>`);
    setText(cc, new Date().toLocaleDateString([], {
      weekday: 'short',
      day: 'numeric',
      month: 'short',
    }));
    setText(cr, powerState.hasBattery && powerState.percentKnown !== false ? `${powerState.percent}%` : '');
  }
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
  $('clock').textContent = now.toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  });
  $('date').textContent = now.toLocaleDateString([], {
    weekday: 'long',
    day: 'numeric',
    month: 'long',
    year: 'numeric',
  });
}

function renderPower(): void {
  $('pct').textContent = powerState.hasBattery ? `${powerState.percent}%` : 'AC power';
  const fill = $('battFill');
  fill.style.width = `${powerState.hasBattery ? powerState.percent : 100}%`;
  fill.className = powerState.charging
    ? 'charging'
    : powerState.hasBattery && powerState.percent <= 15
      ? 'low'
      : '';

  const sub = $('battSub');
  if (!powerState.hasBattery && powerState.statusKnown !== false) {
    $('pct').textContent = 'AC power';
    fill.style.width = '100%';
    fill.className = '';
    sub.textContent = 'No system battery';
  } else if (powerState.statusKnown === false || powerState.percentKnown === false) {
    $('pct').textContent = 'Battery unavailable';
    fill.style.width = '0%';
    fill.className = '';
    sub.textContent = 'Windows has not reported battery status';
  } else if (powerState.charging) {
    sub.textContent = 'Charging';
  } else if (powerState.minutesLeft != null) {
    const h = Math.floor(powerState.minutesLeft / 60);
    const m = powerState.minutesLeft % 60;
    sub.textContent = `${h}h ${m}m left`;
  } else {
    sub.textContent = powerState.acConnected === true ? 'Plugged in' : powerState.hasBattery ? 'On battery' : 'AC power';
  }

  renderCompact();
}

setInterval(() => {
  renderToday();
  renderMediaHeader();
  if (state !== 'open') renderCompact();
}, 1000);

/* ================= Live Activities ================= */

function setupActivitiesUI(): void {
  activitiesManager.subscribe((_list, top) => {
    renderCompact();
    renderActivitiesPage(top);
  });

  $('actQuick5').onclick = (e) => {
    e.stopPropagation();
    activitiesManager.startTimer(5 * 60, '5m Timer');
  };

  $('actQuick15').onclick = (e) => {
    e.stopPropagation();
    activitiesManager.startTimer(15 * 60, '15m Timer');
  };

  $('actQuick25').onclick = (e) => {
    e.stopPropagation();
    activitiesManager.startPomodoro(25);
  };

  $('actStopwatch').onclick = (e) => {
    e.stopPropagation();
    activitiesManager.startStopwatch();
  };

  $('actPause').onclick = (e) => {
    e.stopPropagation();
    const top = activitiesManager.getTopActivity();
    if (!top) return;
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

function renderActivitiesPage(top: Activity | null): void {
  const digitsEl = $('actDigits');
  const titleEl = $('actTitle');
  const statusEl = $('actStatus');
  const pauseBtn = $('actPause');

  if (!top) {
    digitsEl.textContent = '05:00';
    titleEl.textContent = 'Timer & Pomodoro';
    statusEl.textContent = 'Ready';
    statusEl.className = 'act-status-pill';
    pauseBtn.textContent = 'Pause';
    return;
  }

  digitsEl.textContent = activitiesManager.formatRemaining(top);
  titleEl.textContent = top.title;
  statusEl.textContent = top.paused ? 'Paused' : 'Active';
  statusEl.className = `act-status-pill ${top.paused ? '' : 'active'}`;
  pauseBtn.textContent = top.paused ? 'Resume' : 'Pause';
}

/* ================= AI Usage ================= */

let rawMeters: Meter[] = [];
let prevPcts: Record<string, number> = {};

function announceReset(meter: Meter): void { showAnnouncement(`${meter.label} reset`); }

function renderUsage(): void {
  const list = $('uList');
  list.replaceChildren();

  const allMeters = mergeMeters(rawMeters);

  if (allMeters.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'u-empty';
    empty.textContent =
      'No AI usage sources configured yet. Open Settings from the quick actions or tray to connect API keys or track ChatGPT.';
    list.appendChild(empty);
    return;
  }

  allMeters.forEach((m) => {
    const row = document.createElement('div');
    row.className = 'u-row';

    const name = document.createElement('span');
    name.className = 'u-name';
    name.textContent = m.label;
    row.appendChild(name);

    const val = document.createElement('span');
    val.className = 'u-val';
    if (m.usedPct != null) {
      val.textContent = `${Math.round(m.usedPct)}%`;
    } else if (m.spentUsd != null) {
      val.textContent = `$${m.spentUsd.toFixed(2)}`;
    } else {
      val.textContent = '\u2014';
    }
    row.appendChild(val);

    if (m.usedPct != null) {
      const bar = document.createElement('div');
      bar.className = 'u-bar';
      const fill = document.createElement('i');
      if (m.usedPct >= 90) fill.className = 'hot';
      else if (m.usedPct >= 70) fill.className = 'warn';
      fill.style.width = `${Math.min(100, Math.max(0, m.usedPct))}%`;
      bar.appendChild(fill);
      row.appendChild(bar);
    }

    const sub = document.createElement('div');
    sub.className = 'u-sub';

    const parts: string[] = [];
    if (m.spentUsd != null && m.usedPct != null) parts.push(`$${m.spentUsd.toFixed(2)} spent`);
    if (m.error) parts.push(m.error);
    if (m.status && m.status !== 'ok') parts.push(m.status);
    if (m.id === 'chatgpt' && m.count != null && m.cap != null) {
      parts.push(`${m.count}/${m.cap} messages`);
    }

    const when = formatUntil(m.resetsAt);
    if (when) {
      parts.push(m.id === 'chatgpt' ? `slot frees ${when.replace('resets ', '')}` : when);
    }

    parts.push(m.source === 'manual' ? 'estimate' : m.source === 'api' ? 'API' : 'official');
    if (m.stale) {
      parts.push(m.provider === 'claude' ? 'stale: open Claude Code to refresh' : 'stale: waiting for provider refresh');
    }

    const partsSpan = document.createElement('span');
    partsSpan.textContent = parts.join(' \u00b7 ');
    sub.appendChild(partsSpan);

    if (m.id === 'chatgpt') {
      const logBtn = document.createElement('button');
      logBtn.textContent = 'Log message';
      logBtn.onclick = (e) => {
        e.stopPropagation();
        logChatGPTMessage();
        renderUsage();
      };
      sub.appendChild(logBtn);
    }

    row.appendChild(sub);
    list.appendChild(row);
  });
}

function onUsageUpdate(meters: Meter[]): void {
  rawMeters = meters || [];
  const merged = mergeMeters(rawMeters);
  prevPcts = checkResets(merged, prevPcts, announceReset);
  renderUsage();
}

window.addEventListener('storage', () => { renderUsage(); });
setInterval(() => {
  if (!document.querySelector('#uList button:focus')) renderUsage();
}, 30000);

/* ================= Quick Actions ================= */

function setupQuickActions(): void {
  $('icoWifi').innerHTML = SVG_ICONS.wifi;
  $('icoBt').innerHTML = SVG_ICONS.bluetooth;
  $('icoFocus').innerHTML = SVG_ICONS.focus;
  $('icoCapture').innerHTML = SVG_ICONS.capture;
  $('prevPage').innerHTML = SVG_ICONS.navLeft;
  $('nextPage').innerHTML = SVG_ICONS.navRight;
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

  $('openSettingsBtn').onclick = (e) => {
    e.stopPropagation();
    runAction(openSettingsCmd(), 'Open settings');
  };
}

function showAnnouncement(text:string):void {
  $('statusMessage').textContent=text;
  const returnOpen = state === 'open';
  if(returnOpen && (pageIndex !== 0 || keyboardMode))return;
  announcement={text:text.slice(0,180),until:Date.now()+3500};
  window.clearTimeout(closeTimer);setState('peek');
  closeTimer=window.setTimeout(()=>{announcement=null;setState(returnOpen && (hoverInside || livePreferences.pinned) ? 'open' : hoverInside?'peek':'idle');},3500);
}
window.addEventListener('tuck-activity',event=>{
  const detail=(event as CustomEvent).detail;
  if(!detail||typeof detail.id!=='string'||typeof detail.title!=='string'||typeof detail.text!=='string'||!Number.isFinite(detail.priority))return;
  if(detail.active)externalActivities.set(detail.id,{title:detail.title.slice(0,80),text:detail.text.slice(0,80),priority:Math.max(0,Math.min(100,detail.priority))});else externalActivities.delete(detail.id);
  renderCompact();
});
window.addEventListener('tuck-preview',event=>{
  const detail=(event as CustomEvent).detail;
  if(detail&&typeof detail.title==='string'&&typeof detail.text==='string')showAnnouncement(`${detail.title}: ${detail.text}`);
});
function holdDrag(active:boolean):void {
  nativeDragActive=active;window.clearTimeout(closeTimer);
  if(active){setState('open');const index=pages.findIndex(page=>page.dataset.page==='extensions');if(index>=0)showPage(index);}
  else onHover(hoverInside);
}
window.addEventListener('tuck-drop-active',event=>holdDrag((event as CustomEvent).detail===true));
window.addEventListener('tuck-drag-active',event=>holdDrag((event as CustomEvent).detail===true));
window.addEventListener('blur',()=>{keyboardMode=false;if(!nativeDragActive)onHover(hoverInside);});

/* ================= Subscriptions & Native Events ================= */

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
  if (livePreferences.desktopPeeks && previous.statusKnown && payload.statusKnown && previous.acConnected !== payload.acConnected) showAnnouncement(payload.acConnected ? 'Charger connected' : 'Charger disconnected');
});
void listenToEvent<{title: string; text: string}>('desktop-peek', payload => {
  if (livePreferences.desktopPeeks && typeof payload?.title === 'string' && typeof payload.text === 'string') showAnnouncement(`${payload.title}: ${payload.text}`);
});
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
  runAction(invokeCommand<MediaState>('media_now').then(onMediaUpdate), 'Read media session');
  runAction(invokeCommand<boolean>('fullscreen_now').then(hidden => document.body.classList.toggle('fullscreen-hidden', hidden)), 'Read fullscreen state');
}

// Fetch initial usage if running in native app
void fetchUsageNow().then((initial) => {
  if (initial && initial.length > 0) {
    onUsageUpdate(initial);
  }
});

/* ================= Initialization ================= */

setupActivitiesUI();
setupQuickActions();
showPage(0);
renderToday();
renderPower();
renderUsage();
renderMedia();
renderCompact();
setState('idle');

// Refresh usage meters periodically
setInterval(() => {
  void fetchUsageNow();
}, 60000);
