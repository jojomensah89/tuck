export const LIVE_SETTINGS_KEY = 'tuck.live.v2';
export interface LivePreferences {
  hoverExpand: boolean; openDelayMs: number; closeDelayMs: number; pinned: boolean;
  compactContent: 'lyrics' | 'title'; lyricsOpen: boolean; openWidth: number;
}
export const DEFAULT_LIVE_PREFERENCES: LivePreferences = {
  hoverExpand: true, openDelayMs: 180, closeDelayMs: 900, pinned: false,
  compactContent: 'lyrics', lyricsOpen: true, openWidth: 500,
};
export function normalizePreferences(input: unknown): LivePreferences {
  const p = input && typeof input === 'object' ? input as Partial<LivePreferences> : {};
  const bounded = (value: unknown, fallback: number, min: number, max: number) =>
    typeof value === 'number' && Number.isFinite(value) ? Math.round(Math.max(min, Math.min(max, value))) : fallback;
  return {
    hoverExpand: typeof p.hoverExpand === 'boolean' ? p.hoverExpand : true,
    pinned: p.pinned === true, lyricsOpen: p.lyricsOpen !== false,
    compactContent: p.compactContent === 'title' ? 'title' : 'lyrics',
    openDelayMs: bounded(p.openDelayMs, 180, 0, 2000),
    closeDelayMs: bounded(p.closeDelayMs, 900, 100, 10000),
    openWidth: bounded(p.openWidth, 500, 440, 500),
  };
}
export function readPreferences(): LivePreferences {
  try { return normalizePreferences(JSON.parse(localStorage.getItem(LIVE_SETTINGS_KEY) || '{}')); }
  catch { return { ...DEFAULT_LIVE_PREFERENCES }; }
}
export interface LyricLine { atMs: number; text: string }
/** Parse line-timed LRC, preserving repeated timestamps and applying its offset. */
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
      result.push({ atMs: Math.max(0, Number(match[1]) * 60000 + Number(match[2]) * 1000 + fraction + offset), text: label });
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
