import './setup.js';
import { describe, it, expect } from 'bun:test';
import {
  activityProgress,
  batteryTone,
  clampPct,
  formatSpan,
  meterWindow,
  providerInitial,
  providerMark,
  ringDashOffset,
  remainingPct,
  ringGeometry,
  usageTone,
} from '../rings.js';
import { resetSpan } from '../usage.js';
import type { Activity } from '../types.js';

describe('ring geometry', () => {
  it('keeps the stroke inside the square', () => {
    const g = ringGeometry(56, 6);
    expect(g.r).toBe(25);
    expect(g.center).toBe(28);
    expect(g.c).toBeCloseTo(2 * Math.PI * 25, 6);
    expect(ringGeometry(4, 6).r).toBe(0);
  });

  it('draws the share of the circumference that is used', () => {
    const { c } = ringGeometry(56, 6);
    expect(ringDashOffset(0, c)).toBeCloseTo(c);
    expect(ringDashOffset(100, c)).toBeCloseTo(0);
    expect(ringDashOffset(25, c)).toBeCloseTo(c * 0.75);
    // Out of range and missing values clamp rather than overdraw
    expect(ringDashOffset(140, c)).toBeCloseTo(0);
    expect(ringDashOffset(-5, c)).toBeCloseTo(c);
    expect(ringDashOffset(null, c)).toBeCloseTo(c);
    expect(ringDashOffset(NaN, c)).toBeCloseTo(c);
  });

  it('clamps percentages and rejects non-numbers', () => {
    expect(clampPct(50)).toBe(50);
    expect(clampPct(120)).toBe(100);
    expect(clampPct(-1)).toBe(0);
    expect(clampPct(null)).toBeNull();
    expect(clampPct(undefined)).toBeNull();
    expect(clampPct(Infinity)).toBeNull();
  });
});

describe('remaining quota', () => {
  it('turns usage into what is left', () => {
    expect(remainingPct(42)).toBe(58);
    expect(remainingPct(0)).toBe(100);
    expect(remainingPct(120)).toBe(0);
    expect(remainingPct(99.6)).toBe(0);
  });
  it('keeps unknown usage unknown', () => {
    expect(remainingPct(null)).toBeNull();
    expect(remainingPct(undefined)).toBeNull();
    expect(remainingPct(Number.NaN)).toBeNull();
  });
});

describe('threshold tones', () => {
  it('turns a quota amber at 70% and red at 90%', () => {
    expect(usageTone(0)).toBe('ok');
    expect(usageTone(69.9)).toBe('ok');
    expect(usageTone(70)).toBe('caution');
    expect(usageTone(89.9)).toBe('caution');
    expect(usageTone(90)).toBe('warn');
    expect(usageTone(150)).toBe('warn');
    expect(usageTone(null)).toBe('none');
  });

  it('shows charging in green and a low battery in red', () => {
    expect(batteryTone(50, false)).toBe('ink');
    expect(batteryTone(19, false)).toBe('warn');
    expect(batteryTone(20, false)).toBe('ink');
    expect(batteryTone(10, true)).toBe('ok');
    expect(batteryTone(NaN, false)).toBe('none');
  });
});

describe('one duration format', () => {
  it('formats spans as the two largest units', () => {
    expect(formatSpan(0)).toBe('now');
    expect(formatSpan(-5)).toBe('now');
    expect(formatSpan(30)).toBe('<1m');
    expect(formatSpan(45 * 60)).toBe('45m');
    expect(formatSpan(2 * 3600)).toBe('2h');
    expect(formatSpan(2 * 3600 + 15 * 60)).toBe('2h 15m');
    expect(formatSpan(3 * 86400)).toBe('3d');
    expect(formatSpan(3 * 86400 + 4 * 3600 + 59)).toBe('3d 4h');
  });

  it('rounds a pending reset up to a minute', () => {
    const now = 1_000_000;
    expect(resetSpan(null, now)).toBe('');
    expect(resetSpan(now - 1, now)).toBe('now');
    expect(resetSpan(now + 20, now)).toBe('1m');
    expect(resetSpan(now + 8100, now)).toBe('2h 15m');
  });
});

