import './settings.css';
import { mountExtensionSettings } from './extensions.js';
import { mountLiveSettings } from './live-settings.js';
import { isTauri, setSecret, invokeCommand } from './ipc.js';
import { STORAGE_KEYS, getJson, saveJson } from './usage.js';
import { describeResult, filterSettings, navTarget, type SearchEntry } from './settings-search.js';
import { clearSaved, confirmThen, markSaved, onCommit, setStatus } from './settings-ui.js';
import type { ShapeStyle, ChatGPTConfig } from './types.js';
import pkg from '../package.json';

function $<T extends HTMLElement = HTMLElement>(id: string): T | null {
  return document.getElementById(id) as T | null;
}

/* ================= Shell: views, search, keyboard ================= */

const VIEWS = ['general', 'island', 'media', 'activities', 'tools', 'usage', 'keys', 'privacy', 'about'];

function navLinks(): HTMLAnchorElement[] {
  return [...document.querySelectorAll<HTMLAnchorElement>('#nav a[data-view]')];
}

function currentView(): string {
  const hash = location.hash.slice(1);
  return VIEWS.includes(hash) ? hash : 'general';
}

function showView(view: string): void {
  const searching = document.body.classList.contains('searching');
  for (const link of navLinks()) {
    if (link.dataset.view === view) link.setAttribute('aria-current', 'page');
    else link.removeAttribute('aria-current');
  }
  if (searching) return;
  for (const section of document.querySelectorAll<HTMLElement>('.view')) section.hidden = section.dataset.view !== view;
  if (view === 'privacy') refreshPrivacy();
}

let searchIndex: SearchEntry[] = [];

function buildSearchIndex(): SearchEntry[] {
  const entries: SearchEntry[] = [];
  for (const section of document.querySelectorAll<HTMLElement>('.view')) {
    const viewName = section.querySelector('h1')?.textContent ?? '';
    for (const row of section.querySelectorAll<HTMLElement>('.set-row')) {
      const group = row.closest('.group')?.querySelector('h2')?.textContent ?? '';
      const words = [viewName, group, row.querySelector('.row-title')?.textContent ?? '',
        row.querySelector('.row-desc')?.textContent ?? '', row.dataset.keywords ?? ''];
      entries.push({ id: row.id, view: section.dataset.view!, text: words.join(' ') });
    }
  }
  return entries;
}

function applySearch(query: string): void {
  const searching = query.trim() !== '';
  document.body.classList.toggle('searching', searching);
  const result = filterSettings(searchIndex, query);
  const matched = new Set(result.ids);
  for (const entry of searchIndex) {
    const row = document.getElementById(entry.id);
    if (row) row.hidden = searching && !matched.has(entry.id);
  }
  for (const group of document.querySelectorAll<HTMLElement>('.group')) {
    group.hidden = searching && !group.querySelector('.set-row:not([hidden])');
  }
  for (const section of document.querySelectorAll<HTMLElement>('.view')) {
    const hits = result.views.get(section.dataset.view!) ?? 0;
    section.hidden = searching ? hits === 0 : section.dataset.view !== currentView();
    for (const extra of section.querySelectorAll<HTMLElement>('.lead, .about-head')) extra.hidden = searching;
  }
  for (const link of navLinks()) {
    if (searching && !result.views.has(link.dataset.view!)) link.dataset.empty = '';
    else delete link.dataset.empty;
  }
  $('searchEmpty')!.hidden = !searching || result.ids.length > 0;
  $('searchSummary')!.textContent = describeResult(result, query);
  if (!searching) showView(currentView());
}

function firstVisibleControl(): HTMLElement | null {
  const selector = '.view:not([hidden]) .set-row:not([hidden]) :is(input:not([disabled]), button:not([disabled]), a[href])';
  return document.querySelector<HTMLElement>(selector);
}

