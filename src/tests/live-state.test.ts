import { describe, expect, test } from 'bun:test';
import { historyPath, lyricIndex, measuredPercent, normalizePreferences, parseLrc } from '../live-state';
import { dueTodos, normalizeTodos } from '../todos';

describe('live island state', () => {
  test('bounds saved geometry and delays while retaining explicit choices', () => {
    const prefs = normalizePreferences({ openWidth: 9000, openDelayMs: -1, closeDelayMs: Infinity, hoverExpand: false, pinned: true });
    expect(prefs.openWidth).toBe(500); expect(prefs.openDelayMs).toBe(0); expect(prefs.closeDelayMs).toBe(900);
    expect(prefs.hoverExpand).toBe(false); expect(prefs.pinned).toBe(true);
  });
  test('parses repeated LRC timestamps, decimals, offsets and invalid seconds', () => {
    expect(parseLrc('[offset:-500]\n[00:01.5][00:03.25]Line\n[01:99]bad')).toEqual([{ atMs: 1000, text: 'Line' }, { atMs: 2750, text: 'Line' }]);
  });
  test('timing responds to backward seeks and prelude without invented timing', () => {
    const lines = parseLrc('[00:02]One\n[00:05]Two');
    expect(lyricIndex(lines, 1000)).toBe(-1); expect(lyricIndex(lines, 6000)).toBe(1); expect(lyricIndex(lines, 2000)).toBe(0);
    expect(parseLrc('Plain lyrics have no fabricated timestamps')).toEqual([]);
  });
  test('unknown metrics stay unknown and histories have gaps', () => {
    expect(measuredPercent(null)).toBeNull(); expect(measuredPercent(NaN)).toBeNull(); expect(measuredPercent(140)).toBeNull();
    expect(measuredPercent(0)).toBe(0); expect(historyPath([30, null, 50])).toBe('M0.0,25.8  M180.0,19.0');
  });
  test('reminders ignore completed, already reminded and invalid persisted tasks', () => {
    const todos = normalizeTodos([null, { id: 'a', text: '<not markup>', dueAt: 10 }, { id: 'b', text: 'Done', done: true, dueAt: 10 }, { id: 'c', text: 'Seen', reminded: true, dueAt: 10 }]);
    expect(dueTodos(todos, 20).map(todo => todo.id)).toEqual(['a']);
    expect(normalizeTodos({ id: 'corrupt' })).toEqual([]);
  });
});