describe('provider identity', () => {
  it('maps providers to a mark family, or an initial', () => {
    expect(providerMark('claude')).toBe('claude');
    expect(providerMark('codex')).toBe('openai');
    expect(providerMark('gemini')).toBeNull();
    expect(providerInitial({ provider: 'gemini', label: 'gemini pro' })).toBe('G');
    expect(providerInitial({ provider: 'x', label: '  ' })).toBe('X');
  });

  it('names the window so two rings with one mark stay apart', () => {
    expect(meterWindow({ id: 'claude-5h', label: 'Claude session (5 h)', source: 'official' })).toBe('5h');
    expect(meterWindow({ id: 'claude-7d', label: 'Claude weekly', source: 'official' })).toBe('7d');
    expect(meterWindow({ id: 'codex-installed-x-primary', label: 'Codex 5 h', source: 'official' })).toBe('5h');
    expect(meterWindow({ id: 'codex-installed-x-secondary', label: 'Codex weekly', source: 'official' })).toBe('7d');
    expect(meterWindow({ id: 'codex-installed-x-primary', label: 'Codex 30 min', source: 'official' })).toBe('30m');
    expect(meterWindow({ id: 'codex-5h', label: 'Codex 5-hour window', source: 'official' })).toBe('5h');
    expect(meterWindow({ id: 'claude-week', label: 'Claude weekly', source: 'official' })).toBe('7d');
    expect(meterWindow({ id: 'opus', label: 'Claude weekly Opus', source: 'official' })).toBe('Opus');
    expect(meterWindow({ id: 'claude-installed', label: 'Claude Code', source: 'official' })).toBe('');
  });
});

describe('timer ring progress', () => {
  const now = 1_000_000;
  const timer = (over: Partial<Activity>): Activity => ({
    id: 't', type: 'timer', title: 'Timer', priority: 80, active: true, paused: false, ...over,
  });

  it('drains a countdown from full to empty', () => {
    expect(activityProgress(timer({ totalDurationMs: 60000, targetEndTime: now + 60000 }), now)).toBe(1);
    expect(activityProgress(timer({ totalDurationMs: 60000, targetEndTime: now + 15000 }), now)).toBe(0.25);
    expect(activityProgress(timer({ totalDurationMs: 60000, targetEndTime: now - 5 }), now)).toBe(0);
    expect(activityProgress(timer({ totalDurationMs: 60000, paused: true, remainingMs: 30000 }), now)).toBe(0.5);
  });

  it('sweeps a stopwatch once a minute', () => {
    const sw = (over: Partial<Activity>) => timer({ type: 'stopwatch', ...over });
    expect(activityProgress(sw({ startTime: now - 15000 }), now)).toBe(0.25);
    expect(activityProgress(sw({ startTime: now - 75000 }), now)).toBe(0.25);
    expect(activityProgress(sw({ paused: true, accumulatedMs: 30000 }), now)).toBe(0.5);
  });

  it('draws nothing when idle', () => {
    expect(activityProgress(null, now)).toBeNull();
    expect(activityProgress(timer({ active: false }), now)).toBeNull();
    expect(activityProgress(timer({ totalDurationMs: 0 }), now)).toBeNull();
  });
});

describe('ring wrap', () => {
  it('the stopwatch wraps to zero each minute', () => {
    const base = { id: 's', type: 'stopwatch', title: 'Stopwatch', active: true, paused: false, startTime: 1_000 } as unknown as Parameters<typeof activityProgress>[0];
    expect(activityProgress(base, 60_900)!).toBeGreaterThan(0.99);
    expect(activityProgress(base, 61_100)!).toBeLessThan(0.01);
  });
});
