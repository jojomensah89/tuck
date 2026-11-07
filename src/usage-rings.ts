/**
 * One provider card per connected source. Subscription providers combine their
 * 5-hour and weekly quotas into concentric rings; other providers keep one ring.
 */
import { SVG_ICONS } from './icons.js';
import { createRing, meterWindow, providerInitial, providerMark, remainingPct, usageTone, type Ring } from './rings.js';
import { resetSpan } from './usage.js';
import type { Meter } from './types.js';

const OUTER_SIZE = 72;
const INNER_SIZE = 54;
const RING_STROKE = 6;

export interface UsageProviderGroup {
  key: string;
  label: string;
  meters: Meter[];
}

interface ValueLine {
  window: HTMLElement;
  value: HTMLElement;
}

interface Cell {
  root: HTMLElement;
  face: HTMLElement;
  glyph: HTMLElement;
  lines: ValueLine[];
  state: HTMLElement;
  rings: Ring[];
  glyphKey: string;
}

const cells = new Map<string, Cell>();

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text = ''): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text) node.textContent = text;
  return node;
}

function providerKey(meter: Meter): string {
  const provider = meter.provider.toLowerCase();
  if (provider === 'claude') return 'anthropic-subscription';
  if (provider === 'codex' || provider === 'openai') return 'openai-subscription';
  return provider;
}

function providerLabel(meter: Meter): string {
  const key = providerKey(meter);
  if (key === 'anthropic-subscription') return 'Claude';
  if (key === 'openai-subscription') return 'OpenAI';
  return meter.label || meter.provider;
}

function windowRank(meter: Meter): number {
  const window = meterWindow(meter);
  if (window === '5h') return 0;
  if (window === '7d') return 1;
  return 2;
}

/** Groups subscription quota windows by provider. */
export function groupUsageProviders(meters: Meter[]): UsageProviderGroup[] {
  const groups = new Map<string, UsageProviderGroup>();
  for (const meter of meters.filter(m => m.source === 'official')) {
    const key = providerKey(meter);
    let group = groups.get(key);
    if (!group) {
      group = { key, label: providerLabel(meter), meters: [] };
      groups.set(key, group);
    }
    group.meters.push(meter);
  }
  for (const group of groups.values()) group.meters.sort((a, b) => windowRank(a) - windowRank(b));
  return [...groups.values()];
}

function sourceWord(_meter: Meter): string {
  return 'official';
}

/** Codex reports each quota bucket as `<bucket>-primary` and `<bucket>-secondary` */
function quotaBucket(meter: Meter): string {
  const bucket = meter.id.replace(/-(primary|secondary)$/, '');
  return bucket === meter.id ? '' : bucket;
}

/**
 * The windows a card draws: 5h outside and weekly inside, both from one quota
 * bucket, preferring a bucket that has both. Model-scoped and other windows stay
 * in the tooltip. A provider with neither window draws its first meter as one ring.
 */
export function ringMeters(meters: Meter[]): Meter[] {
  if (meters[0]?.provider === 'antigravity') return meters.slice(0, 1);
  const candidates = [...new Set(meters.map(quotaBucket))].map(bucket => (['5h', '7d'] as const)
    .map(name => meters.find(meter => quotaBucket(meter) === bucket && meterWindow(meter) === name))
    .filter((meter): meter is Meter => !!meter));
  return candidates.find(windows => windows.length === 2)
    ?? candidates.find(windows => windows.length === 1)
    ?? meters.slice(0, 1);
}

/** Full tooltip fragment for one window, including its reset. */
export function meterDetails(meter: Meter, nowSeconds = Date.now() / 1000, name = meterWindow(meter)): string {
  const parts: string[] = [];
  if (name) parts.push(name);
  const left = remainingPct(meter.usedPct);
  if (left != null) parts.push(`${left}% left`);
  const reset = resetSpan(meter.resetsAt, nowSeconds);
  if (reset) parts.push(reset === 'now' ? 'resetting now' : `resets in ${reset}`);
  if (meter.status && meter.status !== 'ok' && meter.status !== 'stale') parts.push(meter.status);
  if (meter.error) parts.push(meter.error);
  if (meter.stale) parts.push(meter.provider === 'claude' ? 'stale, open Claude Code to refresh' : 'stale, waiting for the provider');
  parts.push(sourceWord(meter));
  return parts.join(', ');
}

export function valueText(meter: Meter): string {
  const left = remainingPct(meter.usedPct);
  if (left != null) return `${left}% left`;
  return '\u2013';
}

/** The shown value says nothing is left. An empty ring has no colour of its own, so the number warns instead. */
export function isOut(meter: Meter): boolean {
  return /^0%? left$/.test(valueText(meter));
}

