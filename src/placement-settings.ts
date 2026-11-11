import { choiceForValue, displayOptions, normalizePlacement, selectedDisplayValue, normalizeDock, type PlacementState } from './island-placement.js';
import { invokeCommand, isTauri, listenToEvent } from './ipc.js';
import { clearSaved, markSaved, setStatus } from './settings-ui.js';

/** Settings > Island > Placement: the monitor and the side of its top edge. The island moves as soon as either changes. */
export function mountPlacementSettings(): void {
  const select = document.getElementById('islandDisplay') as HTMLSelectElement | null;
  const radios = [...document.querySelectorAll<HTMLInputElement>('input[name="islandDock"]')];
  if (!select || !radios.length) return;
  if (!isTauri()) { setStatus('placementStatus', 'Available in the desktop app.'); return; }

  let state: PlacementState | null = null;

  const render = (next: PlacementState) => {
    state = next;
    select.replaceChildren(...displayOptions(next).map(option => {
      const node = document.createElement('option');
      node.value = option.value; node.textContent = option.label;
      return node;
    }));
    select.value = selectedDisplayValue(next);
    for (const radio of radios) radio.checked = radio.value === next.dock;
    select.disabled = false;
    for (const radio of radios) radio.disabled = false;
  };

  const save = async (control: HTMLElement, display: ReturnType<typeof choiceForValue>, dock: ReturnType<typeof normalizeDock>) => {
    try {
      const saved = normalizePlacement(await invokeCommand('island_placement_set', { display, dock }));
      if (saved) render(saved);
      setStatus('placementStatus', '');
      markSaved(control, 'Island position saved');
    } catch (error) {
      clearSaved(control);
      if (state) render(state);
      setStatus('placementStatus', typeof error === 'string' && error ? error : 'Could not move the island. Try again.', 'error');
    }
  };

  select.addEventListener('change', () => {
    if (state) void save(select, choiceForValue(state, select.value), state.dock);
  });
  for (const radio of radios) {
    radio.addEventListener('change', () => {
      if (state && radio.checked) void save(radio, state.display, normalizeDock(radio.value));
    });
  }

  void invokeCommand('island_placement_get')
    .then(value => { const next = normalizePlacement(value); if (next) render(next); })
    .catch(() => setStatus('placementStatus', 'Could not read the island position.', 'error'));
  // Plugging a monitor in or out updates the list while Settings is open
  void listenToEvent('island-placement', value => { const next = normalizePlacement(value); if (next) render(next); });
}
