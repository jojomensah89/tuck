/**
 * Progress rings: the geometry, the threshold tones and the short labels that
 * sit under them. The pure helpers are unit tested; `createRing` is the one DOM
 * builder, and its value changes animate through CSS (spring tokens).
 */
import type { Activity, Meter } from './types.js';

export type RingTone = 'ok' | 'caution' | 'warn' | 'ink' | 'muted' | 'none';

/** Usage tones: a ring turns amber at 70% and red at 90% */
export const USAGE_CAUTION_PCT = 70;
export const USAGE_WARN_PCT = 90;
/** A discharging battery turns red below this level */
export const BATTERY_LOW_PCT = 20;

export interface RingGeometry {
  /** Radius of the stroke's centre line */
  r: number;
  /** Circumference, the full dash length */
  c: number;
  /** Centre of the square viewBox */
  center: number;
}

/** The stroke sits fully inside a `size` square */
export function ringGeometry(size: number, stroke: number): RingGeometry {
  const r = Math.max(0, (size - stroke) / 2);
  return { r, c: 2 * Math.PI * r, center: size / 2 };
}

/** A finite percentage clamped to 0..100, or null when there is no value */
export function clampPct(pct: number | null | undefined): number | null {
  if (pct == null || !Number.isFinite(pct)) return null;
  return Math.max(0, Math.min(100, pct));
}

/** Dash offset that leaves `pct` of the circumference drawn */
export function ringDashOffset(pct: number | null | undefined, c: number): number {
  const value = clampPct(pct) ?? 0;
  return c * (1 - value / 100);
}

/** Threshold colour for a quota: the logo names the provider, the colour says how close the limit is */
export function usageTone(pct: number | null | undefined): RingTone {
  const value = clampPct(pct);
  if (value === null) return 'none';
  if (value >= USAGE_WARN_PCT) return 'warn';
  if (value >= USAGE_CAUTION_PCT) return 'caution';
  return 'ok';
}

/** Battery: green while charging, red when low on battery, otherwise ink */
export function batteryTone(pct: number | null | undefined, charging: boolean): RingTone {
  const value = clampPct(pct);
  if (value === null) return 'none';
  if (charging) return 'ok';
  return value < BATTERY_LOW_PCT ? 'warn' : 'ink';
}

/**
 * One duration format for the whole island: "3d 4h", "2h 15m", "45m", "<1m".
 * Seconds at or below zero read as "now".
 */
export function formatSpan(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return 'now';
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (d > 0) return h > 0 ? `${d}d ${h}h` : `${d}d`;
  if (h > 0) return m > 0 ? `${h}h ${m}m` : `${h}h`;
  return m > 0 ? `${m}m` : '<1m';
}

/** Which family of mark identifies a meter's provider; null falls back to an initial */
export type ProviderMark = 'anthropic' | 'openai';

export function providerMark(provider: string): ProviderMark | null {
  const p = provider.toLowerCase();
  if (p === 'claude' || p.startsWith('anthropic')) return 'anthropic';
  if (p === 'codex' || p === 'chatgpt' || p.startsWith('openai')) return 'openai';
  return null;
}

/** The provider's initial, for providers without a mark */
export function providerInitial(meter: Pick<Meter, 'provider' | 'label'>): string {
  const source = meter.label.trim() || meter.provider.trim();
  return (source[0] || '?').toUpperCase();
}

/**
 * The short window name under a ring, so two rings with the same mark stay
 * apart: "5h", "7d", "API", "Est".
 */
export function meterWindow(meter: Pick<Meter, 'id' | 'label' | 'source'>): string {
  if (meter.source === 'manual') return 'Est';
  if (meter.source === 'api') return 'API';
  // The label is written for people; the id is the fallback
  for (const raw of [meter.label, meter.id]) {
    const text = raw.toLowerCase();
    const minutes = text.match(/\b(\d+)\s*-?\s*min(?:ute)?s?\b/);
    if (minutes) return `${minutes[1]}m`;
    const hours = text.match(/\b(\d+)\s*-?\s*h(?:our|r)?s?\b/);
    if (hours) return `${hours[1]}h`;
    const days = text.match(/\b(\d+)\s*-?\s*d(?:ay)?s?\b/);
    if (days) return `${days[1]}d`;
    if (/week/.test(text)) return '7d';
    if (/month/.test(text)) return '30d';
  }
  return '';
}

/**
 * Fraction of the timer ring to draw: the remaining share of a countdown, or
 * the seconds sweep of a stopwatch. Null when nothing runs.
 */
export function activityProgress(act: Activity | null, now = Date.now()): number | null {
  if (!act || !act.active) return null;
  if (act.type === 'stopwatch') {
    const elapsed = act.paused ? act.accumulatedMs || 0 : act.startTime ? now - act.startTime : 0;
    return (Math.max(0, elapsed) % 60000) / 60000;
  }
  const total = act.totalDurationMs || 0;
  if (total <= 0) return null;
  const remaining = act.paused ? act.remainingMs || 0 : act.targetEndTime ? act.targetEndTime - now : 0;
  return Math.max(0, Math.min(1, remaining / total));
}

export interface Ring {
  root: SVGSVGElement;
  /** Sets the drawn share (0..100, null for an empty track) and the tone */
  set(pct: number | null, tone?: RingTone): void;
}

const SVG_NS = 'http://www.w3.org/2000/svg';

/**
 * A ring drawn in a `size` square viewBox; CSS sets its rendered size. The fill
 * starts at 12 o'clock and runs clockwise. Values change by dash offset, so the
 * CSS transition on `stroke-dashoffset` animates them.
 */
export function createRing(size: number, stroke: number, className = ''): Ring {
  const { r, c, center } = ringGeometry(size, stroke);
  const root = document.createElementNS(SVG_NS, 'svg');
  root.setAttribute('class', `ring${className ? ` ${className}` : ''}`);
  root.setAttribute('viewBox', `0 0 ${size} ${size}`);
  root.setAttribute('aria-hidden', 'true');
  const circle = (cls: string) => {
    const el = document.createElementNS(SVG_NS, 'circle');
    el.setAttribute('class', cls);
    el.setAttribute('cx', String(center));
    el.setAttribute('cy', String(center));
    el.setAttribute('r', String(r));
    el.setAttribute('stroke-width', String(stroke));
    return el;
  };
  const track = circle('ring-track');
  const fill = circle('ring-fill');
  fill.setAttribute('stroke-dasharray', c.toFixed(3));
  fill.setAttribute('stroke-dashoffset', c.toFixed(3));
  fill.setAttribute('transform', `rotate(-90 ${center} ${center})`);
  root.append(track, fill);
  fill.style.strokeDashoffset = c.toFixed(3);
  let pending = 0;
  return {
    root,
    set(pct, tone = 'ink') {
      const value = clampPct(pct);
      root.dataset.tone = value === null ? 'none' : tone;
      root.dataset.empty = String(value === null || value <= 0);
      // An inline style (not the attribute) so the CSS transition runs. A new
      // ring waits until it has been styled once, so it fills in from empty.
      const offset = ringDashOffset(value, c).toFixed(3);
      const apply = () => { fill.style.strokeDashoffset = offset; };
      cancelAnimationFrame(pending);
      if (root.isConnected && root.dataset.styled) apply();
      else pending = requestAnimationFrame(() => { pending = requestAnimationFrame(() => { root.dataset.styled = '1'; apply(); }); });
    },
  };
}