function glyphFor(meter: Meter): { key: string; markup: string } {
  const mark = providerMark(meter.provider);
  if (mark) return { key: mark, markup: SVG_ICONS[mark] };
  const initial = providerInitial(meter);
  return { key: `initial-${initial}`, markup: `<span class="ring-initial">${initial}</span>` };
}

/** One word for a card that cannot be trusted right now; '' when its reading is fresh */
export function usageStateWord(meters: Meter[]): string {
  if (meters.some(meter => meter.status === 'error' || meter.error)) return 'Error';
  if (meters.some(meter => meter.status === 'unavailable')) return 'Unavailable';
  return meters.some(meter => meter.stale || meter.status === 'stale') ? 'Stale' : '';
}

function setIfChanged(node: HTMLElement, text: string): void {
  if (node.textContent !== text) node.textContent = text;
}

function buildCell(drawn: Meter[]): Cell {
  const root = el('div', 'u-provider');
  const face = el('div', 'u-face');
  face.tabIndex = 0; face.setAttribute('role', 'img');
  const rings = drawn.map((_, index) => createRing(index === 0 ? OUTER_SIZE : INNER_SIZE, RING_STROKE, index === 0 ? 'u-ring-outer' : 'u-ring-inner'));
  const glyph = el('span', 'ring-glyph');
  face.append(...rings.map(ring => ring.root), glyph);
  const values = el('div', 'u-values');
  const lines = drawn.map(() => {
    const line = el('span', 'u-value-line');
    const window = el('span', 'u-window');
    const value = el('span', 'u-val');
    line.append(window, value);
    values.append(line);
    return { window, value };
  });
  const state = el('span', 'u-state');
  values.append(state);
  root.append(face, values);
  return { root, face, glyph, lines, state, rings, glyphKey: '' };
}

/** Renders connected meters and returns the number of provider cards. */
export function renderUsageRings(list: HTMLElement, meters: Meter[], nowSeconds = Date.now() / 1000): number {
  const groups = groupUsageProviders(meters);
  const hadFocus = list.contains(document.activeElement);
  const focusedKey = [...cells].find(([, cell]) => cell.root.contains(document.activeElement))?.[0];
  if (groups.length === 0) {
    cells.clear();
    list.replaceChildren();
    list.classList.remove('dense');
    if (hadFocus) list.closest<HTMLElement>('.tab-panel')?.focus({ preventScroll: true });
    return 0;
  }
  // Two rows of two fit the section at full size; a fifth card needs a third column
  list.classList.toggle('dense', groups.length > 4);

  const seen = new Set<string>();
  groups.forEach((group, index) => {
    seen.add(group.key);
    const drawn = ringMeters(group.meters);
    let cell = cells.get(group.key);
    if (!cell || cell.rings.length !== drawn.length) {
      cell?.root.remove();
      cell = buildCell(drawn);
      cells.set(group.key, cell);
    }
    const { root, rings, lines } = cell;
    if (list.children[index] !== root) list.insertBefore(root, list.children[index] ?? null);

    const { key, markup } = glyphFor(drawn[0]);
    if (cell.glyphKey !== key) { cell.glyph.innerHTML = markup; cell.glyphKey = key; }
    drawn.forEach((meter, meterIndex) => {
      // The ring drains as quota runs out; its colour still warns from how much is used
      rings[meterIndex].set(remainingPct(meter.usedPct), usageTone(meter.usedPct));
      setIfChanged(lines[meterIndex].window, meterWindow(meter));
      setIfChanged(lines[meterIndex].value, valueText(meter));
      lines[meterIndex].value.dataset.out = String(isOut(meter));
    });

    const word = usageStateWord(drawn);
    setIfChanged(cell.state, word);
    root.dataset.stale = String(word !== '');
    const others = group.meters.filter(meter => !drawn.includes(meter));
    const details = [
      group.label,
      ...drawn.map(meter => meterDetails(meter, nowSeconds)),
      ...others.map(meter => meterDetails(meter, nowSeconds, meter.label)),
    ].join('. ');
    if (root.title !== details) {
      root.title = details;
      cell.face.setAttribute('aria-label', details);
    }
  });

  for (const [key, cell] of cells) {
    if (!seen.has(key)) { cell.root.remove(); cells.delete(key); }
  }
  // A rebuilt or removed card must not drop keyboard focus out of the island
  if (hadFocus && !list.contains(document.activeElement)) {
    (cells.get(focusedKey ?? '') ?? cells.values().next().value)?.face.focus({ preventScroll: true });
  }
  return groups.length;
}
