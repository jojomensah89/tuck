/**
 * The open island's icon tab rail. Builds the rail and one tabpanel per tab,
 * moves the existing sections into their panels, and turns rail clicks, keys,
 * wheel and drag swipes into tab changes. The pure rules live in tab-nav.ts.
 */
import { SVG_ICONS } from './icons.js';
import {
  clampTab,
  DRAG_SLOP,
  INITIAL_WHEEL_STATE,
  rovingTarget,
  swipeIntent,
  tabForSection,
  tabIndex,
  TABS,
  wheelStep,
  type ScrollMetrics,
  type TabId,
  type WheelState,
} from './tab-nav.js';

export interface TabRailOptions {
  initial: TabId;
  isOpen: () => boolean;
  /** Any tab change, including programmatic ones */
  onChange: (id: TabId) => void;
  /** Tab changes the user asked for (rail, keys, wheel, swipe); used to remember the tab */
  onUserChange: (id: TabId) => void;
  openSettings: () => void;
}

/** Where the incoming panel's vertical scroll lands */
type Landing = 'keep' | 'start' | 'end';

export interface TabRail {
  active(): TabId;
  show(id: TabId, options?: { landing?: Landing; user?: boolean }): void;
  /** Move by one tab. Keys wrap; wheel and swipe stop at the ends */
  step(delta: number, via: 'key' | 'wheel' | 'swipe', landing?: Landing): void;
  /** Show the tab holding a section and scroll the section into view */
  reveal(section: string): void;
}

const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"]';

