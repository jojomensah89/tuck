import type { Meter } from './types.js';
import { formatSpan } from './rings.js';

export const STORAGE_KEYS = {
  SHAPE: 'notch.shape',
} as const;

export function getJson<T>(key: string, defaultValue: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) : defaultValue;
  } catch {
    return defaultValue;
  }
}

export function saveJson<T>(key: string, value: T): boolean {
  try {
    localStorage.setItem(key, JSON.stringify(value));
    return true;
  } catch {
    return false;
  }
}

/**
 * Time until a reset in the island's one duration format ("2h 15m"), at least a
 * minute; "now" once it has passed and '' when there is no reset time.
 */
export function resetSpan(resetsAt: number | null, nowSeconds = Date.now() / 1000): string {
  if (!resetsAt) return '';
  const diff = resetsAt - nowSeconds;
  if (diff <= 0) return 'now';
  return formatSpan(Math.max(60, diff));
}

/**
 * Formats countdown string to human-readable reset string.
 */
export function formatUntil(resetsAt: number | null, nowSeconds = Date.now() / 1000): string {
  const span = resetSpan(resetsAt, nowSeconds);
  if (!span) return '';
  return span === 'now' ? 'resetting now' : `resets in ${span}`;
}

/** Only subscription quota meters are supported. Old local estimates and budgets are ignored. */
export function mergeMeters(nativeMeters: Meter[]): Meter[] {
  return nativeMeters.filter(m => m.source === 'official').map(m => ({...m,
    usedPct: m.usedPct != null ? Math.max(0, Math.min(100, m.usedPct)) : null,
  }));
}

/** Native emits only connected sources; this keeps an unconfigured reading off the island regardless. */
export function connectedUsageMeters(meters: Meter[]): Meter[] {
  return meters.filter(meter => meter.status !== 'unconfigured');
}

/**
 * Detects if any meter had a significant drop (> 30%) indicating a quota reset.
 */
export function checkResets(
  newList: Meter[],
  previousPcts: Record<string, number>,
  onReset: (meter: Meter) => void
): Record<string, number> {
  const updatedPcts: Record<string, number> = { ...previousPcts };

  for (const meter of newList) {
    if (meter.usedPct == null || meter.stale || !Number.isFinite(meter.usedPct) || meter.source !== 'official') continue;
    const prev = previousPcts[meter.id];
    if (prev != null && prev - meter.usedPct > 30) {
      onReset(meter);
    }
    updatedPcts[meter.id] = meter.usedPct;
  }

  return updatedPcts;
}
