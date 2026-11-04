import { describe, expect, it } from 'bun:test';
import { groupUsageProviders, meterDetails, ringMeters, usageStateWord } from '../usage-rings.js';
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

describe('card state', () => {
  it('names a reading that cannot be trusted and stays quiet otherwise', () => {
    expect(usageStateWord([meter('a', 'claude', 'A')])).toBe('');
    expect(usageStateWord([{ ...meter('a', 'claude', 'A'), stale: true, status: 'stale' }])).toBe('Stale');
    expect(usageStateWord([{ ...meter('a', 'claude', 'A'), usedPct: null, stale: true, status: 'unavailable' }])).toBe('Unavailable');
    expect(usageStateWord([{ ...meter('a', 'codex', 'A'), stale: true, status: 'error', error: 'timed out' }])).toBe('Error');
  });

  it('states the window, value and reset in the details', () => {
    const details = meterDetails({ ...meter('a', 'claude', 'Claude 5 h'), usedPct: 42, resetsAt: 1000 + 8130 }, 1000);
    expect(details).toBe('5h, 42% used, resets in 2h 15m, official');
  });
});
