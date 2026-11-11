import { describe, expect, test } from 'bun:test';
import { FULLSCREEN_STATS_INTERVAL_MS, FullscreenStatsSampler, fullscreenStats, fullscreenStatsVisible } from '../fullscreen-stats';
import type { TelemetrySnapshot } from '../live-services';

const snapshot = (over: Partial<TelemetrySnapshot> = {}): TelemetrySnapshot => ({
  sampledAt: 1, cpuPct: 12.4, ramUsedBytes: 8, ramTotalBytes: 16,
  gpus: [{ id: '0:iGPU', name: 'iGPU', usedPct: 3 }, { id: '1:dGPU', name: 'dGPU', usedPct: 87.6 }], ...over,
});

describe('fullscreen stats visibility', () => {
  test('shows only when enabled, a fullscreen app is in front and the island is collapsed', () => {
    expect(fullscreenStatsVisible({ enabled: true, fullscreen: true, state: 'idle' })).toBe(true);
    expect(fullscreenStatsVisible({ enabled: false, fullscreen: true, state: 'idle' })).toBe(false);
    expect(fullscreenStatsVisible({ enabled: true, fullscreen: false, state: 'idle' })).toBe(false);
  });
  test('a peek or an open island keeps hiding for fullscreen as before', () => {
    expect(fullscreenStatsVisible({ enabled: true, fullscreen: true, state: 'peek' })).toBe(false);
    expect(fullscreenStatsVisible({ enabled: true, fullscreen: true, state: 'open' })).toBe(false);
  });
});

describe('fullscreen stats readings', () => {
  test('rounds CPU, takes the busiest GPU and computes RAM from bytes', () => {
    const cells = fullscreenStats(snapshot());
    expect(cells.map(c => [c.label, c.text])).toEqual([['CPU', '12%'], ['GPU', '88%'], ['RAM', '50%']]);
    expect(cells.map(c => c.spoken)).toEqual(['CPU 12%', 'GPU 88%', 'Memory 50%']);
  });
  test('shows an en dash for a missing reading and never a stale number', () => {
    const missing = fullscreenStats(snapshot({ cpuPct: null, gpus: [], ramTotalBytes: 0 }));
    expect(missing.map(c => c.text)).toEqual(['–', '–', '–']);
    expect(missing[0].spoken).toBe('CPU unavailable');
    expect(fullscreenStats(snapshot({ stale: true })).map(c => c.text)).toEqual(['–', '–', '–']);
    expect(fullscreenStats(null).map(c => c.text)).toEqual(['–', '–', '–']);
  });
  test('clamps out-of-range values', () => {
    expect(fullscreenStats(snapshot({ cpuPct: 140, gpus: [{ id: 'a', name: 'a', usedPct: -5 }] })).map(c => c.text)).toEqual(['100%', '0%', '50%']);
  });
});

/** Manual clock and timer so the sampler's schedule can be checked without waiting */
function harness(read: () => Promise<TelemetrySnapshot>) {
  let clock = 0;
  const timers = new Map<number, { at: number; run: () => void }>();
  let nextId = 0;
  const samples: (TelemetrySnapshot | null)[] = [];
  const sampler = new FullscreenStatsSampler({
    read, onSample: s => samples.push(s), now: () => clock,
    setTimer: (run, ms) => { timers.set(++nextId, { at: clock + ms, run }); return nextId; },
    clearTimer: handle => { timers.delete(handle as number); },
  });
  const advance = async (ms: number) => {
    clock += ms;
    for (const [id, timer] of [...timers]) if (timer.at <= clock) { timers.delete(id); timer.run(); await Promise.resolve(); await Promise.resolve(); }
  };
  return { sampler, samples, timers, advance };
}

describe('fullscreen stats sampler', () => {
  test('does nothing until started and nothing after stop', async () => {
    let reads = 0;
    const h = harness(async () => { reads++; return snapshot(); });
    await h.advance(5000);
    expect(reads).toBe(0);
    h.sampler.start();
    await h.advance(0);
    expect(reads).toBe(1);
    h.sampler.stop();
    expect(h.timers.size).toBe(0);
    await h.advance(10_000);
    expect(reads).toBe(1);
  });
  test('samples at most once per second', async () => {
    let reads = 0;
    const h = harness(async () => { reads++; return snapshot(); });
    h.sampler.start();
    await h.advance(0);
    expect(reads).toBe(1);
    await h.advance(FULLSCREEN_STATS_INTERVAL_MS - 1);
    expect(reads).toBe(1);
    await h.advance(1);
    expect(reads).toBe(2);
    h.sampler.stop();
  });
  test('a restart right after a stop still waits out the interval', async () => {
    let reads = 0;
    const h = harness(async () => { reads++; return snapshot(); });
    h.sampler.start();
    await h.advance(0);
    h.sampler.stop();
    h.sampler.start();
    await h.advance(500);
    expect(reads).toBe(1);
    await h.advance(500);
    expect(reads).toBe(2);
    h.sampler.stop();
  });
  test('a reply that lands after stop is dropped', async () => {
    let finish: (value: TelemetrySnapshot) => void = () => {};
    const h = harness(() => new Promise<TelemetrySnapshot>(resolve => { finish = resolve; }));
    h.sampler.start();
    await h.advance(0);
    h.sampler.stop();
    finish(snapshot());
    await Promise.resolve();
    expect(h.samples).toEqual([]);
    expect(h.timers.size).toBe(0);
  });
  test('a failed read reports no data and keeps sampling', async () => {
    let reads = 0;
    const h = harness(async () => { reads++; throw new Error('native down'); });
    h.sampler.start();
    await h.advance(0);
    await h.advance(1000);
    expect(reads).toBe(2);
    expect(h.samples).toEqual([null, null]);
    h.sampler.stop();
  });
});
