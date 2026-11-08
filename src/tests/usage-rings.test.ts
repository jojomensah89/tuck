import { describe, expect, it } from 'bun:test';
import { groupUsageProviders, isOut, meterDetails, ringMeters, usageStateWord, valueText } from '../usage-rings.js';
import type { Meter } from '../types.js';

const meter = (id: string, provider: string, label: string): Meter => ({
  id, provider, label, usedPct: 10, spentUsd: null, resetsAt: null,
  source: 'official', stale: false, status: 'ok',
});

describe('usage provider groups', () => {
  it('combines 5h and weekly windows into one provider with 5h outside', () => {
    const groups = groupUsageProviders([
      meter('claude-week', 'claude', 'Claude weekly'),
      meter('claude-5h', 'claude', 'Claude session (5 h)'),
      meter('codex-secondary', 'codex', 'Codex weekly'),
      meter('codex-primary', 'codex', 'Codex 5 h'),
    ]);
    expect(groups).toHaveLength(2);
    expect(groups.map(group => group.label)).toEqual(['Claude', 'OpenAI']);
    expect(groups[0].meters.map(item => item.id)).toEqual(['claude-5h', 'claude-week']);
    expect(groups[1].meters.map(item => item.id)).toEqual(['codex-primary', 'codex-secondary']);
  });


});

describe('windows a card draws', () => {
  it('draws Antigravity short and weekly windows and keeps a stable provider name', () => {
    const short = meter('antigravity-short', 'antigravity', 'Antigravity 5 h');
    const week = meter('antigravity-weekly', 'antigravity', 'Antigravity weekly');
    const group = groupUsageProviders([week, short])[0];
    expect(group.label).toBe('Antigravity');
    expect(ringMeters(group.meters).map(m => m.id)).toEqual([short.id, week.id]);
    expect(ringMeters([short])).toEqual([short]);
  });

  it('takes the first 5h and the first weekly, leaving model-scoped windows to the tooltip', () => {
    const drawn = ringMeters([
      meter('claude-5h', 'claude', 'Claude 5 h'),
      meter('claude-week', 'claude', 'Claude weekly'),
      meter('claude-opus', 'claude', 'Claude weekly Opus'),
    ]);
    expect(drawn.map(item => item.id)).toEqual(['claude-5h', 'claude-week']);
  });

  it('never pairs two 5h windows from separate quota buckets', () => {
    const drawn = ringMeters([
      meter('a-primary', 'codex', 'Codex 5 h'),
      meter('b-primary', 'codex', 'Spark 5 h'),
      meter('a-secondary', 'codex', 'Codex weekly'),
    ]);
    expect(drawn.map(item => item.id)).toEqual(['a-primary', 'a-secondary']);
  });

  it('draws one ring for a single-window provider', () => {
    expect(ringMeters([meter('week', 'claude', 'Claude weekly')])).toHaveLength(1);
  });
});

describe('windows from one quota', () => {
  it('prefers the bucket that has both windows over pairing across buckets', () => {
    const drawn = ringMeters([
      meter('codex-installed-a-primary', 'codex', 'Spark 5 h'),
      meter('codex-installed-b-primary', 'codex', 'Codex 5 h'),
      meter('codex-installed-b-secondary', 'codex', 'Codex weekly'),
    ]);
    expect(drawn.map(item => item.id)).toEqual(['codex-installed-b-primary', 'codex-installed-b-secondary']);
  });

  it('never pairs a 5h from one bucket with a weekly from another', () => {
    const drawn = ringMeters([
      meter('codex-installed-a-primary', 'codex', 'Spark 5 h'),
      meter('codex-installed-b-secondary', 'codex', 'Codex weekly'),
    ]);
    expect(drawn.map(item => item.id)).toEqual(['codex-installed-a-primary']);
  });

  it('does not draw a model-scoped window as the general weekly', () => {
    const drawn = ringMeters([
      meter('claude-installed-five_hour', 'claude', 'Claude 5 h'),
      meter('claude-installed-seven_day_opus', 'claude', 'Claude weekly Opus'),
    ]);
    expect(drawn.map(item => item.id)).toEqual(['claude-installed-five_hour']);
    const only = ringMeters([meter('claude-installed-seven_day_opus', 'claude', 'Claude weekly Opus')]);
    expect(only).toHaveLength(1);
    expect(meterDetails(only[0], 0)).toStartWith('Opus,');
  });
});

describe('card state', () => {
  it('names a reading that cannot be trusted and stays quiet otherwise', () => {
    expect(usageStateWord([meter('a', 'claude', 'A')])).toBe('');
    expect(usageStateWord([{ ...meter('a', 'claude', 'A'), stale: true, status: 'stale' }])).toBe('Stale');
    expect(usageStateWord([{ ...meter('a', 'claude', 'A'), usedPct: null, stale: true, status: 'unavailable' }])).toBe('Unavailable');
    expect(usageStateWord([{ ...meter('a', 'codex', 'A'), stale: true, status: 'error', error: 'timed out' }])).toBe('Error');
  });

  it('shows what is left of a quota', () => {
    expect(valueText({ ...meter('a', 'claude', 'Claude 5 h'), usedPct: 100 })).toBe('0% left');
    expect(valueText({ ...meter('a', 'claude', 'Claude 5 h'), usedPct: 36 })).toBe('64% left');
  });

  it('marks a reading as out only when its value says nothing is left', () => {
    expect(isOut({ ...meter('a', 'claude', 'Claude 5 h'), usedPct: 100 })).toBe(true);
    expect(isOut({ ...meter('a', 'claude', 'Claude 5 h'), usedPct: 99 })).toBe(false);
    expect(isOut({ ...meter('a', 'claude', 'Claude 5 h'), usedPct: null })).toBe(false);
  });

  it('states the window, value and reset in the details', () => {
    const details = meterDetails({ ...meter('a', 'claude', 'Claude 5 h'), usedPct: 42, resetsAt: 1000 + 8130 }, 1000);
    expect(details).toBe('5h, 58% left, resets in 2h 15m, official');
  });
});