function mountShell(): void {
  searchIndex = buildSearchIndex();
  const search = $<HTMLInputElement>('settingsSearch')!;
  search.addEventListener('input', () => applySearch(search.value));
  search.addEventListener('keydown', event => {
    if (event.key === 'Escape' && search.value) { event.preventDefault(); search.value = ''; applySearch(''); }
    else if (event.key === 'ArrowDown' || event.key === 'Enter') {
      const target = search.value.trim() ? firstVisibleControl() : navLinks().find(link => link.getAttribute('aria-current') === 'page') ?? null;
      if (target) { event.preventDefault(); target.focus(); }
    }
  });

  const nav = $('nav')!;
  nav.addEventListener('keydown', event => {
    const links = navLinks();
    const index = links.indexOf(document.activeElement as HTMLAnchorElement);
    const next = navTarget(index, event.key, links.length);
    if (next === null) return;
    event.preventDefault();
    links[next].focus();
    links[next].click();
  });
  nav.addEventListener('click', event => {
    const link = (event.target as Element).closest<HTMLAnchorElement>('a[data-view]');
    if (!link) return;
    if (search.value) { search.value = ''; applySearch(''); }
    // Keep focus in the sidebar; the hash change swaps the view.
    if (location.hash === `#${link.dataset.view}`) { event.preventDefault(); showView(link.dataset.view!); }
  });

  document.addEventListener('keydown', event => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f') {
      event.preventDefault(); search.focus(); search.select();
    }
  });

  window.addEventListener('hashchange', () => {
    // Going to a section always leaves search, whether from the sidebar, a privacy link or history.
    if (search.value) { search.value = ''; applySearch(''); }
    $('content')!.scrollTop = 0;
    showView(currentView());
  });
  showView(currentView());
}

/* ================= Privacy mirror ================= */

function refreshPrivacy(): void {
  for (const link of document.querySelectorAll<HTMLAnchorElement>('.mirror[data-mirror]')) {
    const source = $<HTMLInputElement>(link.dataset.mirror!);
    const state = !source ? '—' : source.disabled && !source.checked ? 'Unavailable' : source.checked ? 'On' : 'Off';
    link.dataset.state = state === 'On' ? 'on' : 'off';
    link.querySelector('.mirror-state')!.textContent = state;
    const title = link.closest('.set-row')?.querySelector('.row-title')?.textContent ?? 'setting';
    link.setAttribute('aria-label', `${title}: ${state}. Change`);
  }
}

function mountPrivacy(): void {
  for (const link of document.querySelectorAll<HTMLAnchorElement>('.mirror[data-focus]')) {
    link.addEventListener('click', () => {
      const id = link.dataset.focus!;
      // Focus after the hash change has swapped the view in.
      window.setTimeout(() => $(id)?.focus(), 0);
    });
  }
  const privacyLabel = document.querySelector<HTMLAnchorElement>('#row-privacy-api .mirror');
  privacyLabel?.setAttribute('aria-label', 'API spend: change keys');
  document.addEventListener('change', refreshPrivacy);
}

/* ================= Island shape ================= */

function markShape(): void {
  const current = (localStorage.getItem(STORAGE_KEYS.SHAPE) as ShapeStyle) === 'float' ? 'float' : 'notch';
  $('pickNotch')?.setAttribute('aria-pressed', String(current === 'notch'));
  $('pickFloat')?.setAttribute('aria-pressed', String(current === 'float'));
}

function setShape(style: ShapeStyle, control: HTMLElement | null): void {
  try { localStorage.setItem(STORAGE_KEYS.SHAPE, style); }
  catch { setStatus('shapeStatus', 'Could not save the shape. Check available storage and try again.', 'error'); return; }
  setStatus('shapeStatus', '');
  markShape();
  markSaved(control, 'Island shape saved');
  // Same-window listeners do not receive storage events, so dispatch one for them.
  window.dispatchEvent(new StorageEvent('storage', { key: STORAGE_KEYS.SHAPE, newValue: style }));
}

/* ================= Budgets: saved on change ================= */

