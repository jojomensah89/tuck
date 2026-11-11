import { invokeCommand } from './ipc.js';
import { displayShortcut, recordKey } from './shortcut-recorder.js';
export function mountSensorSettings(): void {
  const indicator = document.getElementById('sensorIndicator') as HTMLInputElement;
  const button = document.getElementById('micShortcut') as HTMLButtonElement;
  const off = document.getElementById('micShortcutOff') as HTMLButtonElement;
  const error = document.getElementById('sensorError')!;
  let settings = { shortcut: 'Ctrl+Alt+M', indicator: true };
  const paint = () => { indicator.checked = settings.indicator; button.textContent = settings.shortcut ? displayShortcut(settings.shortcut) : 'Off'; };
  const save = async (next: typeof settings) => {
    indicator.disabled = button.disabled = off.disabled = true;
    try { await invokeCommand('sensor_configure', { settings: next }); settings = next; error.hidden = true; }
    catch (reason) { error.textContent = String(reason); error.hidden = false; }
    finally { paint(); indicator.disabled = button.disabled = off.disabled = false; }
  };
  indicator.onchange = () => { void save({ ...settings, indicator: indicator.checked }); };
  off.onclick = () => { void save({ ...settings, shortcut: '' }); };
  button.onclick = async () => {
    try {
      await invokeCommand('sensor_shortcuts_pause', { paused: true });
      await invokeCommand('dictation_shortcuts_pause', { paused: true });
    } catch (reason) { error.textContent = String(reason); error.hidden = false; void invokeCommand('sensor_shortcuts_pause', {paused:false}).catch(() => {}); void invokeCommand('dictation_shortcuts_pause', {paused:false}).catch(() => {}); return; }
    button.disabled = off.disabled = indicator.disabled = true;
    button.textContent = 'Press shortcut';
    const record = (event: KeyboardEvent) => {
      event.preventDefault(); event.stopPropagation();
      if (event.code === 'Escape') { cleanup(); paint(); return; }
      const result = recordKey({ code: event.code, ctrl: event.ctrlKey, alt: event.altKey, shift: event.shiftKey, meta: event.metaKey }, new Set([event.code]), false);
      if (result.kind === 'done') { cleanup(); void save({ ...settings, shortcut: result.value }); }
      else if (result.kind === 'refused') { error.textContent = result.reason; error.hidden = false; }
    };
    const cleanup = () => { window.removeEventListener('keydown', record, true); window.removeEventListener('blur', cancel); clearTimeout(timeout); button.disabled = off.disabled = indicator.disabled = false; void invokeCommand('sensor_shortcuts_pause', {paused:false}).catch(() => {}); void invokeCommand('dictation_shortcuts_pause', {paused:false}).catch(() => {}); };
    const cancel = () => { cleanup(); paint(); };
    const timeout = window.setTimeout(cancel, 25000);
    window.addEventListener('keydown', record, true); window.addEventListener('blur', cancel, { once: true });
  };
  void invokeCommand<typeof settings>('sensor_settings').then(value => { if (value) { settings = value; paint(); } }).catch(reason => { error.textContent = String(reason); error.hidden = false; });
}
