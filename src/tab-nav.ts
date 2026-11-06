/**
 * Pure navigation logic for the open island's tab rail: which sections live in
 * which tab, keyboard roving, and how wheel and swipe input turn into tab
 * changes. No DOM access, so it is unit tested directly.
 */

export type TabId = 'media' | 'today' | 'tools' | 'usage';
export type TabIcon = 'note' | 'calendar' | 'grid' | 'gauge';

export interface TabDef {
  id: TabId;
  label: string;
  icon: TabIcon;
  /** `data-page` values of the sections, top to bottom */
  sections: readonly string[];
}

export const TABS: readonly TabDef[] = [
  { id: 'media', label: 'Media', icon: 'note', sections: ['media'] },
  { id: 'today', label: 'Today', icon: 'calendar', sections: ['today', 'activities'] },
  { id: 'tools', label: 'Tools', icon: 'grid', sections: ['quick', 'clipboard', 'extensions', 'dictation'] },
  { id: 'usage', label: 'Usage', icon: 'gauge', sections: ['usage', 'stats'] },
];

export const DEFAULT_TAB: TabId = 'media';

/** What each section is called on its indicator mark ("Go to AI usage") */
const SECTION_LABELS: Readonly<Record<string, string>> = {
  media: 'now playing',
  today: 'clock and battery',
  activities: 'timer',
  quick: 'quick actions',
  clipboard: 'clipboard',
  extensions: 'files and devices',
  dictation: 'dictation',
  usage: 'AI usage',
  stats: 'system',
};

export function sectionLabel(section: string): string {
  return SECTION_LABELS[section] ?? section;
}

/** Lock after a wheel or swipe tab change, so one gesture moves one tab */
export const SWITCH_LOCK_MS = 350;
/** Wheel events closer together than this belong to the same gesture */
export const GESTURE_GAP_MS = 180;

export function isTabId(value: unknown): value is TabId {
  return TABS.some(tab => tab.id === value);
}

export function tabIndex(id: TabId): number {
  return TABS.findIndex(tab => tab.id === id);
}

export function tabForSection(section: string): TabId | null {
  return TABS.find(tab => tab.sections.includes(section))?.id ?? null;
}

/** Wheel and swipe stop at the first and last tab instead of looping */
export function clampTab(index: number, delta: number, count = TABS.length): number {
  return Math.max(0, Math.min(count - 1, index + delta));
}

/** Roving tabindex on the rail; arrows wrap, Home and End jump. Null for other keys */
export function rovingTarget(key: string, index: number, count = TABS.length): number | null {
  switch (key) {
    case 'ArrowRight':
    case 'ArrowDown':
      return (index + 1) % count;
    case 'ArrowLeft':
    case 'ArrowUp':
      return (index - 1 + count) % count;
    case 'Home':
      return 0;
    case 'End':
      return count - 1;
    default:
      return null;
  }
}

export interface ScrollMetrics {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

/** Whether a scroller has room left in the wheel direction (1px tolerance for fractional scroll) */
export function canScroll(m: ScrollMetrics, delta: number): boolean {
  if (delta > 0) return m.scrollTop + m.clientHeight < m.scrollHeight - 1;
  if (delta < 0) return m.scrollTop > 1;
  return false;
}

/** Active section for a snap panel, clamped when sections appear or disappear. */
export function sectionIndexAt(scrollTop: number, clientHeight: number, count: number): number {
  if (count <= 0 || clientHeight <= 0) return 0;
  return Math.max(0, Math.min(count - 1, Math.round(scrollTop / clientHeight)));
}

export interface WheelState {
  lockedUntil: number;
  lastWheelAt: number;
  /** The current gesture already scrolled content, so it may not also change tabs */
  gestureScrolled: boolean;
  /** The current gesture already changed tabs; the rest of it (trackpad momentum) is swallowed */
  gestureSwitched: boolean;
}

export const INITIAL_WHEEL_STATE: WheelState = {
  lockedUntil: 0, lastWheelAt: -Infinity, gestureScrolled: false, gestureSwitched: false,
};

export interface WheelInput {
  deltaX: number;
  deltaY: number;
  now: number;
  /** Scrollers between the event target and the tab panel, innermost first */
  chain: readonly ScrollMetrics[];
}

/**
 * - `scroll`: let the browser scroll the content
 * - `block`: swallow the event (just switched; keeps momentum out of the new tab)
 * - `switch`: change tab by `delta`
 * - `none`: ignore
 */
export type WheelAction =
  | { kind: 'scroll' }
  | { kind: 'block' }
  | { kind: 'switch'; delta: 1 | -1 }
  | { kind: 'none' };

/**
 * Edge-aware wheel handling. Vertical wheel scrolls the innermost scroller that
 * still has room; only a fresh gesture that starts at the edge switches tabs.
 * Horizontal wheel (trackpad swipe, wheel tilt) always switches.
 */
export function wheelStep(state: WheelState, input: WheelInput): { state: WheelState; action: WheelAction } {
  const { deltaX, deltaY, now, chain } = input;
  const freshGesture = now - state.lastWheelAt > GESTURE_GAP_MS;
  const next: WheelState = {
    lockedUntil: state.lockedUntil,
    lastWheelAt: now,
    gestureScrolled: freshGesture ? false : state.gestureScrolled,
    gestureSwitched: freshGesture ? false : state.gestureSwitched,
  };
  const horizontal = Math.abs(deltaX) > Math.abs(deltaY);
  const delta = horizontal ? deltaX : deltaY;
  if (delta === 0) return { state: next, action: { kind: 'none' } };
  // One gesture moves one tab: momentum can outlast the lock, so block until the gesture ends
  if (next.gestureSwitched || now < state.lockedUntil) return { state: next, action: { kind: 'block' } };
  if (!horizontal && chain.some(m => canScroll(m, delta))) {
    next.gestureScrolled = true;
    return { state: next, action: { kind: 'scroll' } };
  }
  if (!horizontal && next.gestureScrolled) return { state: next, action: { kind: 'none' } };
  next.lockedUntil = now + SWITCH_LOCK_MS;
  next.gestureSwitched = true;
  return { state: next, action: { kind: 'switch', delta: delta > 0 ? 1 : -1 } };
}

/** Distance alone, or a shorter flick with enough speed, turns a drag into a tab change */
export const SWIPE_DISTANCE = 56;
export const SWIPE_FLICK_DISTANCE = 20;
export const SWIPE_FLICK_SPEED = 0.35; // px per ms
/** Movement before a press becomes a drag (and stops being a click) */
export const DRAG_SLOP = 8;

/** Tab delta for a finished horizontal drag: dragging left moves to the next tab */
export function swipeIntent(dx: number, dy: number, durationMs: number): -1 | 0 | 1 {
  const distance = Math.abs(dx);
  if (distance <= Math.abs(dy) * 1.2) return 0;
  const speed = distance / Math.max(1, durationMs);
  const passed = distance >= SWIPE_DISTANCE || (distance >= SWIPE_FLICK_DISTANCE && speed >= SWIPE_FLICK_SPEED);
  if (!passed) return 0;
  return dx < 0 ? 1 : -1;
}