function initBudgets(): void {
  const budgets = getJson<Record<string, number>>(STORAGE_KEYS.BUDGETS, {});
  const fields: [HTMLInputElement, string][] = [[$<HTMLInputElement>('bAnthropic')!, 'anthropic-api'], [$<HTMLInputElement>('bOpenai')!, 'openai-api']];
  for (const [input, key] of fields) if (budgets[key] != null) input.value = String(budgets[key]);

  for (const [input] of fields) {
    onCommit(input, () => {
      const values = fields.map(([field]) => Number(field.value) || 0);
      if (values.some(value => !Number.isFinite(value) || value < 0)) {
        input.setAttribute('aria-invalid', 'true');
        setStatus('budgetStatus', 'Budgets must be zero or a positive amount.', 'error'); return;
      }
      input.removeAttribute('aria-invalid');
      const saved = saveJson(STORAGE_KEYS.BUDGETS, Object.fromEntries(fields.map(([, key], i) => [key, values[i]])));
      if (saved) { setStatus('budgetStatus', ''); markSaved(input, 'Budget saved'); }
      else { clearSaved(input); setStatus('budgetStatus', 'Could not save budgets. Check available storage and try again.', 'error'); }
    });
  }
}

/* ================= Admin keys: explicit save (Windows Credential Manager) ================= */

function initAdminKeys(): void {
  const status = $('settingsStatus')!;
  const fields = ['kAnthropic', 'kOpenai'].map(id => $<HTMLInputElement>(id)!);
  const controls = ['saveKeys', 'removeAnthropic', 'removeOpenai'].map(id => $<HTMLButtonElement>(id)!);
  async function update(changes: [string, string][]): Promise<void> {
    // Secrets never stay in the DOM and are never echoed back.
    fields.forEach(field => { field.value = ''; });
    controls.forEach(button => { button.disabled = true; });
    setStatus(status, '');
    try {
      if (!isTauri()) throw new Error('Desktop app required');
      for (const [name, value] of changes) await setSecret(name, value);
      setStatus(status, changes.length ? (changes.every(([, value]) => value === '') ? 'Key removed.' : 'Keys saved to Credential Manager.') : 'Paste a key to save, or choose Remove.');
    } catch {
      setStatus(status, 'Credential update failed. Open the desktop app and try again. Some earlier updates may have succeeded.', 'error');
    } finally {
      controls.forEach(button => { button.disabled = false; });
    }
  }
  $('saveKeys')?.addEventListener('click', () => {
    const changes: [string, string][] = [];
    for (let i = 0; i < fields.length; i++) {
      const value = fields[i].value.trim();
      if (value) changes.push([i === 0 ? 'anthropic-admin' : 'openai-admin', value]);
    }
    void update(changes);
  });
  for (const field of fields) field.addEventListener('keydown', event => { if (event.key === 'Enter') $('saveKeys')!.click(); });
  confirmThen($<HTMLButtonElement>('removeAnthropic')!, 'Remove the saved Anthropic admin key? Claude API spend stops updating.', 'Remove key', () => update([['anthropic-admin', '']]));
  confirmThen($<HTMLButtonElement>('removeOpenai')!, 'Remove the saved OpenAI admin key? OpenAI API spend stops updating.', 'Remove key', () => update([['openai-admin', '']]));
}

/* ================= ChatGPT estimate: saved on change ================= */

