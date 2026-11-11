/** Which monitor the island lives on and where along its top edge. Rust owns the choice
 *  (`src-tauri/src/placement.rs`); this file only describes it for the pickers and the CSS. */

export type Dock = 'left' | 'center' | 'right';
export const DOCKS: readonly Dock[] = ['left', 'center', 'right'];

export interface RememberedDisplay { id: string; name: string }
export interface DisplayEntry { id: string; name: string; primary: boolean }
export interface PlacementState {
  display: RememberedDisplay | null;
  dock: Dock;
  displays: DisplayEntry[];
  /** The monitor the island is on right now */
  active: string | null;
}

export const DOCK_CACHE_KEY = 'tuck.dock';
/** The picker value that follows whichever monitor is primary */
export const PRIMARY_VALUE = '';

export function normalizeDock(value: unknown): Dock {
  return value === 'left' || value === 'right' ? value : 'center';
}

/** The last dock Rust reported, so a restart paints the island on the right side at once. */
export function readCachedDock(): Dock {
  try { return normalizeDock(localStorage.getItem(DOCK_CACHE_KEY)); } catch { return 'center'; }
}

export function cacheDock(dock: Dock): void {
  try { localStorage.setItem(DOCK_CACHE_KEY, dock); } catch { /* storage disabled */ }
}

export interface DisplayOption { value: string; label: string }

/** "Primary" first, then each other connected monitor. A remembered monitor that is
 *  unplugged stays in the list, so the choice does not silently turn into Primary. */
export function displayOptions(state: Pick<PlacementState, 'display' | 'displays'>): DisplayOption[] {
  const options: DisplayOption[] = [{ value: PRIMARY_VALUE, label: 'Primary' }];
  for (const display of state.displays) {
    if (!display.primary) options.push({ value: display.id, label: display.name });
  }
  const remembered = state.display;
  if (remembered && !state.displays.some(display => display.id === remembered.id)) {
    options.push({ value: remembered.id, label: `${remembered.name || 'Display'} (not connected)` });
  }
  return options;
}

/** The option to show as chosen. A remembered monitor that is the primary one right now reads
 *  as Primary, because the list has no second entry for it. */
export function selectedDisplayValue(state: Pick<PlacementState, 'display' | 'displays'>): string {
  const remembered = state.display;
  if (!remembered) return PRIMARY_VALUE;
  const connected = state.displays.find(display => display.id === remembered.id);
  return connected?.primary ? PRIMARY_VALUE : remembered.id;
}

/** What to send for a picker value. */
export function choiceForValue(state: Pick<PlacementState, 'display' | 'displays'>, value: string): RememberedDisplay | null {
  if (value === PRIMARY_VALUE) return null;
  const connected = state.displays.find(display => display.id === value);
  if (connected) return { id: connected.id, name: connected.name };
  return state.display?.id === value ? state.display : null;
}

export function normalizePlacement(input: unknown): PlacementState | null {
  if (!input || typeof input !== 'object') return null;
  const raw = input as Partial<PlacementState>;
  const displays = Array.isArray(raw.displays)
    ? raw.displays.filter(entry => entry && typeof entry.id === 'string' && typeof entry.name === 'string')
        .map(entry => ({ id: entry.id, name: entry.name, primary: entry.primary === true }))
    : [];
  const display = raw.display && typeof raw.display.id === 'string'
    ? { id: raw.display.id, name: typeof raw.display.name === 'string' ? raw.display.name : '' }
    : null;
  return { display, dock: normalizeDock(raw.dock), displays, active: typeof raw.active === 'string' ? raw.active : null };
}
