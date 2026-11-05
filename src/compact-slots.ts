export type CompactActivityKind = 'media' | 'timer' | 'dictation' | 'external';

export interface CompactActivityCandidate {
  id: string;
  kind: CompactActivityKind;
  title: string;
  priority: number;
  active: boolean;
  running: boolean;
}

export interface CompactSlotSelection {
  primary: CompactActivityCandidate | null;
  hasBackgroundActivity: boolean;
}

export type CompactContent = 'artwork' | 'title' | 'lyrics';
export const COMPACT_IDLE_WIDTH = 212;
export const COMPACT_TEXT_WIDTH = 300;

/** Text modes reserve one width for the track instead of resizing on each lyric. */
export function compactShellWidth(
  primary: CompactActivityCandidate | null,
  content: CompactContent,
): number {
  return primary?.kind === 'dictation' || (primary?.kind === 'media' && content !== 'artwork') ? COMPACT_TEXT_WIDTH : COMPACT_IDLE_WIDTH;
}

/** Share of the one-shot marquee spent travelling; the rest holds, then returns to the start */
const COMPACT_SCROLL_TRAVEL = 0.7;
const COMPACT_SCROLL_PX_PER_S = 60;
const COMPACT_SCROLL_MIN_MS = 2000;
const COMPACT_SCROLL_TITLE_MAX_MS = 12000;
/** Lines change every few seconds, so a long one travels faster and finishes inside a 4s line */
const COMPACT_SCROLL_LYRIC_MAX_MS = 3200;

/** Length of the one scroll a collapsed line gets, hold and return included */
export function compactScrollMs(overflowPx: number, lyric: boolean): number {
  const ms = (overflowPx / COMPACT_SCROLL_PX_PER_S) * 1000 / COMPACT_SCROLL_TRAVEL;
  return Math.round(Math.max(COMPACT_SCROLL_MIN_MS, Math.min(lyric ? COMPACT_SCROLL_LYRIC_MAX_MS : COMPACT_SCROLL_TITLE_MAX_MS, ms)));
}

/**
 * Chooses the activity that owns compact leading/trailing slots.
 * Equal priorities remain in caller order so the visible activity does not
 * flicker when independent event sources update together.
 */
export function selectCompactActivity(
  candidates: readonly CompactActivityCandidate[],
): CompactSlotSelection {
  const active = candidates
    .map((candidate, index) => ({ candidate, index }))
    .filter(({ candidate }) => candidate.active)
    .sort((a, b) => b.candidate.priority - a.candidate.priority || a.index - b.index)
    .map(({ candidate }) => candidate);

  return {
    primary: active[0] ?? null,
    hasBackgroundActivity: active.some(candidate => candidate.running && candidate !== active[0]),
  };
}
