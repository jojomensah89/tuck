/**
 * Volume and brightness HUD: pure rules for the island's level display and for
 * turning wheel and middle-click input over the pill-sized strip into volume
 * changes. The DOM side lives in main.ts; native levels arrive as `system-level`
 * events.
 */
import type { IslandState } from './types.js';

export type HudKind = 'volume' | 'brightness';

export interface HudReading {
  kind: HudKind;
  /** 0-100 */
  level: number;
  /** Volume only */
  muted: boolean;
}

/** The HUD leaves this long after the last change */
export const HUD_DISMISS_MS = 1500;
/** Screen readers hear the settled value once input pauses this long */
export const HUD_ANNOUNCE_QUIET_MS = 700;
/** Wheel step in percent; Shift takes the large step */
export const VOLUME_STEP = 2;
export const VOLUME_STEP_LARGE = 10;
/** Pixels of wheel travel per step (one notch of a Windows mouse wheel at 100%) */
const WHEEL_NOTCH = 100;
/** Per-event cap, so a fast flick on a free-spinning wheel cannot jump to 0 */
const MAX_STEPS_PER_EVENT = 5;

/** Validates a native payload; anything malformed is dropped. */
export function readHudPayload(value: unknown): HudReading | null {
  if (!value || typeof value !== 'object') return null;
  const payload = value as Partial<Record<keyof HudReading, unknown>>;
  if (payload.kind !== 'volume' && payload.kind !== 'brightness') return null;
  if (typeof payload.level !== 'number' || !Number.isFinite(payload.level)) return null;
  return {
    kind: payload.kind,
    level: Math.round(Math.max(0, Math.min(100, payload.level))),
    muted: payload.kind === 'volume' && payload.muted === true,
  };
}

/**
 * Next volume after a wheel step, matching `step_volume` in desktop_signals.rs:
 * off-grid values snap to the grid first (37 +2 is 38, 37 -10 is 30). Used only
 * by the browser demo; the desktop app steps natively.
 */
export function stepVolume(current: number, delta: number): number {
  const level = Math.round(Math.max(0, Math.min(100, current)));
  const step = Math.max(1, Math.min(100, Math.abs(Math.trunc(delta))));
  let next = level;
  if (delta > 0) next = (Math.floor(level / step) + 1) * step;
  else if (delta < 0) next = level % step === 0 ? level - step : Math.floor(level / step) * step;
  return Math.max(0, Math.min(100, next));
}

export interface WheelSample {
  deltaX: number;
  deltaY: number;
  /** 0 pixels, 1 lines, 2 pages */
  deltaMode: number;
  shiftKey: boolean;
}

/**
 * Converts one wheel event into a signed volume change in percent. Small trackpad
 * deltas accumulate in `rest` until they add up to a step. Wheel up raises the
 * volume. Shift+wheel arrives as horizontal on Windows, so it is read from deltaX;
 * a plain horizontal tilt does nothing.
 */
export function wheelVolumeDelta(rest: number, sample: WheelSample): { delta: number; rest: number } {
  const scale = sample.deltaMode === 1 ? 40 : sample.deltaMode === 2 ? 800 : 1;
  const vertical = Math.abs(sample.deltaY) >= Math.abs(sample.deltaX);
  const raw = vertical ? sample.deltaY : sample.shiftKey ? sample.deltaX : 0;
  if (!Number.isFinite(raw) || raw === 0) return { delta: 0, rest };
  const total = rest + raw * scale;
  const notches = Math.sign(total) * Math.floor(Math.abs(total) / WHEEL_NOTCH + 0.5);
  if (notches === 0) return { delta: 0, rest: total };
  const steps = Math.sign(notches) * Math.min(MAX_STEPS_PER_EVENT, Math.abs(notches));
  const size = sample.shiftKey ? VOLUME_STEP_LARGE : VOLUME_STEP;
  // Leftover travel carries into the next event, but never a whole extra notch
  const carry = Math.max(-WHEEL_NOTCH / 2, Math.min(WHEEL_NOTCH / 2, total - notches * WHEEL_NOTCH));
  return { delta: -steps * size, rest: carry };
}

/** The island's box in client coordinates */
export interface IslandBox {
  left: number;
  top: number;
  width: number;
}

export interface PillSize {
  width: number;
  height: number;
}

/**
 * Whether a point is over the strip at the island's top centre that has the idle
 * pill's size. The strip is measured from the island's centre and top, which stay
 * put while the shell morphs.
 */
export function overPillStrip(island: IslandBox, pill: PillSize, x: number, y: number): boolean {
  const left = island.left + island.width / 2 - pill.width / 2;
  return x >= left && x < left + pill.width && y >= island.top && y < island.top + pill.height;
}

