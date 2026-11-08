import './setup.js';
import { describe, expect, it } from 'bun:test';
import { DictationUpdates, normalizeDictation } from '../live-services.js';

describe('dictation event ordering', () => {
  it('rejects late initial responses, old sessions, and duplicate events', () => {
    const updates = new DictationUpdates();
    const snapshot = (revision: number, sessionId = 1) => normalizeDictation({ revision, sessionId, status: 'recording' });
    expect(updates.accept(snapshot(3))).toBe(true);
    expect(updates.accept(snapshot(1))).toBe(false);
    expect(updates.accept(snapshot(3))).toBe(false);
    expect(updates.accept(snapshot(5, 2))).toBe(true);
    expect(updates.accept(snapshot(4))).toBe(false);
    expect(updates.accept(normalizeDictation({status: 'idle'}))).toBe(false);
  });

  it('keeps recording text empty and reserves transcript for the final result', () => {
    const data = normalizeDictation({status: 'recording', revision: 4, transcript: null});
    expect(data.transcript).toBe('');
  });
});
