import { test, expect } from 'bun:test';
import { claimUpdateVersion } from './update-alert';
test('claims each version once, including after restarting the caller', () => {
  const values = new Map<string, string>();
  const storage = { getItem: (k: string) => values.get(k) ?? null, setItem: (k: string, v: string) => { values.set(k, v); } };
  expect(claimUpdateVersion(storage, '0.2.2')).toBe(true);
  expect(claimUpdateVersion({ ...storage }, '0.2.2')).toBe(false);
  expect(claimUpdateVersion(storage, '0.2.3')).toBe(true);
});
test('unavailable persistence stays quiet', () => {
  expect(claimUpdateVersion({ getItem() { throw Error(); }, setItem() {} }, '0.2.2')).toBe(false);
});
