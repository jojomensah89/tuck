import type { Meter, ChatGPTConfig } from './types.js';
import { formatSpan } from './rings.js';

export const STORAGE_KEYS = {
  SHAPE: 'notch.shape',
  BUDGETS: 'notch.budgets',
  CHATGPT: 'notch.chatgpt',
  CHATGPT_LOG: 'notch.chatgpt.log',
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

export function getNextMonthStartUtc(): number {
  const n = new Date();
  return Date.UTC(n.getUTCFullYear(), n.getUTCMonth() + 1, 1, 0, 0, 0) / 1000;
}

/**
 * Calculates manual ChatGPT rolling counter meter based on user config and timestamps.
 */
export function getChatGPTMeter(nowSeconds = Date.now() / 1000): Meter | null {
  const cfg = getJson<ChatGPTConfig | null>(STORAGE_KEYS.CHATGPT, null);
  if (!cfg || !Number.isFinite(cfg.cap) || cfg.cap <= 0 || !Number.isFinite(cfg.hours) || cfg.hours <= 0) {
    return null;
  }

  const windowSeconds = cfg.hours * 3600;
  const since = nowSeconds - windowSeconds;

  // Filter timestamps within active rolling window
  const storedLogs = getJson<unknown>(STORAGE_KEYS.CHATGPT_LOG, []);
  const allLogs: unknown[] = Array.isArray(storedLogs) ? storedLogs : [];
  const liveLogs = allLogs.filter((t): t is number => typeof t === 'number' && Number.isFinite(t) && t > since && t <= nowSeconds).sort((a, b) => a - b);

  const count = liveLogs.length;
  const usedPct = (count / cfg.cap) * 100;
  const oldest = liveLogs[0];
  const resetsAt = oldest != null ? Math.round(oldest + windowSeconds) : null;

  return {
    id: 'chatgpt',
    provider: 'chatgpt',
    label: typeof cfg.label === 'string' && cfg.label ? cfg.label : 'ChatGPT (estimate)',
    usedPct,
    spentUsd: null,
    resetsAt,
    source: 'manual',
    stale: false,
    count,
    cap: cfg.cap,
  };
}

/**
 * Logs a new ChatGPT message timestamp into the rolling window log.
 */
export function logChatGPTMessage(timestampSeconds = Math.floor(Date.now() / 1000)): void {
  const storedLogs = getJson<unknown>(STORAGE_KEYS.CHATGPT_LOG, []);
  const allLogs: unknown[] = Array.isArray(storedLogs) ? storedLogs : [];
  const cfg = getJson<ChatGPTConfig | null>(STORAGE_KEYS.CHATGPT, null);
  const hours = cfg && Number.isFinite(cfg.hours) && cfg.hours > 0 ? cfg.hours : 48;
  const pruned = allLogs.filter((t): t is number => typeof t === 'number' && Number.isFinite(t) && t > timestampSeconds - hours * 3600 && t <= timestampSeconds);
  pruned.push(timestampSeconds);
  saveJson(STORAGE_KEYS.CHATGPT_LOG, pruned);
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

/**
 * Merges native meters with user budgets and manual ChatGPT meter.
 */
export function mergeMeters(nativeMeters: Meter[]): Meter[] {
  const budgets = getJson<Record<string, number>>(STORAGE_KEYS.BUDGETS, {});
  const nextMonth = getNextMonthStartUtc();

  const merged: Meter[] = nativeMeters.map((m) => {
    if (m.spentUsd != null) {
      const budgetCap = Number(budgets?.[m.provider]) || 0;
      const usedPct = budgetCap > 0 ? (m.spentUsd / budgetCap) * 100 : null;
      return {
        ...m,
        usedPct,
        resetsAt: nextMonth,
      };
    }
    return {
      ...m,
      usedPct: m.usedPct != null ? Math.max(0, Math.min(100, m.usedPct)) : null,
    };
  });

  const chatgpt = getChatGPTMeter();
  if (chatgpt) {
    merged.push(chatgpt);
  }

  return merged;
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
