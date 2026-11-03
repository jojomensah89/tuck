import { describe, expect, test } from 'bun:test';
import {
  canScroll,
  clampTab,
  DEFAULT_TAB,
  GESTURE_GAP_MS,
  INITIAL_WHEEL_STATE,
  isTabId,
  rovingTarget,
  swipeIntent,
  SWITCH_LOCK_MS,
  tabForSection,
  TABS,
  wheelStep,
  type ScrollMetrics,
  type WheelState,
} from '../tab-nav';
import { normalizePreferences } from '../live-state';

const top: ScrollMetrics = { scrollTop: 0, scrollHeight: 356, clientHeight: 178 };
const bottom: ScrollMetrics = { scrollTop: 178, scrollHeight: 356, clientHeight: 178 };
const flat: ScrollMetrics = { scrollTop: 0, scrollHeight: 178, clientHeight: 178 };

describe('tab and section mapping', () => {
  test('every section that existed as a page has exactly one tab', () => {
    const pages = ['media', 'today', 'activities', 'usage', 'quick', 'extensions', 'calendar', 'stats', 'dictation', 'clipboard', 'visualizer'];
    for (const page of pages) expect(tabForSection(page)).not.toBeNull();
    const all = TABS.flatMap(tab => tab.sections);
    expect(new Set(all).size).toBe(all.length);
    expect(all.sort()).toEqual([...pages].sort());
  });
  test('groups follow the roadmap', () => {
    expect(TABS.map(tab => tab.id)).toEqual(['media', 'today', 'tools', 'usage']);
    expect(tabForSection('visualizer')).toBe('media');
    expect(tabForSection('calendar')).toBe('today');
    expect(tabForSection('activities')).toBe('today');
    expect(tabForSection('clipboard')).toBe('tools');
    expect(tabForSection('stats')).toBe('usage');
    expect(tabForSection('missing')).toBeNull();
  });
  test('media is the default and only known ids are accepted', () => {
    expect(DEFAULT_TAB).toBe('media');
    expect(isTabId('tools')).toBe(true);
    expect(isTabId('settings')).toBe(false);
    expect(isTabId(3)).toBe(false);
  });
});

describe('keyboard and clamping', () => {
  test('rail keys wrap and jump', () => {
    expect(rovingTarget('ArrowRight', 3, 4)).toBe(0);
    expect(rovingTarget('ArrowDown', 1, 4)).toBe(2);
    expect(rovingTarget('ArrowLeft', 0, 4)).toBe(3);
    expect(rovingTarget('ArrowUp', 2, 4)).toBe(1);
    expect(rovingTarget('Home', 2, 4)).toBe(0);
    expect(rovingTarget('End', 0, 4)).toBe(3);
    expect(rovingTarget('Enter', 0, 4)).toBeNull();
  });
  test('wheel and swipe stop at the ends', () => {
    expect(clampTab(0, -1, 4)).toBe(0);
    expect(clampTab(3, 1, 4)).toBe(3);
    expect(clampTab(1, 1, 4)).toBe(2);
  });
});

describe('scroll edges', () => {
  test('room in each direction', () => {
    expect(canScroll(top, 1)).toBe(true);
    expect(canScroll(top, -1)).toBe(false);
    expect(canScroll(bottom, 1)).toBe(false);
    expect(canScroll(bottom, -1)).toBe(true);
    expect(canScroll(flat, 1)).toBe(false);
    expect(canScroll({ scrollTop: 177.5, scrollHeight: 356, clientHeight: 178 }, 1)).toBe(false); // fractional end
  });
});