function initChatGPT(): void {
  const cfg = getJson<Partial<ChatGPTConfig>>(STORAGE_KEYS.CHATGPT, {});
  const cCap = $<HTMLInputElement>('cCap')!;
  const cHours = $<HTMLInputElement>('cHours')!;
  const cLabel = $<HTMLInputElement>('cLabel')!;

  if (cfg.cap != null) cCap.value = String(cfg.cap);
  if (cfg.hours != null) cHours.value = String(cfg.hours);
  if (cfg.label != null) cLabel.value = cfg.label;

  const commit = (changed: HTMLInputElement) => {
    const cap = Number(cCap.value) || 0;
    const hours = Number(cHours.value) || 0;
    const label = cLabel.value.trim() || 'ChatGPT (estimate)';
    const capBad = cCap.value !== '' && (!Number.isInteger(cap) || cap <= 0);
    const hoursBad = cHours.value !== '' && (!Number.isFinite(hours) || hours <= 0);
    for (const [field, bad] of [[cCap, capBad], [cHours, hoursBad]] as const) {
      if (bad) field.setAttribute('aria-invalid', 'true'); else field.removeAttribute('aria-invalid');
    }
    if (capBad || hoursBad) { clearSaved(changed); setStatus('chatStatus', 'Set a positive whole message cap and a positive window in hours.', 'error'); return; }
    if (cap <= 0 || hours <= 0) { setStatus('chatStatus', cap > 0 ? 'Set the rolling window to start the estimate.' : 'Set the message cap to start the estimate.'); return; }
    if (saveJson<ChatGPTConfig>(STORAGE_KEYS.CHATGPT, { cap, hours, label })) { setStatus('chatStatus', ''); markSaved(changed, 'Estimate saved'); }
    else { clearSaved(changed); setStatus('chatStatus', 'Could not save estimate settings. Check available storage and try again.', 'error'); }
  };
  for (const input of [cCap, cHours, cLabel]) onCommit(input, () => commit(input));

  confirmThen($<HTMLButtonElement>('clearChatLog')!, 'Clear every logged ChatGPT message? The estimate restarts from zero.', 'Clear log', () => {
    if (saveJson(STORAGE_KEYS.CHATGPT_LOG, [])) { setStatus('chatStatus', 'Message log cleared.'); }
    else setStatus('chatStatus', 'Could not clear the message log. Check available storage and try again.', 'error');
  });
}

/* ================= Native toggles: startup and installed usage ================= */

function initNativeToggles(): void {
  const startup = $<HTMLInputElement>('autostart')!;
  const installed = $<HTMLInputElement>('installedUsage')!;
  const refresh = $<HTMLButtonElement>('refreshInstalledUsage')!;
  if (!isTauri()) {
    startup.disabled = true; installed.disabled = true; refresh.disabled = true;
    setStatus('autostartStatus', 'Available in the desktop app.');
    setStatus('installedUsageStatus', 'Available in the desktop app.');
    return;
  }
  void invokeCommand<boolean>('get_autostart').then(enabled => { startup.checked = enabled; refreshPrivacy(); })
    .catch(() => setStatus('autostartStatus', 'Could not read startup preference.', 'error'));
  startup.addEventListener('change', async () => {
    startup.disabled = true;
    try { await invokeCommand('set_autostart', { enable: startup.checked }); setStatus('autostartStatus', ''); markSaved(startup); }
    catch { startup.checked = !startup.checked; setStatus('autostartStatus', 'Could not update startup preference.', 'error'); }
    finally { startup.disabled = false; }
  });

  void invokeCommand<boolean>('installed_usage_enabled').then(value => { installed.checked = value; refreshPrivacy(); })
    .catch(() => setStatus('installedUsageStatus', 'Could not read usage preference.', 'error'));
  installed.addEventListener('change', async () => {
    installed.disabled = true;
    try {
      await invokeCommand('set_installed_usage_enabled', { enabled: installed.checked });
      markSaved(installed);
      setStatus('installedUsageStatus', installed.checked ? 'Reading installed apps. Limits refresh every five minutes.' : '');
    } catch { installed.checked = !installed.checked; setStatus('installedUsageStatus', 'Could not save usage preference.', 'error'); }
    finally { installed.disabled = false; refreshPrivacy(); }
  });
  refresh.addEventListener('click', () => {
    void invokeCommand('refresh_installed_usage')
      .then(() => setStatus('installedUsageStatus', 'Refresh requested.'))
      .catch(() => setStatus('installedUsageStatus', 'Refresh request failed.', 'error'));
  });
}

/* ================= Boot ================= */

document.addEventListener('DOMContentLoaded', () => {
  const version = $('appVersion');
  if (version) version.textContent = pkg.version;
  $('pickNotch')?.addEventListener('click', event => setShape('notch', event.currentTarget as HTMLElement));
  $('pickFloat')?.addEventListener('click', event => setShape('float', event.currentTarget as HTMLElement));
  markShape();
  initNativeToggles();
  mountExtensionSettings();
  mountLiveSettings();
  initBudgets();
  initAdminKeys();
  initChatGPT();
  mountPrivacy();
  mountShell();
  refreshPrivacy();
});
