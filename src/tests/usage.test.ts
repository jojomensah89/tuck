import './setup.js';
import { describe, it, expect, beforeEach } from 'bun:test';
import {
  getChatGPTMeter,
  logChatGPTMessage,
  formatUntil,
  mergeMeters,
  checkResets,
  getNextMonthStartUtc,
  STORAGE_KEYS,
  saveJson,
} from '../usage.js';
import type { Meter } from '../types.js';

describe('Usage Tracker & ChatGPT Rolling Window', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('returns null if ChatGPT config is missing or incomplete', () => {
    expect(getChatGPTMeter()).toBeNull();

    saveJson(STORAGE_KEYS.CHATGPT, { cap: 0, hours: 3, label: 'Test' });
    expect(getChatGPTMeter()).toBeNull();

    saveJson(STORAGE_KEYS.CHATGPT, { cap: 80, hours: 0, label: 'Test' });
    expect(getChatGPTMeter()).toBeNull();
  });

  it('calculates rolling window count and percentage correctly', () => {
    const now = 1000000;
    saveJson(STORAGE_KEYS.CHATGPT, { cap: 40, hours: 3, label: 'ChatGPT Plus' });

    // 3 hours = 10800 seconds. Window cutoff = 1000000 - 10800 = 989200
    // Save 3 timestamps: 1 expired, 2 active
    const timestamps = [
      989000, // expired (> 3h ago)
      990000, // active (10000 sec ago, oldest active)
      995000, // active (5000 sec ago)
    ];
    saveJson(STORAGE_KEYS.CHATGPT_LOG, timestamps);

    const meter = getChatGPTMeter(now);
    expect(meter).not.toBeNull();
    expect(meter?.count).toBe(2);
    expect(meter?.cap).toBe(40);
    expect(meter?.usedPct).toBe((2 / 40) * 100); // 5%
    // Oldest active is 990000; slot frees at 990000 + 10800 = 1000800
    expect(meter?.resetsAt).toBe(1000800);
  });

  it('logs a new message and prunes stale history', () => {
    const now = 1000000;
    saveJson(STORAGE_KEYS.CHATGPT, { cap: 80, hours: 3, label: 'ChatGPT' });

    // Pre-populate with an entry older than 48 hours (48 * 3600 = 172800)
    saveJson(STORAGE_KEYS.CHATGPT_LOG, [now - 200000, now - 1000]);

    logChatGPTMessage(now);

    const raw = localStorage.getItem(STORAGE_KEYS.CHATGPT_LOG);
    const parsed: number[] = raw ? JSON.parse(raw) : [];

    // The entry older than 48h was pruned
    expect(parsed).toContain(now);
    expect(parsed).toContain(now - 1000);
    expect(parsed).not.toContain(now - 200000);
    expect(parsed.length).toBe(2);
  });

  it('merges native meters with user budgets and calculates percentages', () => {
    saveJson(STORAGE_KEYS.BUDGETS, {
      'anthropic-api': 50,
      'openai-api': 100,
    });

    const nativeMeters: Meter[] = [
      {
        id: 'claude-5h',
        provider: 'claude',
        label: 'Claude session (5 h)',
        usedPct: 75,
        spentUsd: null,
        resetsAt: 1234567,
        source: 'official',
        stale: false,
      },
      {
        id: 'anthropic-api',
        provider: 'anthropic-api',
        label: 'Claude API',
        usedPct: null,
        spentUsd: 25.0, // 25 out of 50 budget = 50%
        resetsAt: null,
        source: 'api',
        stale: false,
      },
      {
        id: 'openai-api',
        provider: 'openai-api',
        label: 'OpenAI API',
        usedPct: null,
        spentUsd: 10.0, // 10 out of 100 budget = 10%
        resetsAt: null,
        source: 'api',
        stale: false,
      },
    ];

    const merged = mergeMeters(nativeMeters);
    expect(merged.length).toBe(3);

    const ant = merged.find((m) => m.id === 'anthropic-api');
    expect(ant?.usedPct).toBe(50);
    expect(ant?.resetsAt).toBe(getNextMonthStartUtc());

    const oai = merged.find((m) => m.id === 'openai-api');
    expect(oai?.usedPct).toBe(10);
  });

  it('detects quota resets when usage drops by > 30%', () => {
    const prevPcts = {
      'claude-5h': 85,
      'claude-week': 40,
    };

    const newMeters: Meter[] = [
      {
        id: 'claude-5h',
        provider: 'claude',
        label: 'Claude session (5 h)',
        usedPct: 10, // Dropped 75% -> triggers reset
        spentUsd: null,
        resetsAt: null,
        source: 'official',
        stale: false,
      },
      {
        id: 'claude-week',
        provider: 'claude',
        label: 'Claude weekly',
        usedPct: 35, // Dropped only 5% -> no reset
        spentUsd: null,
        resetsAt: null,
        source: 'official',
        stale: false,
      },
    ];

    const triggeredResets: string[] = [];
    const updated = checkResets(newMeters, prevPcts, (m) => {
      triggeredResets.push(m.id);
    });

    expect(triggeredResets).toEqual(['claude-5h']);
    expect(updated['claude-5h']).toBe(10);
    expect(updated['claude-week']).toBe(35);
  });

  it('formats countdowns accurately', () => {
    const now = 1000000;
    expect(formatUntil(null, now)).toBe('');
    expect(formatUntil(now - 10, now)).toBe('resetting now');
    expect(formatUntil(now + 45, now)).toBe('resets in 1m');
    expect(formatUntil(now + 3660, now)).toBe('resets in 1h 1m');
    expect(formatUntil(now + 90000, now)).toBe('resets in 1d 1h');
  });
});


describe('Storage recovery and long rolling windows', () => {
  beforeEach(() => localStorage.clear());
  it('ignores corrupt log shapes and future timestamps', () => {
    saveJson(STORAGE_KEYS.CHATGPT, {cap:80,hours:3,label:'Test'});
    saveJson(STORAGE_KEYS.CHATGPT_LOG, {});
    expect(getChatGPTMeter(1000000)?.count).toBe(0);
    logChatGPTMessage(1000000);
    saveJson(STORAGE_KEYS.CHATGPT_LOG, [1000001, null, 'bad', 999999]);
    expect(getChatGPTMeter(1000000)?.count).toBe(1);
  });
  it('keeps valid logs in a seven day configured window', () => {
    saveJson(STORAGE_KEYS.CHATGPT, {cap:80,hours:168,label:'Test'});
    saveJson(STORAGE_KEYS.CHATGPT_LOG, [1000000 - 3 * 86400]);
    logChatGPTMessage(1000000);
    expect(getChatGPTMeter(1000000)?.count).toBe(2);
  });
});


it('reports persistence failure instead of claiming a successful save', () => {
  const original = localStorage.setItem;
  localStorage.setItem = () => { throw new Error('Storage quota'); };
  try { expect(saveJson('test', {})).toBe(false); }
  finally { localStorage.setItem = original; }
});
