import { describe, expect, it } from 'bun:test';
import { displayUsedPct, groupUsageProviders, isOut, meterDetails, ringMeters, usageStateWord, valueText } from '../usage-rings.js';
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

  it('keeps API and manual single-window providers separate from subscriptions', () => {
    const groups = groupUsageProviders([
      meter('claude-5h', 'claude', 'Claude 5 h'),
      { ...meter('anthropic-api', 'anthropic-api', 'Anthropic API'), source: 'api' },
      { ...meter('chatgpt', 'chatgpt', 'ChatGPT estimate'), source: 'manual' },
    ]);
    expect(groups.map(group => group.key)).toEqual(['anthropic-subscription', 'anthropic-api', 'chatgpt']);
    expect(groups.map(group => group.meters.length)).toEqual([1, 1, 1]);
  });
});

describe('windows a card draws', () => {
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
    expect(ringMeters([{ ...meter('api', 'openai-api', 'OpenAI API'), source: 'api' }])).toHaveLength(1);
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
  it('does not present retained Claude values as current remaining quota', () => {
    for (const state of [
      { stale: true, status: 'stale' },
      { stale: true, status: 'unavailable' },
      { stale: false, status: 'unavailable' },
      { stale: true, status: 'error', error: 'Usage unavailable' },
    ]) {
      const old = { ...meter('claude-5h', 'claude', 'Claude 5 h'), usedPct: 20, resetsAt: 1000, ...state };
      expect(displayUsedPct(old)).toBeNull();
      expect(valueText(old)).toBe('\u2013');
      expect(meterDetails(old)).not.toContain('% left');
      expect(meterDetails(old)).not.toContain('resetting now');
      expect(isOut({ ...old, usedPct: 100 })).toBe(false);
    }
  });

  it('shows a current weekly Claude limit even when the session has quota left', () => {
    const session = { ...meter('claude-5h', 'claude', 'Claude 5 h'), usedPct: 20 };
    const week = { ...meter('claude-week', 'claude', 'Claude weekly'), usedPct: 100 };
    expect(valueText(session)).toBe('80% left');
    expect(valueText(week)).toBe('0% left');
    expect(usageStateWord([session, week])).toBe('Limit reached');
    expect(usageStateWord([{ ...week, stale: true, status: 'stale' }])).toBe('Stale');
    expect(usageStateWord([{ ...week, label: 'Claude weekly Opus' }])).toBe('');
  });

  it('preserves the existing Codex retained-reading behavior', () => {
    const codex = { ...meter('codex-primary', 'codex', 'Codex 5 h'), usedPct: 47, stale: true, status: 'error' };
    expect(displayUsedPct(codex)).toBe(47);
    expect(valueText(codex)).toBe('53% left');
  });
  it('names a reading that cannot be trusted and stays quiet otherwise', () => {
    expect(usageStateWord([meter('a', 'claude', 'A')])).toBe('');
    expect(usageStateWord([{ ...meter('a', 'claude', 'A'), stale: true, status: 'stale' }])).toBe('Stale');
    expect(usageStateWord([{ ...meter('a', 'claude', 'A'), usedPct: null, stale: true, status: 'unavailable' }])).toBe('Unavailable');
    expect(usageStateWord([{ ...meter('a', 'codex', 'A'), stale: true, status: 'error', error: 'timed out' }])).toBe('Error');
  });

  it('shows what is left of a quota and keeps spend in dollars', () => {
    expect(valueText({ ...meter('a', 'claude', 'Claude 5 h'), usedPct: 100 })).toBe('0% left');
    expect(valueText({ ...meter('a', 'claude', 'Claude 5 h'), usedPct: 36 })).toBe('64% left');
    expect(valueText({ ...meter('a', 'anthropic-api', 'API'), usedPct: 25, spentUsd: 25 })).toBe('$25.00');
    expect(valueText({ ...meter('a', 'anthropic-api', 'API'), usedPct: null, spentUsd: 12.4 })).toBe('$12.40');
  });

  it('marks a reading as out only when its value says nothing is left', () => {
    expect(isOut({ ...meter('a', 'claude', 'Claude 5 h'), usedPct: 100 })).toBe(true);
    expect(isOut({ ...meter('a', 'claude', 'Claude 5 h'), usedPct: 99 })).toBe(false);
    expect(isOut({ ...meter('chatgpt', 'chatgpt', 'ChatGPT'), count: 40, cap: 40 })).toBe(true);
    expect(isOut({ ...meter('chatgpt', 'chatgpt', 'ChatGPT'), count: 30, cap: 40 })).toBe(false);
    expect(isOut({ ...meter('a', 'anthropic-api', 'API'), usedPct: 100, spentUsd: 25 })).toBe(false);
    expect(isOut({ ...meter('a', 'claude', 'Claude 5 h'), usedPct: null })).toBe(false);
  });

  it('states the window, value and reset in the details', () => {
    const details = meterDetails({ ...meter('a', 'claude', 'Claude 5 h'), usedPct: 42, resetsAt: 1000 + 8130 }, 1000);
    expect(details).toBe('5h, 58% left, resets in 2h 15m, official');
  });
});
