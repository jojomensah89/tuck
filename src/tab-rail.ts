/**
 * The open island's icon tab rail. Builds the rail and one tabpanel per tab,
 * moves the existing sections into their panels, and turns rail clicks, keys,
 * wheel and drag swipes into tab changes. The pure rules live in tab-nav.ts.
 */
import { mountUpdateButton } from './update-button.js';
import { SVG_ICONS } from './icons.js';
import {
  clampTab,
  DRAG_SLOP,
  INITIAL_WHEEL_STATE,
  rovingTarget,
  sectionIndexAt,
  sectionLabel,
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
  /** Include or remove a section from the tab's parts */
  setSectionVisible(section: string, visible: boolean): void;
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
  // Reserved for the update item. Hidden, it takes no room and the gear stays pinned
  const updateSlot = document.createElement('div');
  updateSlot.className = 'rail-update';
  updateSlot.id = 'railUpdate';
  updateSlot.hidden = true;
  rail.append(tablist, updateSlot, settings);
  full.insertBefore(rail, container);
  mountUpdateButton(updateSlot);

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

  /* ---------- A tab's parts: the segmented switch and the part's actions ---------- */
  const sectionRow = document.createElement('div');
  sectionRow.className = 'section-row';
  const sectionSwitch = document.createElement('div');
  sectionSwitch.className = 'section-switch';
  sectionSwitch.setAttribute('role', 'tablist');
  sectionSwitch.setAttribute('aria-label', 'Sections');
  const sectionActions = document.createElement('div');
  sectionActions.className = 'section-actions';
  // Each group names the sections it serves. It moves here once, and is only shown
  // or hidden after that, so focus survives. Groups shared by a tab's parts come last.
  const servedBy = (group: HTMLElement) => (group.dataset.sectionActions ?? '').split(' ');
  const actionGroups = [...container.querySelectorAll<HTMLElement>('[data-section-actions]')]
    .sort((a, b) => servedBy(a).length - servedBy(b).length);
  sectionActions.append(...actionGroups);
  sectionRow.append(sectionSwitch, sectionActions);
  full.insertBefore(sectionRow, container);

  const sectionsOf = (panel: HTMLElement) => [...panel.querySelectorAll<HTMLElement>(':scope > .page:not([hidden])')];
  const visibleSection = (panel: HTMLElement): number =>
    sectionIndexAt(panel.scrollTop, panel.clientHeight, sectionsOf(panel).length);

  /** Two parts need the switch; one part still needs the row if it has actions */
  function syncPanelRow(panel: HTMLElement): void {
    const parts = sectionsOf(panel);
    const name = parts[0]?.dataset.page ?? '';
    panel.classList.toggle('has-row', parts.length > 1 || actionGroups.some(group => servedBy(group).includes(name)));
  }
  panels.forEach(syncPanelRow);

  /** The same snap scroller the wheel uses */
  function goToSection(index: number): void {
    const panel = panels[current];
    const target = sectionsOf(panel)[index];
    if (target) panel.scrollTo({ top: target.offsetTop, behavior: reducedMotion.matches ? 'auto' : 'smooth' });
  }

  function buildSegment(name: string, index: number): HTMLButtonElement {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'section-switch-button';
    button.id = `section-tab-${name}`;
    button.textContent = sectionLabel(name);
    button.setAttribute('role', 'tab');
    const section = sections.find(s => s.dataset.page === name);
    if (section) {
      section.id ||= `section-${name}`;
      button.setAttribute('aria-controls', section.id);
    }
    button.addEventListener('keydown', event => {
      const target = rovingTarget(event.key, index, sectionsOf(panels[current]).length);
      if (target === null) return;
      event.preventDefault();
      event.stopPropagation();
      goToSection(target);
      (sectionSwitch.children[target] as HTMLElement | undefined)?.focus();
    });
    button.addEventListener('click', event => {
      event.stopPropagation();
      goToSection(index);
      options.onUserChange(TABS[current].id);
    });
    return button;
  }

  let switchKey: string | null = null;
  function renderSectionSwitch(): void {
    const panel = panels[current];
    const parts = sectionsOf(panel);
    const names = parts.map(section => section.dataset.page ?? 'section');
    const count = names.length;
    const at = visibleSection(panel);
    sectionRow.hidden = !panel.classList.contains('has-row');
    sectionSwitch.hidden = count < 2;
    // Tabs can have the same number of parts, so the segments are keyed by name
    const key = names.join(' ');
    if (key !== switchKey) {
      const hadFocus = sectionSwitch.contains(document.activeElement);
      switchKey = key;
      sectionSwitch.replaceChildren(...(count > 1 ? names.map(buildSegment) : []));
      for (const section of sections) {
        if (count > 1 && parts.includes(section)) {
          section.setAttribute('role', 'tabpanel');
          section.setAttribute('aria-labelledby', `section-tab-${section.dataset.page}`);
        } else if (panel.contains(section)) {
          section.removeAttribute('role');
          section.removeAttribute('aria-labelledby');
        }
      }
      if (hadFocus) (count > 1 ? sectionSwitch.children[at] as HTMLElement : panel).focus({ preventScroll: true });
    }
    [...sectionSwitch.children].forEach((button, index) => {
      button.classList.toggle('on', index === at);
      button.setAttribute('aria-selected', String(index === at));
      (button as HTMLElement).tabIndex = index === at ? 0 : -1;
    });
    // Only the part on screen can take focus or show its actions
    parts.forEach((section, index) => { section.inert = index !== at; });
    let lostFocus = false;
    for (const group of actionGroups) {
      const hidden = !servedBy(group).includes(names[at]);
      if (hidden && group.contains(document.activeElement)) lostFocus = true;
      group.hidden = hidden;
    }
    if (lostFocus) (sectionSwitch.children[at] as HTMLElement | undefined)?.focus({ preventScroll: true });
  }
  /** The section the reader last moved to by hand this session, per panel */
  const readerSection = new WeakMap<HTMLElement, string>();
  let readerMoving = false;
  const noteReader = () => { readerMoving = true; };
  full.addEventListener('wheel', noteReader, { capture: true, passive: true });
  full.addEventListener('pointerdown', noteReader, { capture: true, passive: true });
  full.addEventListener('keydown', noteReader, { capture: true });

  let switchFrame = 0;
  for (const panel of panels) {
    panel.addEventListener('scroll', () => {
      if (panel !== panels[current] || switchFrame) return;
      switchFrame = requestAnimationFrame(() => {
        switchFrame = 0;
        renderSectionSwitch();
        const name = sectionsOf(panel)[visibleSection(panel)]?.dataset.page;
        if (readerMoving && name) readerSection.set(panel, name);
      });
    }, { passive: true });
  }

  /** Back to the section the reader chose, or the first one if they have not chosen */
  function placeReader(panel: HTMLElement): void {
    readerMoving = false;
    const chosen = sectionsOf(panel).find(section => section.dataset.page === readerSection.get(panel));
    panel.scrollTop = chosen ? chosen.offsetTop : 0;
  }
  /** Panels whose sections changed while they were not laid out */
  const placeOnShow = new Set<HTMLElement>();

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
    const focusInPanel = focused instanceof HTMLElement && (panels[previous].contains(focused) || sectionRow.contains(focused));
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
    if (placeOnShow.delete(panels[current]) && landing === 'keep') placeReader(panels[current]);
    if (changed) land(panels[current], landing);

    tabs.forEach((tab, i) => {
      const selected = i === current;
      tab.setAttribute('aria-selected', String(selected));
      tab.tabIndex = selected ? 0 : -1;
      tab.classList.toggle('on', selected);
    });
    rail.style.setProperty('--tab-index', String(current));
    renderSectionSwitch();

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
    renderSectionSwitch();
  }

  function setSectionVisible(section: string, visible: boolean): void {
    const target = panels.flatMap(panel => [...panel.querySelectorAll<HTMLElement>(':scope > .page')])
      .find(page => page.dataset.page === section);
    if (!target || target.hidden === !visible) return;
    const panel = target.parentElement as HTMLElement;
    target.hidden = !visible;
    syncPanelRow(panel);
    if (panel === panels[current]) placeReader(panel);
    else placeOnShow.add(panel);
    renderSectionSwitch();
  }

  /* ---------- Rail input ---------- */
  tabs.forEach((tab, i) => {
    tab.addEventListener('click', e => {
      e.stopPropagation();
      if (i === current) {
        // Re-selecting the active tab returns to its first part
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

  /* ---------- Wheel: scroll lists first, then parts, then tabs ---------- */
  let wheel: WheelState = { ...INITIAL_WHEEL_STATE };
  /** Lists under the pointer, innermost first. The panel itself moves part by part */
  function scrollChain(target: Element): ScrollMetrics[] {
    const panel = panels[current];
    if (!panel.contains(target)) return [];
    const chain: ScrollMetrics[] = [];
    for (let el: Element | null = target; el && el !== panel; el = el.parentElement) {
      if (el instanceof HTMLElement && el.hasAttribute('data-horizontal-scroll')) {
        chain.push({ scrollTop: el.scrollLeft, scrollHeight: el.scrollWidth, clientHeight: el.clientWidth });
      }
      if (el instanceof HTMLElement && el.scrollHeight > el.clientHeight + 1) {
        const style = getComputedStyle(el);
        if (/(auto|scroll)/.test(style.overflowY)) {
          chain.push({ scrollTop: el.scrollTop, scrollHeight: el.scrollHeight, clientHeight: el.clientHeight });
          // Contained scrollers never hand the wheel to their ancestors
          if (style.overscrollBehaviorY === 'contain' || style.overscrollBehaviorY === 'none') break;
        }
      }
    }
    return chain;
  }
  full.addEventListener('wheel', e => {
    if (!options.isOpen()) return;
    const target = e.target as Element;
    if (target.closest('input, select, textarea')) return;
    const panel = panels[current];
    const result = wheelStep(wheel, {
      deltaX: e.deltaX,
      deltaY: e.deltaY,
      now: performance.now(),
      chain: scrollChain(target),
      // Over the rail the wheel means tabs; everywhere else it visits the parts first
      part: rail.contains(target) ? undefined : { index: visibleSection(panel), count: sectionsOf(panel).length },
    });
    wheel = result.state;
    const action = result.action;
    // Only a list with room scrolls natively; the panel never free-scrolls under the wheel
    if (action.kind !== 'scroll') e.preventDefault();
    const horizontalList = target.closest<HTMLElement>('[data-horizontal-scroll]');
    if (action.kind === 'scroll' && horizontalList && Math.abs(e.deltaY) >= Math.abs(e.deltaX)) {
      e.preventDefault(); horizontalList.scrollLeft += e.deltaY;
    }
    if (action.kind === 'section') goToSection(visibleSection(panel) + action.delta);
    if (action.kind !== 'switch') return;
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
  return { active: () => TABS[current].id, show, step, reveal, setSectionVisible };
}
