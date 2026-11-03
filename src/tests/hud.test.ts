import './setup.js';
import { describe, expect, it } from 'bun:test';
import {
  AnnouncementQueue,
  CHARGER_RAMP_MS,
  HUD_ANNOUNCE_QUIET_MS,
  HUD_DISMISS_MS,
  HudModel,
  hudLabel,
  levelEventShowsHud,
  levelInputApplies,
  overPillStrip,
  peekReturnTarget,
  readHudPayload,
  stepVolume,
  volumeGlyph,
  wheelVolumeDelta,
} from '../hud.js';

const wheel = (deltaY: number, extra: Partial<{ deltaX: number; deltaMode: number; shiftKey: boolean }> = {}) => ({
  deltaX: 0, deltaMode: 0, shiftKey: false, deltaY, ...extra,
});

describe('peekReturnTarget', () => {
  const context = { returnOpen: false, hover: false, pinned: false, dragging: false };

  it('goes back to an open island the user is still using', () => {
    expect(peekReturnTarget({ ...context, returnOpen: true, pinned: true })).toBe('open');
    expect(peekReturnTarget({ ...context, returnOpen: true, hover: true })).toBe('open');
  });

  it('collapses an open island nobody is using', () => {
    expect(peekReturnTarget({ ...context, returnOpen: true })).toBe('idle');
  });

  it('never opens an island that was collapsed, even when pinned or hovered', () => {
    expect(peekReturnTarget({ ...context, hover: true })).toBe('peek');
    expect(peekReturnTarget({ ...context, pinned: true })).toBe('idle');
    expect(peekReturnTarget({ ...context, dragging: true })).toBe('peek');
    expect(peekReturnTarget(context)).toBe('idle');
  });
});

describe('pill strip hit test', () => {
  const pill = { width: 212, height: 32 };
  const open = { left: 10, top: 0, width: 500 };

  it('covers the idle pill size at the top centre of the open island', () => {
    expect(overPillStrip(open, pill, 260, 16)).toBe(true);
    expect(overPillStrip(open, pill, 154, 0)).toBe(true);
    expect(overPillStrip(open, pill, 365.5, 31.5)).toBe(true);
  });

  it('stops at the edges of the strip', () => {
    expect(overPillStrip(open, pill, 153.5, 16)).toBe(false);
    expect(overPillStrip(open, pill, 366, 16)).toBe(false);
    expect(overPillStrip(open, pill, 260, 32)).toBe(false);
    expect(overPillStrip(open, pill, 260, -1)).toBe(false);
  });

  it('follows the island down in the floating shape and stays centred at any width', () => {
    const float = { left: 40, top: 10, width: 440 };
    expect(overPillStrip(float, pill, 260, 5)).toBe(false);
    expect(overPillStrip(float, pill, 260, 10)).toBe(true);
    expect(overPillStrip(float, pill, 260, 41)).toBe(true);
    expect(overPillStrip(float, pill, 260, 42)).toBe(false);
    expect(overPillStrip(float, pill, 154, 20)).toBe(true);
  });

  it('means volume on the whole collapsed island and only the strip when open', () => {
    const peek = { left: 110, top: 0, width: 300 };
    expect(levelInputApplies('idle', peek, pill, 115, 30)).toBe(true);
    expect(levelInputApplies('peek', peek, pill, 115, 37)).toBe(true);
    expect(levelInputApplies('open', open, pill, 260, 16)).toBe(true);
    expect(levelInputApplies('open', open, pill, 260, 120)).toBe(false);
    expect(levelInputApplies('open', open, pill, 30, 16)).toBe(false);
  });
});

describe('levelEventShowsHud', () => {
  const volume = { kind: 'volume', level: 40, muted: false } as const;
  const brightness = { kind: 'brightness', level: 40, muted: false } as const;

  it('follows the setting', () => {
    expect(levelEventShowsHud(volume, true, 1000, null, false)).toBe(true);
    expect(levelEventShowsHud(volume, false, 1000, null, false)).toBe(false);
    expect(levelEventShowsHud(brightness, false, 1000, null, false)).toBe(false);
  });

  it('ignores brightness ramps right after a charger change, but not volume', () => {
    expect(levelEventShowsHud(brightness, true, 5000, 1000, false)).toBe(false);
    expect(levelEventShowsHud(brightness, true, 1000 + CHARGER_RAMP_MS, 1000, false)).toBe(true);
    expect(levelEventShowsHud(volume, true, 1001, 1000, false)).toBe(true);
  });

  it('drops the charger quiet period while the Windows flyout is hidden, but not the setting', () => {
    expect(levelEventShowsHud(brightness, true, 5000, 1000, true)).toBe(true);
    expect(levelEventShowsHud(brightness, false, 5000, 1000, true)).toBe(false);
  });
});

describe('AnnouncementQueue', () => {
  it('releases text oldest first, once each', () => {
    const queue = new AnnouncementQueue();
    queue.add('first', 0);
    queue.add('second', 0);
    queue.add('first', 0);
    expect(queue.next(10)).toBe('first');
    expect(queue.next(10)).toBe('second');
    expect(queue.next(10)).toBeNull();
  });

  it('puts an interrupted announcement ahead of what arrived later', () => {
    const queue = new AnnouncementQueue();
    queue.add('failure', 0);
    queue.addFirst('caps lock', 0);
    expect(queue.next(10)).toBe('caps lock');
    expect(queue.next(10)).toBe('failure');
  });

  it('drops stale text and stays bounded', () => {
    const queue = new AnnouncementQueue();
    queue.add('old', 0);
    expect(queue.next(60000)).toBeNull();
    for (const text of ['a', 'b', 'c', 'd']) queue.add(text, 0);
    expect(queue.next(10)).toBe('b');
  });
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