export function mountTabRail(options: TabRailOptions): TabRail {
  const full = document.querySelector<HTMLElement>('.full')!;
  const container = full.querySelector<HTMLElement>('.pages-container')!;
  const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');

  /* ---------- Rail ---------- */
  const rail = document.createElement('nav');
  rail.className = 'rail';
  rail.setAttribute('aria-label', 'Island');
  const tablist = document.createElement('div');
  tablist.className = 'rail-tabs';
  tablist.setAttribute('role', 'tablist');
  tablist.setAttribute('aria-orientation', 'vertical');
  tablist.setAttribute('aria-label', 'Island tabs');
  const indicator = document.createElement('span');
  indicator.className = 'rail-indicator';
  indicator.setAttribute('aria-hidden', 'true');
  tablist.append(indicator);

  const tabs: HTMLButtonElement[] = [];
  const panels: HTMLElement[] = [];
  for (const def of TABS) {
    const tab = document.createElement('button');
    tab.type = 'button';
    tab.className = 'rail-tab';
    tab.id = `tab-${def.id}`;
    tab.dataset.tab = def.id;
    tab.setAttribute('role', 'tab');
    tab.setAttribute('aria-controls', `panel-${def.id}`);
    tab.setAttribute('aria-label', def.label);
    tab.title = def.label;
    tab.innerHTML = SVG_ICONS[def.icon];
    tabs.push(tab);
    tablist.append(tab);

    const panel = document.createElement('div');
    panel.className = 'tab-panel';
    panel.id = `panel-${def.id}`;
    panel.dataset.tab = def.id;
    panel.setAttribute('role', 'tabpanel');
    panel.setAttribute('aria-labelledby', tab.id);
    panel.tabIndex = 0; // keyboard users can scroll sections that have no controls
    panels.push(panel);
  }

  const settings = document.createElement('button');
  settings.type = 'button';
  settings.className = 'rail-tab rail-settings';
  settings.id = 'railSettings';
  settings.setAttribute('aria-label', 'Open settings');
  settings.title = 'Settings';
  settings.innerHTML = SVG_ICONS.gear;
  settings.addEventListener('click', e => {
    e.stopPropagation();
    options.openSettings();
  });
  rail.append(tablist, settings);
  full.insertBefore(rail, container);

  // Sections move into their tab's panel in the documented order. A section
  // without a mapping still stays reachable at the end of Tools.
  const sections = [...container.querySelectorAll<HTMLElement>(':scope > .page')];
  TABS.forEach((def, i) => {
    for (const name of def.sections) {
      const section = sections.find(s => s.dataset.page === name);
      if (section) panels[i].append(section);
    }
  });
  const fallback = panels[tabIndex('tools')];
  for (const section of sections) if (!section.parentElement?.classList.contains('tab-panel')) fallback.append(section);
  container.replaceChildren(...panels);

  /* ---------- Section position (passive) ---------- */
  const sectionIndex = document.createElement('div');
  sectionIndex.className = 'section-index';
  sectionIndex.setAttribute('aria-hidden', 'true');
  full.append(sectionIndex);

  const sectionsOf = (panel: HTMLElement) => [...panel.querySelectorAll<HTMLElement>(':scope > .page')];
  const visibleSection = (panel: HTMLElement): number =>
    panel.clientHeight > 0 ? Math.round(panel.scrollTop / panel.clientHeight) : 0;

  function renderSectionIndex(): void {
    const panel = panels[current];
    const count = sectionsOf(panel).length;
    if (sectionIndex.childElementCount !== count) {
      sectionIndex.replaceChildren(...Array.from({ length: count }, () => document.createElement('i')));
    }
    sectionIndex.hidden = count < 2;
    const at = visibleSection(panel);
    [...sectionIndex.children].forEach((dot, i) => dot.classList.toggle('on', i === at));
  }
  let indexFrame = 0;
  for (const panel of panels) {
    panel.addEventListener('scroll', () => {
      if (panel !== panels[current] || indexFrame) return;
      indexFrame = requestAnimationFrame(() => {
        indexFrame = 0;
        renderSectionIndex();
      });
    }, { passive: true });
  }

  /* ---------- Switching ---------- */
  const leavingHandlers = new WeakMap<HTMLElement, (e: AnimationEvent) => void>();
  function clearLeaving(panel: HTMLElement): void {
    const handler = leavingHandlers.get(panel);
    if (handler) {
      panel.removeEventListener('animationend', handler);
      leavingHandlers.delete(panel);
    }
    panel.classList.remove('leaving');
    if (!panel.classList.contains('on')) panel.classList.remove('fwd', 'back');
  }

  let current = Math.max(0, tabIndex(options.initial));

  function land(panel: HTMLElement, landing: Landing): void {
    if (landing === 'start') panel.scrollTop = 0;
    else if (landing === 'end') panel.scrollTop = panel.scrollHeight;
  }

  function focusInto(panel: HTMLElement): void {
    const section = sectionsOf(panel)[visibleSection(panel)];
    const control = section?.querySelector<HTMLElement>(FOCUSABLE);
    (control ?? panel).focus({ preventScroll: true });
  }

  function show(id: TabId, { landing = 'keep', user = false }: { landing?: Landing; user?: boolean } = {}): void {
    const next = tabIndex(id);
    if (next < 0) return;
    const previous = current;
    const changed = next !== previous;
    const focused = document.activeElement;
    const focusOnRail = focused instanceof HTMLElement && tablist.contains(focused);
    const focusInPanel = focused instanceof HTMLElement && panels[previous].contains(focused);
    const dir = next >= previous ? 1 : -1;
    current = next;

    panels.forEach((panel, i) => {
      const active = i === current;
      panel.inert = !active;
      if (active) {
        clearLeaving(panel);
        if (changed || !panel.classList.contains('on')) {
          panel.classList.remove('on', 'fwd', 'back');
          void panel.offsetWidth; // restart the enter animation, even when re-entering mid-exit
          panel.classList.add('on', dir > 0 ? 'fwd' : 'back');
        }
      } else if (i === previous && changed && panel.classList.contains('on')) {
        // The outgoing panel stays painted while it fades out in the travel direction
        panel.classList.remove('on', 'fwd', 'back');
        clearLeaving(panel);
        if (!reducedMotion.matches) {
          panel.classList.add('leaving', dir > 0 ? 'fwd' : 'back');
          const handler = (e: AnimationEvent) => { if (e.target === panel) clearLeaving(panel); };
          leavingHandlers.set(panel, handler);
          panel.addEventListener('animationend', handler);
        }
      } else {
        panel.classList.remove('on');
        clearLeaving(panel);
      }
    });
    if (changed) land(panels[current], landing);

    tabs.forEach((tab, i) => {
      const selected = i === current;
      tab.setAttribute('aria-selected', String(selected));
      tab.tabIndex = selected ? 0 : -1;
      tab.classList.toggle('on', selected);
    });
    rail.style.setProperty('--tab-index', String(current));
    renderSectionIndex();

    if (changed && options.isOpen()) {
      if (focusOnRail) tabs[current].focus({ preventScroll: true });
      else if (focusInPanel) focusInto(panels[current]);
    }
    if (changed && user) options.onUserChange(TABS[current].id);
    if (changed) options.onChange(TABS[current].id);
  }

  function step(delta: number, via: 'key' | 'wheel' | 'swipe', landing: Landing = 'keep'): void {
    const target = via === 'key'
      ? (current + delta + TABS.length) % TABS.length
      : clampTab(current, delta);
    if (target !== current) show(TABS[target].id, { landing, user: true });
  }

  function reveal(section: string): void {
    const id = tabForSection(section);
    if (!id) return;
    show(id);
    const panel = panels[tabIndex(id)];
    const target = sectionsOf(panel).find(s => s.dataset.page === section);
    if (target) panel.scrollTop = target.offsetTop;
    renderSectionIndex();
  }

  /* ---------- Rail input ---------- */
  tabs.forEach((tab, i) => {
    tab.addEventListener('click', e => {
      e.stopPropagation();
      if (i === current) {
        // Re-selecting the active tab returns to its first section
        panels[i].scrollTo({ top: 0, behavior: reducedMotion.matches ? 'auto' : 'smooth' });
        return;
      }
      show(TABS[i].id, { user: true });
    });
  });
  tablist.addEventListener('keydown', e => {
    const target = rovingTarget(e.key, current);
    if (target === null) return;
    e.preventDefault();
    e.stopPropagation();
    if (target !== current) show(TABS[target].id, { user: true });
    tabs[current].focus();
  });

  /* ---------- Wheel: scroll content first, switch at the edge ---------- */
  let wheel: WheelState = { ...INITIAL_WHEEL_STATE };
  function scrollChain(target: Element | null): ScrollMetrics[] {
    const panel = panels[current];
    if (!target || !panel.contains(target)) return [];
    const chain: ScrollMetrics[] = [];
    for (let el: Element | null = target; el; el = el.parentElement) {
      if (el instanceof HTMLElement && el.scrollHeight > el.clientHeight + 1) {
        const style = getComputedStyle(el);
        if (el === panel || /(auto|scroll)/.test(style.overflowY)) {
          chain.push({ scrollTop: el.scrollTop, scrollHeight: el.scrollHeight, clientHeight: el.clientHeight });
          // Contained scrollers never hand the wheel to their ancestors
          if (style.overscrollBehaviorY === 'contain' || style.overscrollBehaviorY === 'none') break;
        }
      }
      if (el === panel) break;
    }
    return chain;
  }
  full.addEventListener('wheel', e => {
    if (!options.isOpen()) return;
    if ((e.target as Element).closest('input, select, textarea')) return;
    const result = wheelStep(wheel, {
      deltaX: e.deltaX,
      deltaY: e.deltaY,
      now: performance.now(),
      chain: scrollChain(e.target as Element),
    });
    wheel = result.state;
    const action = result.action;
    if (action.kind === 'block') e.preventDefault();
    if (action.kind !== 'switch') return;
    e.preventDefault();
    const vertical = Math.abs(e.deltaY) >= Math.abs(e.deltaX);
    // Continuing a vertical scroll lands where the motion would have gone
    step(action.delta, 'wheel', vertical ? (action.delta > 0 ? 'start' : 'end') : 'keep');
  }, { passive: false });

  /* ---------- Pointer drag swipe ---------- */
  let drag: { id: number; x: number; y: number; t: number; active: boolean } | null = null;
  let suppressClick = false;
  container.addEventListener('pointerdown', e => {
    suppressClick = false;
    if (!options.isOpen() || e.button !== 0 || !e.isPrimary) return;
    if ((e.target as Element).closest('input, select, textarea, [role="slider"], [data-no-swipe]')) return;
    drag = { id: e.pointerId, x: e.clientX, y: e.clientY, t: performance.now(), active: false };
  });
  container.addEventListener('pointermove', e => {
    if (!drag || e.pointerId !== drag.id) return;
    const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
    if (drag.active) return;
    if (Math.abs(dy) > DRAG_SLOP && Math.abs(dy) >= Math.abs(dx)) { drag = null; return; } // a vertical scroll
    if (Math.abs(dx) > DRAG_SLOP) {
      drag.active = true;
      container.setPointerCapture(e.pointerId);
    }
  });
  const endDrag = (e: PointerEvent) => {
    if (!drag || e.pointerId !== drag.id) return;
    const finished = drag;
    drag = null;
    if (container.hasPointerCapture(e.pointerId)) container.releasePointerCapture(e.pointerId);
    if (!finished.active || e.type === 'pointercancel') return;
    suppressClick = true; // a drag is never also a click on what it started over
    const delta = swipeIntent(e.clientX - finished.x, e.clientY - finished.y, performance.now() - finished.t);
    if (delta !== 0) step(delta, 'swipe');
  };
  container.addEventListener('pointerup', endDrag);
  container.addEventListener('pointercancel', endDrag);
  container.addEventListener('click', e => {
    if (!suppressClick) return;
    suppressClick = false;
    e.preventDefault();
    e.stopPropagation();
  }, true);

  show(TABS[current].id);
  return { active: () => TABS[current].id, show, step, reveal };
}