describe('wheel', () => {
  const step = (state: WheelState, deltaY: number, now: number, chain: ScrollMetrics[], deltaX = 0) =>
    wheelStep(state, { deltaX, deltaY, now, chain });

  test('scrolls content that has room before switching', () => {
    const { action } = step(INITIAL_WHEEL_STATE, 100, 1000, [top]);
    expect(action).toEqual({ kind: 'scroll' });
  });
  test('switches when nothing in the chain can scroll', () => {
    const { action, state } = step(INITIAL_WHEEL_STATE, 100, 1000, [bottom]);
    expect(action).toEqual({ kind: 'switch', delta: 1 });
    expect(state.lockedUntil).toBe(1000 + SWITCH_LOCK_MS);
    expect(step(INITIAL_WHEEL_STATE, -40, 1000, [flat]).action).toEqual({ kind: 'switch', delta: -1 });
  });
  test('inner scrollers are checked before outer ones', () => {
    expect(step(INITIAL_WHEEL_STATE, 100, 1000, [bottom, top]).action).toEqual({ kind: 'scroll' });
  });
  test('momentum that reaches the edge does not switch in the same gesture', () => {
    let s = step(INITIAL_WHEEL_STATE, 100, 1000, [top]).state;
    const edge = step(s, 100, 1016, [bottom]);
    expect(edge.action).toEqual({ kind: 'none' });
    s = edge.state;
    // still the same gesture while events keep arriving
    s = step(s, 100, 1100, [bottom]).state;
    // a pause starts a new gesture, which may switch
    expect(step(s, 100, 1100 + GESTURE_GAP_MS + 1, [bottom]).action).toEqual({ kind: 'switch', delta: 1 });
  });
  test('locks after a switch so one gesture moves one tab', () => {
    const first = step(INITIAL_WHEEL_STATE, 100, 1000, []);
    expect(first.action.kind).toBe('switch');
    expect(step(first.state, 100, 1000 + SWITCH_LOCK_MS - 1, [top]).action).toEqual({ kind: 'block' });
    expect(step(first.state, 100, 1000 + SWITCH_LOCK_MS + GESTURE_GAP_MS + 1, []).action.kind).toBe('switch');
  });
  test('horizontal wheel switches even when content could scroll vertically', () => {
    expect(step(INITIAL_WHEEL_STATE, 10, 1000, [top], -80).action).toEqual({ kind: 'switch', delta: -1 });
  });
  test('zero deltas do nothing', () => {
    expect(step(INITIAL_WHEEL_STATE, 0, 1000, []).action).toEqual({ kind: 'none' });
  });
});

describe('swipe', () => {
  test('a long drag left moves forward, right moves back', () => {
    expect(swipeIntent(-80, 4, 600)).toBe(1);
    expect(swipeIntent(80, -4, 600)).toBe(-1);
  });
  test('a short fast flick counts, a short slow drag does not', () => {
    expect(swipeIntent(-30, 0, 50)).toBe(1);
    expect(swipeIntent(-30, 0, 400)).toBe(0);
  });
  test('mostly vertical movement is a scroll, not a swipe', () => {
    expect(swipeIntent(-70, 80, 100)).toBe(0);
  });
});

describe('remembered tab', () => {
  test('lastTab survives normalization and falls back to media', () => {
    expect(normalizePreferences({ lastTab: 'usage' }).lastTab).toBe('usage');
    expect(normalizePreferences({ lastTab: 'settings' }).lastTab).toBe('media');
    expect(normalizePreferences({}).lastTab).toBe('media');
  });
});

describe('wheel momentum', () => {
  const bottom: ScrollMetrics = { scrollTop: 178, scrollHeight: 356, clientHeight: 178 };
  const stream = (deltaX: number, deltaY: number, from: number, to: number, state = INITIAL_WHEEL_STATE) => {
    const actions: string[] = [];
    for (let now = from; now <= to; now += 16) {
      const r = wheelStep(state, { deltaX, deltaY, now, chain: [bottom] });
      state = r.state;
      actions.push(r.action.kind);
    }
    return { actions, state };
  };

  test('moves one tab per continuous vertical gesture, even past the lock', () => {
    const { actions } = stream(0, 40, 1000, 2000);
    expect(actions.filter(a => a === 'switch')).toHaveLength(1);
    expect(actions.filter(a => a === 'scroll')).toHaveLength(0);
  });

  test('moves one tab per continuous horizontal swipe', () => {
    const { actions } = stream(40, 0, 1000, 2000);
    expect(actions.filter(a => a === 'switch')).toHaveLength(1);
  });

  test('lets a new gesture switch again after a pause', () => {
    const first = stream(40, 0, 1000, 1400);
    const second = stream(40, 0, 2000, 2100, first.state);
    expect(second.actions[0]).toBe('switch');
  });
});
