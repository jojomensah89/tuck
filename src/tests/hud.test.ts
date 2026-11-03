import './setup.js';
import { describe, expect, it } from 'bun:test';
import {
  HUD_ANNOUNCE_QUIET_MS,
  HUD_DISMISS_MS,
  HudModel,
  hudLabel,
  readHudPayload,
  stepVolume,
  volumeGlyph,
  wheelVolumeDelta,
} from '../hud.js';

const wheel = (deltaY: number, extra: Partial<{ deltaX: number; deltaMode: number; shiftKey: boolean }> = {}) => ({
  deltaX: 0, deltaMode: 0, shiftKey: false, deltaY, ...extra,
});

describe('stepVolume', () => {
  it('snaps off-grid levels to the step grid', () => {
    expect(stepVolume(36, 2)).toBe(38);
    expect(stepVolume(37, 2)).toBe(38);
    expect(stepVolume(37, -2)).toBe(36);
    expect(stepVolume(36, -2)).toBe(34);
    expect(stepVolume(37, 10)).toBe(40);
    expect(stepVolume(37, -10)).toBe(30);
  });

  it('clamps at both ends', () => {
    expect(stepVolume(99, 2)).toBe(100);
    expect(stepVolume(100, 10)).toBe(100);
    expect(stepVolume(1, -2)).toBe(0);
    expect(stepVolume(0, -2)).toBe(0);
    expect(stepVolume(50, 0)).toBe(50);
  });
});

describe('wheelVolumeDelta', () => {
  it('turns one notch up into +2 and down into -2', () => {
    expect(wheelVolumeDelta(0, wheel(-100))).toEqual({ delta: 2, rest: 0 });
    expect(wheelVolumeDelta(0, wheel(100))).toEqual({ delta: -2, rest: 0 });
  });

  it('uses the large step with Shift, which Windows reports as horizontal', () => {
    expect(wheelVolumeDelta(0, wheel(0, { deltaX: -100, shiftKey: true })).delta).toBe(10);
    expect(wheelVolumeDelta(0, wheel(100, { shiftKey: true })).delta).toBe(-10);
  });

  it('ignores a plain horizontal tilt', () => {
    expect(wheelVolumeDelta(0, wheel(0, { deltaX: 120 })).delta).toBe(0);
  });

  it('accumulates small trackpad deltas into a step', () => {
    let rest = 0;
    const deltas: number[] = [];
    for (let i = 0; i < 5; i++) {
      const result = wheelVolumeDelta(rest, wheel(-12));
      rest = result.rest;
      deltas.push(result.delta);
    }
    expect(deltas).toEqual([0, 0, 0, 0, 2]);
  });

  it('reads line and page deltas and caps a fast flick', () => {
    expect(wheelVolumeDelta(0, wheel(-3, { deltaMode: 1 })).delta).toBe(2);
    expect(wheelVolumeDelta(0, wheel(-2, { deltaMode: 2 })).delta).toBe(10);
  });

  it('carries less than half a notch between events', () => {
    const { rest } = wheelVolumeDelta(0, wheel(-125));
    expect(Math.abs(rest)).toBeLessThanOrEqual(50);
  });
});

describe('readHudPayload', () => {
  it('accepts volume and brightness and clamps the level', () => {
    expect(readHudPayload({ kind: 'volume', level: 40, muted: true })).toEqual({ kind: 'volume', level: 40, muted: true });
    expect(readHudPayload({ kind: 'brightness', level: 140, muted: true })).toEqual({ kind: 'brightness', level: 100, muted: false });
  });

  it('drops malformed payloads', () => {
    expect(readHudPayload(null)).toBeNull();
    expect(readHudPayload({ kind: 'bass', level: 4 })).toBeNull();
    expect(readHudPayload({ kind: 'volume', level: Number.NaN })).toBeNull();
  });
});

describe('glyph and label', () => {
  it('picks the speaker glyph from level and mute', () => {
    expect(volumeGlyph(40, true)).toBe('volumeMute');
    expect(volumeGlyph(0, false)).toBe('volumeOff');
    expect(volumeGlyph(20, false)).toBe('volumeLow');
    expect(volumeGlyph(80, false)).toBe('volumeHigh');
  });

  it('names the value', () => {
    expect(hudLabel({ kind: 'volume', level: 40, muted: false })).toBe('Volume 40%');
    expect(hudLabel({ kind: 'volume', level: 40, muted: true })).toBe('Volume muted, 40%');
    expect(hudLabel({ kind: 'brightness', level: 70, muted: false })).toBe('Brightness 70%');
  });
});

describe('HudModel', () => {
  const volume = (level: number) => ({ kind: 'volume' as const, level, muted: false });

  it('stays up until the dismiss delay after the last change', () => {
    const hud = new HudModel();
    expect(hud.visible(0)).toBe(false);
    hud.show(volume(40), 0);
    hud.show(volume(42), 1000);
    expect(hud.visible(1000 + HUD_DISMISS_MS - 1)).toBe(true);
    expect(hud.visible(1000 + HUD_DISMISS_MS)).toBe(false);
    expect(hud.remaining(1000)).toBe(HUD_DISMISS_MS);
  });

  it('announces once the input settles, without repeating a value', () => {
    const hud = new HudModel();
    hud.show(volume(40), 0);
    hud.show(volume(42), 200);
    expect(hud.dueAnnouncement(200 + HUD_ANNOUNCE_QUIET_MS - 1)).toBeNull();
    expect(hud.dueAnnouncement(200 + HUD_ANNOUNCE_QUIET_MS)).toBe('Volume 42%');
    expect(hud.dueAnnouncement(2000)).toBeNull();
    hud.show(volume(42), 2100);
    expect(hud.dueAnnouncement(2100 + HUD_ANNOUNCE_QUIET_MS)).toBeNull();
    hud.dismiss();
    hud.show(volume(42), 5000);
    expect(hud.dueAnnouncement(5000 + HUD_ANNOUNCE_QUIET_MS)).toBe('Volume 42%');
  });
});
