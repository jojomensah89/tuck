import { DEFAULT_TAB, isTabId, type TabId } from './tab-nav.js';

export const LIVE_SETTINGS_KEY = 'tuck.live.v2';
export function newRequestId(): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), byte => byte.toString(16).padStart(2, '0')).join('');
}
export interface LivePreferences {
  hoverExpand: boolean; openDelayMs: number; closeDelayMs: number; pinned: boolean;
  fullscreenAutoHide: boolean;
  compactContent: 'artwork' | 'title' | 'lyrics'; lyricsOpen: boolean; openWidth: number;
  desktopPeeks: boolean;
  /** Volume and brightness changes from outside the island show the HUD */
  levelHud: boolean;
  /** Last open-island tab, restored on the next launch */
  lastTab: TabId;
  /** Bounded, per-track lyric timing corrections in 100 ms steps. */
  lyricsOffsets: LyricsOffsetPreference[];
  /** Bumped when a saved default changes meaning; 2 = collapse delay defaults to instant. */
  prefsVersion: number;
}
export const PREFS_VERSION = 2;
export const DEFAULT_LIVE_PREFERENCES: LivePreferences = {
  hoverExpand: true, openDelayMs: 180, closeDelayMs: 0, pinned: false,
  compactContent: 'lyrics', lyricsOpen: true, openWidth: 500,
  desktopPeeks: true, levelHud: true, fullscreenAutoHide: true, lastTab: DEFAULT_TAB, lyricsOffsets: [], prefsVersion: PREFS_VERSION,
};
export interface LyricsOffsetPreference { key: string; offsetMs: number }
export const MAX_LYRIC_OFFSETS = 128;
export const MAX_LYRIC_OFFSET_MS = 15000;
const normalizeLyricOffsets = (value: unknown): LyricsOffsetPreference[] => {
  if (!Array.isArray(value)) return [];
  const result: LyricsOffsetPreference[] = [];
  for (const { key, offsetMs } of value.filter(Boolean) as Partial<LyricsOffsetPreference>[]) {
    if (typeof key !== 'string' || !key || key.length > 512 || typeof offsetMs !== 'number' || !Number.isFinite(offsetMs)) continue;
    const stepped = Math.round(Math.max(-MAX_LYRIC_OFFSET_MS, Math.min(MAX_LYRIC_OFFSET_MS, offsetMs)) / 100) * 100;
    if (stepped !== 0) result.push({ key, offsetMs: stepped });
  }
  return result.slice(-MAX_LYRIC_OFFSETS);
};
export function normalizePreferences(input: unknown): LivePreferences {
  const p = input && typeof input === 'object' ? input as Partial<LivePreferences> : {};
  const bounded = (value: unknown, fallback: number, min: number, max: number) =>
    typeof value === 'number' && Number.isFinite(value) ? Math.round(Math.max(min, Math.min(max, value))) : fallback;
  return {
    hoverExpand: typeof p.hoverExpand === 'boolean' ? p.hoverExpand : true,
    pinned: p.pinned === true, lyricsOpen: p.lyricsOpen !== false,
    compactContent: p.compactContent === 'artwork' || p.compactContent === 'title' ? p.compactContent : 'lyrics',
    desktopPeeks: p.desktopPeeks !== false,
    // Before this setting existed, System peeks also carried the level peeks
    levelHud: typeof p.levelHud === 'boolean' ? p.levelHud : p.desktopPeeks !== false,
    fullscreenAutoHide: p.fullscreenAutoHide !== false,
    openDelayMs: bounded(p.openDelayMs, 180, 0, 2000),
    // Settings saved before version 2 that still hold the old 900ms default move to instant once
    closeDelayMs: bounded(
      (p.prefsVersion ?? 1) < 2 && p.closeDelayMs === 900 ? 0 : p.closeDelayMs, 0, 0, 10000,
    ),
    prefsVersion: PREFS_VERSION,
    openWidth: bounded(p.openWidth, 500, 440, 500),
    lastTab: isTabId(p.lastTab) ? p.lastTab : DEFAULT_TAB,
    lyricsOffsets: normalizeLyricOffsets(p.lyricsOffsets),
  };
}
export function readPreferences(): LivePreferences {
  try { return normalizePreferences(JSON.parse(localStorage.getItem(LIVE_SETTINGS_KEY) || '{}')); }
  catch { return { ...DEFAULT_LIVE_PREFERENCES }; }
}
/** Merge a partial update into the stored preferences; storage failures are ignored */
export function writePreferences(update: Partial<LivePreferences>): LivePreferences {
  const next = normalizePreferences({ ...readPreferences(), ...update });
  try { localStorage.setItem(LIVE_SETTINGS_KEY, JSON.stringify(next)); } catch { /* storage disabled */ }
  return next;
}
export function lyricOffsetFor(preferences: LivePreferences, key: string): number {
  return preferences.lyricsOffsets.find(entry => entry.key === key)?.offsetMs ?? 0;
}
export function withLyricOffset(preferences: LivePreferences, key: string, offsetMs: number): LyricsOffsetPreference[] {
  return normalizeLyricOffsets([
    ...preferences.lyricsOffsets.filter(entry => entry.key !== key),
    { key, offsetMs },
  ]);
}
export interface LyricLine { atMs: number; text: string }
/** Parse line-timed LRC, preserving repeated timestamps. A positive offset makes lyrics appear sooner. */
export function parseLrc(text: string): LyricLine[] {
  const offset = Number(text.match(/\[offset:([+-]?\d+)\]/i)?.[1] || 0);
  const result: LyricLine[] = [];
  for (const row of text.slice(0, 250000).split(/\r?\n/)) {
    const times = [...row.matchAll(/\[(\d{1,3}):(\d{2})(?:[.:](\d{1,3}))?\]/g)];
    if (!times.length) continue;
    const label = row.replace(/\[[^\]]*\]/g, '').trim();
    for (const match of times) {
      if (Number(match[2]) >= 60) continue;
      const fraction = Number((match[3] || '').padEnd(3, '0'));
      result.push({ atMs: Math.max(0, Number(match[1]) * 60000 + Number(match[2]) * 1000 + fraction - offset), text: label });
    }
  }
  return result.sort((a, b) => a.atMs - b.atMs).slice(0, 5000);
}
export function lyricIndex(lines: LyricLine[], positionMs: number): number {
  let low = 0, high = lines.length;
  while (low < high) { const mid = (low + high) >>> 1; if (lines[mid].atMs <= positionMs) low = mid + 1; else high = mid; }
  return low - 1;
}
export function measuredPercent(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= 100 ? value : null;
}
export function historyPath(values: (number | null)[]): string {
  // Missing samples create gaps, never artificial drops to zero.
  let active = false;
  return values.map((value, i) => {
    if (value === null) { active = false; return ''; }
    const segment = `${active ? 'L' : 'M'}${(i / Math.max(1, values.length - 1) * 180).toFixed(1)},${(36 - value * .34).toFixed(1)}`;
    active = true; return segment;
  }).join(' ');
}

export function matchesLyricRequest(current: { trackId: string; requestId: string }, candidate: { trackId: string; requestId?: string }): boolean {
  return current.requestId !== '' && candidate.trackId === current.trackId && candidate.requestId === current.requestId;
}

/** One native snapshot call at a time, and no time reversal from events or responses. */
export class TelemetryGate {
  private pending = false;
  private latest = 0;
  private failures = 0;
  get sampledAt(): number { return this.latest; }
  begin(): boolean { if (this.pending) return false; this.pending = true; return true; }
  finish(): void { this.pending = false; }
  accept(sampledAt: number): boolean {
    if (!Number.isFinite(sampledAt) || sampledAt <= this.latest) return false;
    this.latest = sampledAt; this.failures = 0; return true;
  }
  /** Whether failed polls should clear the metrics: one blip must not blank a valid value. */
  failed(): boolean { this.failures++; return this.latest === 0 || this.failures >= 3; }
}