/**
 * Whether wheel and middle click at a point mean volume. The collapsed island is
 * all pill; on the open island only the strip is, and the wheel switches tabs
 * everywhere else.
 */
export function levelInputApplies(state: IslandState, island: IslandBox, pill: PillSize, x: number, y: number): boolean {
  return state !== 'open' || overPillStrip(island, pill, x, y);
}

export type VolumeGlyph = 'volumeMute' | 'volumeOff' | 'volumeLow' | 'volumeHigh';

export function volumeGlyph(level: number, muted: boolean): VolumeGlyph {
  if (muted) return 'volumeMute';
  if (level <= 0) return 'volumeOff';
  return level < 50 ? 'volumeLow' : 'volumeHigh';
}

/** Accessible name and live-region text, e.g. "Volume 40%" or "Volume muted, 40%" */
export function hudLabel(reading: HudReading): string {
  const name = reading.kind === 'volume' ? 'Volume' : 'Brightness';
  return reading.muted ? `${name} muted, ${reading.level}%` : `${name} ${reading.level}%`;
}

/** Where the island goes when a peek (HUD or announcement) ends */
export function peekReturnTarget(context: {
  /** The island was open before the peek took over */
  returnOpen: boolean;
  hover: boolean;
  pinned: boolean;
  dragging: boolean;
}): 'open' | 'peek' | 'idle' {
  if (context.returnOpen && (context.hover || context.pinned)) return 'open';
  // Hovering stays at peek so scrolling never opens the island
  return context.hover || context.dragging ? 'peek' : 'idle';
}

/** A charger change ramps the panel's brightness for a few seconds on many laptops */
export const CHARGER_RAMP_MS = 8000;

/**
 * Whether a native level event gets a HUD: the Volume and brightness HUD setting
 * must be on, and brightness is ignored right after a charger change. While the
 * Windows flyout is hidden the island is the only feedback, so the charger quiet
 * period does not apply. Wheel and middle-click feedback on the island itself does
 * not pass through here.
 */
export function levelEventShowsHud(
  reading: HudReading,
  enabled: boolean,
  now: number,
  lastChargerChange: number | null,
  flyoutHidden: boolean,
): boolean {
  if (!enabled) return false;
  return reading.kind !== 'brightness'
    || flyoutHidden
    || lastChargerChange === null
    || now - lastChargerChange >= CHARGER_RAMP_MS;
}

/** Announcements older than this are no longer worth interrupting anything for */
const ANNOUNCEMENT_QUEUE_MAX_AGE_MS = 15000;
const ANNOUNCEMENT_QUEUE_LIMIT = 3;

/**
 * Text that arrived while the HUD owned the shell, or that the HUD interrupted.
 * It is shown once the HUD settles, oldest first.
 */
export class AnnouncementQueue {
  private items: { text: string; at: number }[] = [];

  /** Waits behind whatever is already queued */
  add(text: string, now: number): void {
    if (this.items.some(item => item.text === text)) return;
    this.items.push({ text, at: now });
    if (this.items.length > ANNOUNCEMENT_QUEUE_LIMIT) this.items.shift();
  }

  /** The announcement the HUD just interrupted goes first */
  addFirst(text: string, now: number): void {
    this.items = this.items.filter(item => item.text !== text);
    this.items.unshift({ text, at: now });
    if (this.items.length > ANNOUNCEMENT_QUEUE_LIMIT) this.items.pop();
  }

  next(now: number): string | null {
    this.items = this.items.filter(item => now - item.at < ANNOUNCEMENT_QUEUE_MAX_AGE_MS);
    return this.items.shift()?.text ?? null;
  }
}

/**
 * Visibility and announcement timing. Each change re-arms the dismiss deadline;
 * the live region only gets the value once changes pause, and the same value is
 * not repeated while the HUD stays up.
 */
export class HudModel {
  reading: HudReading | null = null;
  private until = 0;
  private announceAt = 0;
  private announced = '';

  show(reading: HudReading, now: number): void {
    this.reading = reading;
    this.until = now + HUD_DISMISS_MS;
    this.announceAt = now + HUD_ANNOUNCE_QUIET_MS;
  }

  visible(now: number): boolean {
    return this.reading !== null && now < this.until;
  }

  /** Milliseconds until the HUD should leave (0 when already gone) */
  remaining(now: number): number {
    return this.reading ? Math.max(0, this.until - now) : 0;
  }

  /** The text to announce now, or null when input has not settled or nothing changed */
  dueAnnouncement(now: number): string | null {
    if (!this.reading || now < this.announceAt) return null;
    const text = hudLabel(this.reading);
    if (text === this.announced) return null;
    this.announced = text;
    return text;
  }

  dismiss(): void {
    this.reading = null;
    this.until = 0;
    this.announced = '';
  }
}
