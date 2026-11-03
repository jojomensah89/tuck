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
