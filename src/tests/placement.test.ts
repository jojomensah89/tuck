import './setup.js';
import { describe, expect, it } from 'bun:test';
import { readFileSync } from 'node:fs';
import {
  DOCK_CACHE_KEY, cacheDock, choiceForValue, displayOptions, normalizeDock, normalizePlacement, readCachedDock, selectedDisplayValue,
  type PlacementState,
} from '../island-placement.js';

const desk: Pick<PlacementState, 'display' | 'displays'> = {
  display: null,
  displays: [
    { id: 'main', name: 'DELL U2723QE', primary: true },
    { id: 'tv', name: 'Display 2', primary: false },
    { id: 'left', name: 'LG 27GL', primary: false },
  ],
};

describe('display picker', () => {
  it('lists Primary first, then the other monitors by name', () => {
    expect(displayOptions(desk)).toEqual([
      { value: '', label: 'Primary' },
      { value: 'tv', label: 'Display 2' },
      { value: 'left', label: 'LG 27GL' },
    ]);
  });
  it('defaults to Primary', () => {
    expect(selectedDisplayValue(desk)).toBe('');
  });
  it('keeps an unplugged choice visible instead of turning it into Primary', () => {
    const state = { ...desk, display: { id: 'gone', name: 'Office' } };
    expect(displayOptions(state)[displayOptions(state).length - 1]).toEqual({ value: 'gone', label: 'Office (not connected)' });
    expect(selectedDisplayValue(state)).toBe('gone');
    expect(choiceForValue(state, 'gone')).toEqual({ id: 'gone', name: 'Office' });
  });
  it('shows a chosen monitor that became the primary one as Primary', () => {
    expect(selectedDisplayValue({ ...desk, display: { id: 'main', name: 'DELL' } })).toBe('');
  });
  it('sends null for Primary and the monitor identity otherwise', () => {
    expect(choiceForValue(desk, '')).toBeNull();
    expect(choiceForValue(desk, 'left')).toEqual({ id: 'left', name: 'LG 27GL' });
    expect(choiceForValue(desk, 'unknown')).toBeNull();
  });
});

describe('dock', () => {
  it('accepts only the three docks and defaults to centre', () => {
    expect(['left', 'center', 'right'].map(normalizeDock)).toEqual(['left', 'center', 'right']);
    for (const bad of ['', 'centre', 'LEFT', 3, null, undefined]) expect(normalizeDock(bad)).toBe('center');
  });
  it('remembers the last dock for the first paint', () => {
    localStorage.removeItem(DOCK_CACHE_KEY);
    expect(readCachedDock()).toBe('center');
    cacheDock('right');
    expect(readCachedDock()).toBe('right');
    localStorage.setItem(DOCK_CACHE_KEY, 'sideways');
    expect(readCachedDock()).toBe('center');
  });
  it('reads a native payload and rejects junk', () => {
    expect(normalizePlacement(null)).toBeNull();
    expect(normalizePlacement({ dock: 'left', display: { id: 'a', name: 'A' }, displays: [{ id: 'a', name: 'A', primary: true }, 7], active: 'a' }))
      .toEqual({ dock: 'left', display: { id: 'a', name: 'A' }, displays: [{ id: 'a', name: 'A', primary: true }], active: 'a' });
    expect(normalizePlacement({})?.dock).toBe('center');
  });
});

describe('native and CSS agree', () => {
  const read = (path: string) => readFileSync(new URL(`../../${path}`, import.meta.url), 'utf8');
  const rust = read('src-tauri/src/placement.rs');
  const number = (name: string) => Number(new RegExp(`pub const ${name}: u32 = ([0-9]+);`).exec(rust)![1]);
  it('uses one dock inset for the window maths and the island', () => {
    const tokens = read('src/tokens.css');
    const token = tokens.match(/--dock-inset:\s*var\((--[\w-]+)\)/)![1];
    const px = Number(new RegExp(`${token}: *([0-9]+)px`).exec(tokens)![1]);
    expect(px).toBe(number('DOCK_INSET'));
  });
  it('sizes the window canvas as the config says', () => {
    const config = JSON.parse(read('src-tauri/tauri.conf.json')).app.windows[0];
    expect(config.width).toBe(number('CANVAS_WIDTH'));
    expect(config.height).toBe(number('CANVAS_HEIGHT'));
    expect(JSON.parse(read('scripts/file-drop-probe.config.json')).app.windows[0].width).toBe(number('CANVAS_WIDTH'));
  });
});
