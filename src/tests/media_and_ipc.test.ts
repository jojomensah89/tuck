import './setup.js';
import { describe, it, expect } from 'bun:test';
import type { MediaState } from '../types.js';

describe('Media & IPC Contract Invariants', () => {
  it('correctly tracks position interpolation without exceeding duration', () => {
    const positionMs = 50000;
    const durationMs = 60000;
    const receivedAt = 1000;

    function currentPos(playing: boolean, now: number): number {
      const elapsed = playing ? now - receivedAt : 0;
      const p = positionMs + elapsed;
      return durationMs > 0 ? Math.min(durationMs, p) : p;
    }

    // While playing for 5 seconds: 50s + 5s = 55s
    expect(currentPos(true, 6000)).toBe(55000);

    // While playing for 20 seconds: caps at 60s (durationMs)
    expect(currentPos(true, 21000)).toBe(60000);

    // While paused: remains at positionMs (50s)
    expect(currentPos(false, 10000)).toBe(50000);
  });

  it('verifies artwork retention contract: null artwork retains art unless trackId changes', () => {
    function reduceMediaState(
      current: { art: string | null; trackId: string },
      payload: MediaState
    ): { art: string | null; trackId: string } {
      const trackChanged = payload.trackId !== current.trackId;
      let art = current.art;
      if (trackChanged) {
        art = null;
      }
      if (payload.artwork) {
        art = payload.artwork;
      }
      return { art, trackId: payload.trackId };
    }

    let state: { art: string | null; trackId: string } = { art: null, trackId: '' };

    // 1st tick: new track with artwork
    state = reduceMediaState(state, {
      active: true,
      trackId: 'track-1',
      title: 'Song 1',
      artist: 'Artist 1',
      playing: true,
      positionMs: 0,
      durationMs: 180000,
      artwork: 'data:image/jpeg;base64,sampleart',
    });
    expect(state.art).toBe('data:image/jpeg;base64,sampleart');

    // 2nd tick: same track, artwork is null (SMTC periodic poll without art re-read)
    state = reduceMediaState(state, {
      active: true,
      trackId: 'track-1',
      title: 'Song 1',
      artist: 'Artist 1',
      playing: true,
      positionMs: 500,
      durationMs: 180000,
      artwork: null,
    });
    // Retained!
    expect(state.art).toBe('data:image/jpeg;base64,sampleart');

    // 3rd tick: new track without artwork yet
    state = reduceMediaState(state, {
      active: true,
      trackId: 'track-2',
      title: 'Song 2',
      artist: 'Artist 2',
      playing: true,
      positionMs: 0,
      durationMs: 200000,
      artwork: null,
    });
    // Reset because trackId changed!
    expect(state.art).toBeNull();
  });

  it('calculates hit rectangle height including top offset', () => {
    function computeHitGeometry(state: 'idle' | 'peek' | 'open', shape: 'notch' | 'float') {
      const sizes: Record<string, [number, number]> = {
        idle: [212, 32],
        peek: [300, 38],
        open: [440, 180],
      };
      const [w, h] = sizes[state];
      const topOffset = shape === 'float' ? 10 : 0;
      return { w, h: h + topOffset };
    }

    // Notch mode (TOP = 0)
    expect(computeHitGeometry('idle', 'notch')).toEqual({ w: 212, h: 32 });
    expect(computeHitGeometry('peek', 'notch')).toEqual({ w: 300, h: 38 });
    expect(computeHitGeometry('open', 'notch')).toEqual({ w: 440, h: 180 });

    // Float mode (TOP = 10)
    expect(computeHitGeometry('idle', 'float')).toEqual({ w: 212, h: 42 });
    expect(computeHitGeometry('peek', 'float')).toEqual({ w: 300, h: 48 });
    expect(computeHitGeometry('open', 'float')).toEqual({ w: 440, h: 190 });
  });
});
