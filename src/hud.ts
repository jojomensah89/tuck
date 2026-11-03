/**
 * Volume and brightness HUD: pure rules for the island's level display and for
 * turning wheel input over the idle pill into volume steps. The DOM side lives in
 * main.ts; native levels arrive as `system-level` events.
 */

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
