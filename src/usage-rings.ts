/**
 * AI usage as a row of rings: the provider's mark in the centre, the fill in a
 * threshold tone (green, amber from 70%, red from 90%), and the value and reset
 * time in small tabular text underneath. Full details live in the title and the
 * accessible name. Cells are keyed by meter id, so a changed value animates
 * instead of being rebuilt.
 */
import { SVG_ICONS } from './icons.js';
import { createRing, meterWindow, providerInitial, providerMark, usageTone, type Ring } from './rings.js';
import { resetSpan } from './usage.js';
import type { Meter } from './types.js';

const RING_SIZE = 56;
const RING_STROKE = 6;

interface Cell {
  root: HTMLElement;
  face: HTMLElement;
  glyph: HTMLElement;
  ring: Ring;
  value: HTMLElement;
  sub: HTMLElement;
  glyphKey: string;
}

const cells = new Map<string, Cell>();

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text = ''): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (text) node.textContent = text;
  return node;
}

function valueText(m: Meter): string {
  if (m.id === 'chatgpt' && m.count != null && m.cap != null) return `${m.count}/${m.cap}`;
  if (m.usedPct != null && Number.isFinite(m.usedPct)) return `${Math.round(m.usedPct)}%`;
  if (m.spentUsd != null && Number.isFinite(m.spentUsd)) return `$${m.spentUsd.toFixed(2)}`;
  return '–';
}

function sourceWord(m: Meter): string {
  return m.source === 'manual' ? 'estimate' : m.source === 'api' ? 'API' : 'official';
}

/** The full sentence for the tooltip and the accessible name */
export function meterDetails(m: Meter, nowSeconds = Date.now() / 1000): string {
  const parts: string[] = [m.label];
  if (m.usedPct != null && Number.isFinite(m.usedPct)) parts.push(`${Math.round(m.usedPct)}% used`);
  if (m.id === 'chatgpt' && m.count != null && m.cap != null) parts.push(`${m.count} of ${m.cap} messages`);
  if (m.spentUsd != null && Number.isFinite(m.spentUsd)) parts.push(`$${m.spentUsd.toFixed(2)} spent this month`);
  const reset = resetSpan(m.resetsAt, nowSeconds);
  if (reset) parts.push(reset === 'now' ? 'resetting now' : m.id === 'chatgpt' ? `next slot frees in ${reset}` : `resets in ${reset}`);
  parts.push(sourceWord(m));
  if (m.status && m.status !== 'ok') parts.push(m.status);
  if (m.error) parts.push(m.error);
  if (m.stale) parts.push(m.provider === 'claude' ? 'stale, open Claude Code to refresh' : 'stale, waiting for the provider');
  return parts.join(', ');
}

function subText(m: Meter, nowSeconds: number): string {
  const window = meterWindow(m);
  if (m.error || m.status === 'unavailable') return window ? `${window} · off` : 'off';
  const reset = resetSpan(m.resetsAt, nowSeconds);
  return [window, reset].filter(Boolean).join(' · ');
}

function glyphFor(m: Meter): { key: string; markup: string } {
  const mark = providerMark(m.provider);
  if (mark) return { key: mark, markup: SVG_ICONS[mark] };
  const initial = providerInitial(m);
  return { key: `initial-${initial}`, markup: `<span class="ring-initial">${initial}</span>` };
}

function buildCell(m: Meter, onLog: () => void): Cell {
  const root = el('div', 'u-cell');
  const isLog = m.id === 'chatgpt';
  const face = isLog ? el('button', 'u-face u-log') : el('div', 'u-face');
  if (face instanceof HTMLButtonElement) {
    face.type = 'button';
    face.addEventListener('click', e => { e.stopPropagation(); onLog(); });
  } else {
    face.tabIndex = 0;
    face.setAttribute('role', 'img');
  }
  const ring = createRing(RING_SIZE, RING_STROKE);
  const glyph = el('span', 'ring-glyph');
  face.append(ring.root, glyph);
  if (isLog) {
    const badge = el('span', 'u-log-badge');
    badge.innerHTML = SVG_ICONS.plus;
    face.append(badge);
  }
  const value = el('span', 'u-val');
  const sub = el('span', 'u-sub');
  root.append(face, value, sub);
  return { root, face, glyph, ring, value, sub, glyphKey: '' };
}

/** Renders the meters into `list`, reusing cells so values animate */
export function renderUsageRings(list: HTMLElement, meters: Meter[], onLog: () => void, nowSeconds = Date.now() / 1000): void {
  if (meters.length === 0) {
    cells.clear();
    const empty = el('p', 'u-empty', 'No usage sources yet, connect them in Settings');
    list.replaceChildren(empty);
    list.classList.remove('dense');
    return;
  }
  list.querySelector('.u-empty')?.remove();
  list.classList.toggle('dense', meters.length > 5);

  const seen = new Set<string>();
  meters.forEach((m, index) => {
    seen.add(m.id);
    let cell = cells.get(m.id);
    if (!cell) {
      cell = buildCell(m, onLog);
      cells.set(m.id, cell);
    }
    // Keep DOM order equal to meter order without detaching unchanged cells
    if (list.children[index] !== cell.root) list.insertBefore(cell.root, list.children[index] ?? null);

    const { key, markup } = glyphFor(m);
    if (cell.glyphKey !== key) { cell.glyph.innerHTML = markup; cell.glyphKey = key; }

    const unavailable = !!m.error || m.status === 'unavailable';
    const pct = unavailable ? null : m.usedPct;
    cell.ring.set(pct, usageTone(pct));
    cell.root.dataset.stale = String(m.stale || unavailable);

    const value = unavailable ? '–' : valueText(m);
    if (cell.value.textContent !== value) cell.value.textContent = value;
    const sub = subText(m, nowSeconds);
    if (cell.sub.textContent !== sub) cell.sub.textContent = sub;

    const details = meterDetails(m, nowSeconds);
    const label = m.id === 'chatgpt' ? `Log a ChatGPT message. ${details}` : details;
    cell.face.title = label;
    cell.face.setAttribute('aria-label', label);
  });
  for (const [id, cell] of cells) {
    if (!seen.has(id)) { cell.root.remove(); cells.delete(id); }
  }
}
