import './setup.js';
import { describe, expect, it } from 'bun:test';
import {
  compactScrollMs,
  compactShellWidth,
  selectCompactActivity,
  type CompactActivityCandidate,
} from '../compact-slots.js';

const activity = (
  overrides: Partial<CompactActivityCandidate> = {},
): CompactActivityCandidate => ({
  id: 'media',
  kind: 'media',
  title: 'Now playing',
  priority: 50,
  active: true,
  running: true,
  ...overrides,
});

describe('compact slot activity priority', () => {
  it('gives the slots to the highest-priority active activity', () => {
    const selection = selectCompactActivity([
      activity(),
      activity({ id: 'timer', kind: 'timer', title: 'Timer', priority: 80 }),
      activity({ id: 'dictation', kind: 'dictation', title: 'Dictation', priority: 99 }),
    ]);

    expect(selection.primary?.id).toBe('dictation');
    expect(selection.hasBackgroundActivity).toBe(true);
  });

  it('ignores inactive candidates and keeps equal priorities stable', () => {
    const selection = selectCompactActivity([
      activity({ id: 'first', kind: 'timer', priority: 80 }),
      activity({ id: 'inactive', kind: 'dictation', priority: 99, active: false }),
      activity({ id: 'second', kind: 'timer', priority: 80 }),
    ]);

    expect(selection.primary?.id).toBe('first');
  });

  it('counts only actually running activities for the background dot', () => {
    const selection = selectCompactActivity([
      activity({ running: false }),
      activity({ id: 'timer', kind: 'timer', title: 'Timer', priority: 80 }),
    ]);

    expect(selection.primary?.id).toBe('timer');
    expect(selection.hasBackgroundActivity).toBe(false);
  });

  it('returns the idle selection when nothing is active', () => {
    expect(selectCompactActivity([
      activity({ active: false, running: false }),
    ])).toEqual({
      primary: null,
      hasBackgroundActivity: false,
    });
  });
});

describe('compact slot background dot', () => {
  it('shows the dot when a paused primary hides a running activity', () => {
    const selection = selectCompactActivity([
      activity({ priority: 90, running: false }),
      activity({ id: 'timer', kind: 'timer', title: 'Timer', priority: 60 }),
    ]);

    expect(selection.primary?.id).toBe('media');
    expect(selection.hasBackgroundActivity).toBe(true);
  });

  it('hides the dot when only the primary is running', () => {
    const selection = selectCompactActivity([
      activity({ priority: 90 }),
      activity({ id: 'timer', kind: 'timer', title: 'Timer', priority: 60, running: false }),
    ]);

    expect(selection.hasBackgroundActivity).toBe(false);
  });
});

describe('collapsed media width', () => {
  it('reserves one stable width for title and lyric modes only while media owns the slot', () => {
    const media = activity();
    expect(compactShellWidth(media, 'lyrics')).toBe(300);
    expect(compactShellWidth(media, 'title')).toBe(300);
    expect(compactShellWidth(media, 'artwork')).toBe(212);
    expect(compactShellWidth(activity({ kind: 'timer' }), 'lyrics')).toBe(212);
    expect(compactShellWidth(null, 'title')).toBe(212);
  });
});

describe('collapsed one-shot scroll', () => {
  it('fits a long lyric inside a 4s line, lead included', () => {
    expect(compactScrollMs(1000, true)).toBe(3200);
    expect(300 + compactScrollMs(1000, true)).toBeLessThan(4000);
  });

  it('gives titles reading time and bounds the longest', () => {
    expect(compactScrollMs(10, false)).toBe(2000);
    expect(compactScrollMs(210, false)).toBe(5000);
    expect(compactScrollMs(5000, false)).toBe(12000);
  });
});
