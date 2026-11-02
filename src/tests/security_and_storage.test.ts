import './setup.js';
import { describe, it, expect, beforeEach } from 'bun:test';
import { setSecret } from '../ipc.js';
import { STORAGE_KEYS, saveJson, getJson } from '../usage.js';

describe('Security & Persistence Invariants', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('never stores admin API keys or secrets in localStorage', async () => {
    await setSecret('anthropic-admin', 'sk-ant-admin-test-placeholder');
    // Save sample budgets and chatgpt configs
    saveJson(STORAGE_KEYS.BUDGETS, { 'anthropic-api': 25 });
    saveJson(STORAGE_KEYS.CHATGPT, { cap: 40, hours: 3, label: 'Chat' });
    localStorage.setItem(STORAGE_KEYS.SHAPE, 'notch');

    // Inspect all keys and values in localStorage
    const keys = Array.from({length:localStorage.length}, (_,i) => localStorage.key(i)!);
    for (const key of keys) {
      expect(key.toLowerCase()).not.toContain('secret');
      expect(key.toLowerCase()).not.toContain('key');
      expect(key.toLowerCase()).not.toContain('token');
      expect(key.toLowerCase()).not.toContain('password');

      const value = localStorage.getItem(key) || '';
      expect(value).not.toContain('sk-ant-admin');
      expect(value).not.toContain('sk-admin');
    }
  });

  it('persists and retrieves shape style accurately', () => {
    expect(localStorage.getItem(STORAGE_KEYS.SHAPE)).toBeNull();

    localStorage.setItem(STORAGE_KEYS.SHAPE, 'float');
    expect(localStorage.getItem(STORAGE_KEYS.SHAPE)).toBe('float');

    localStorage.setItem(STORAGE_KEYS.SHAPE, 'notch');
    expect(localStorage.getItem(STORAGE_KEYS.SHAPE)).toBe('notch');
  });

  it('handles JSON parse failures gracefully without throwing', () => {
    localStorage.setItem('bad-json', '{ invalid json');
    const result = getJson('bad-json', { fallback: true });
    expect(result).toEqual({ fallback: true });
  });
});
