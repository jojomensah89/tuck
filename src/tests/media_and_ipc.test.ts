import './setup.js';
import { describe, it, expect } from 'bun:test';
import { currentPositionMs, retainedArtwork, hitGeometry } from '../runtime.js';
import { dispatchMockEvent, listenToEvent, triggerQuickAction } from '../ipc.js';
import type { MediaState } from '../types.js';
const media: MediaState = { active: true, trackId: 'a', title: 'Track', artist: 'Artist', playing: true, positionMs: 50000, durationMs: 60000 };
describe('Production media and IPC contracts', () => {
  it('interpolates real media positions with duration, pause and no-session bounds', () => {
    expect(currentPositionMs(media, 1000, 6000)).toBe(55000);
    expect(currentPositionMs(media, 1000, 21000)).toBe(60000);
    expect(currentPositionMs({...media, playing:false}, 1000, 6000)).toBe(50000);
    expect(currentPositionMs({...media, active:false}, 1000, 6000)).toBe(0);
  });
  it('retains periodic artwork and rejects non-image data or injected markup', () => {
    const art = 'data:image/png;base64,YQ==';
    expect(retainedArtwork(null, true, art)).toBe(art);
    expect(retainedArtwork(art, false, null)).toBe(art);
    expect(retainedArtwork(art, true, null)).toBeNull();
    expect(retainedArtwork(art, true, "x' onerror='alert(1)")).toBeNull();
  });
  it('reports production hit geometry including float offset', () => {
    expect(hitGeometry(212,32,0)).toEqual({w:212,h:32});
    expect(hitGeometry(440,180,10)).toEqual({w:440,h:190});
  });
  it('dispatches mock events and unsubscribes', async () => {
    const seen: boolean[] = [];
    const unsubscribe = await listenToEvent<boolean>('test-hover', value => seen.push(value));
    dispatchMockEvent('test-hover', true);
    unsubscribe();
    dispatchMockEvent('test-hover', false);
    expect(seen).toEqual([true]);
  });
  it('never navigates the browser webview for native actions', async () => {
    await expect(triggerQuickAction('wifi')).rejects.toThrow();
    await expect(triggerQuickAction('untrusted')).rejects.toThrow();
  });
});
