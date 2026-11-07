import './setup.js';
import { describe, it, expect } from 'bun:test';
import {formatUntil, checkResets, mergeMeters, connectedUsageMeters, saveJson} from '../usage.js';
import type {Meter} from '../types.js';
describe('Subscription usage', () => {
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
  it('preserves subscription windows', () => { const m: Meter = {id:'a',provider:'claude',label:'Claude',usedPct:110,spentUsd:null,resetsAt:null,source:'official',stale:false}; expect(mergeMeters([m])[0].usedPct).toBe(100); expect(connectedUsageMeters([{...m,status:'unconfigured'}])).toEqual([]); });
});

it('reports storage failure', () => {
  const original = localStorage.setItem;
  localStorage.setItem = () => { throw new Error('Storage quota'); };
  try { expect(saveJson('test', {})).toBe(false); }
  finally { localStorage.setItem = original; }
});
