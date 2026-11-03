import { describe, expect, test } from 'bun:test';
import { historyPath, lyricIndex, matchesLyricRequest, measuredPercent, normalizePreferences, parseLrc, TelemetryGate } from '../live-state';

describe('live island state', () => {
  test('bounds saved geometry and delays while retaining explicit choices', () => {
    const prefs = normalizePreferences({ openWidth: 9000, openDelayMs: -1, closeDelayMs: Infinity, hoverExpand: false, pinned: true });
    expect(prefs.openWidth).toBe(500); expect(prefs.openDelayMs).toBe(0); expect(prefs.closeDelayMs).toBe(0);
    expect(prefs.hoverExpand).toBe(false); expect(prefs.pinned).toBe(true);
  });
  test('collapses instantly by default and allows zero', () => {
    expect(normalizePreferences({}).closeDelayMs).toBe(0);
    expect(normalizePreferences({ closeDelayMs: -50, prefsVersion: 2 }).closeDelayMs).toBe(0);
    expect(normalizePreferences({ closeDelayMs: 400, prefsVersion: 2 }).closeDelayMs).toBe(400);
  });
  test('moves the old 900ms default to instant once, but keeps a chosen 900ms', () => {
    const migrated = normalizePreferences({ closeDelayMs: 900 });
    expect(migrated.closeDelayMs).toBe(0);
    expect(migrated.prefsVersion).toBe(2);
    expect(normalizePreferences({ closeDelayMs: 900, prefsVersion: 2 }).closeDelayMs).toBe(900);
    expect(normalizePreferences({ closeDelayMs: 1500 }).closeDelayMs).toBe(1500);
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
  test('lyrics reject old A events after A to B to A and uncorrelated events', () => {
    const current = { trackId: 'A', requestId: 'second-A' };
    expect(matchesLyricRequest(current, { trackId: 'A', requestId: 'first-A' })).toBe(false);
    expect(matchesLyricRequest(current, { trackId: 'A' })).toBe(false);
    expect(matchesLyricRequest(current, { trackId: 'B', requestId: 'second-A' })).toBe(false);
    expect(matchesLyricRequest(current, { trackId: 'A', requestId: 'second-A' })).toBe(true);
  });
  test('telemetry gates concurrent polls and rejects older or duplicate snapshots', () => {
    const gate = new TelemetryGate();
    expect(gate.begin()).toBe(true); expect(gate.begin()).toBe(false);
    expect(gate.accept(200)).toBe(true); expect(gate.accept(100)).toBe(false);
    expect(gate.accept(200)).toBe(false); expect(gate.accept(NaN)).toBe(false);
    gate.finish(); expect(gate.begin()).toBe(true); expect(gate.sampledAt).toBe(200);
  });
});
