import './settings.css';
import { mountExtensionSettings } from './extensions.js';
import { mountLiveSettings } from './live-settings.js';
import { isTauri, invokeCommand, listenToEvent } from './ipc.js';
import { STORAGE_KEYS, getJson, saveJson } from './usage.js';
import { describeResult, filterSettings, isRowHidden, navTarget, type SearchEntry } from './settings-search.js';
import { clearSaved, confirmThen, markSaved, onCommit, setStatus } from './settings-ui.js';
import { checkForUpdates, downloadAndInstallUpdate, type UpdateInfo } from './updater.js';
import { parseReleaseNotes } from './update-notes.js';
import type { ShapeStyle, ChatGPTConfig } from './types.js';
import pkg from '../package.json';

function $<T extends HTMLElement = HTMLElement>(id: string): T | null {
  return document.getElementById(id) as T | null;
}

/* ================= Shell: views, search, keyboard ================= */

const VIEWS = ['general', 'island', 'media', 'activities', 'tools', 'usage', 'privacy', 'about'];

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

const UPDATE_ROW = 'row-update-available';
let updateAvailable = false;

/** Rows that exist in the document but have nothing to show right now. */
function unavailableRows(): ReadonlySet<string> {
  return new Set(updateAvailable ? [] : [UPDATE_ROW]);
}

function setUpdateAvailable(available: boolean): void {
  updateAvailable = available;
  applySearch($<HTMLInputElement>('settingsSearch')?.value ?? '');
}

function applySearch(query: string): void {
  const searching = query.trim() !== '';
  document.body.classList.toggle('searching', searching);
  const unavailable = unavailableRows();
  const result = filterSettings(searchIndex, query, unavailable);
  const matched = new Set(result.ids);
  for (const entry of searchIndex) {
    const row = document.getElementById(entry.id);
    if (row) row.hidden = isRowHidden(entry.id, searching, matched, unavailable);
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
      const search = $<HTMLInputElement>('settingsSearch');
      if (search?.value) { search.value = ''; applySearch(''); }
      // Same view: the hash won't change, so show the target view directly
      const view = link.getAttribute('href')?.slice(1);
      if (view && location.hash === `#${view}`) showView(view);
      // Focus after the hash change has swapped the view in.
      window.setTimeout(() => $(id)?.focus(), 0);
    });
  }
  document.addEventListener('change', refreshPrivacy);
  window.addEventListener('settings-loaded', refreshPrivacy);
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

interface FlyoutStatus { enabled: boolean; state: 'off' | 'hidden' | 'waiting' | 'unavailable'; message: string | null }

/** Opt-in: hide the Windows volume/brightness flyout. Native state wins; it can turn itself off. */
function initFlyoutToggle(): void {
  const toggle = $<HTMLInputElement>('hideSystemFlyout')!;
  if (!isTauri()) {
    toggle.disabled = true;
    setStatus('flyoutStatus', 'Available in the desktop app.');
    return;
  }
  const apply = (status: FlyoutStatus | null) => {
    if (!status) return;
    toggle.checked = status.enabled;
    setStatus('flyoutStatus', status.message ?? '', status.state === 'unavailable' ? 'error' : 'info');
  };
  void invokeCommand<FlyoutStatus>('get_hide_system_flyout').then(apply)
    .catch(() => setStatus('flyoutStatus', 'Could not read the flyout setting.', 'error'));
  void listenToEvent<FlyoutStatus>('system-flyout', apply);
  toggle.addEventListener('change', async () => {
    toggle.disabled = true;
    try {
      const status = await invokeCommand<FlyoutStatus>('set_hide_system_flyout', { enabled: toggle.checked });
      apply(status);
      if (status?.state !== 'unavailable') markSaved(toggle);
    } catch { toggle.checked = !toggle.checked; setStatus('flyoutStatus', 'Could not update the flyout setting.', 'error'); }
    finally { toggle.disabled = false; }
  });
}

function initNativeToggles(): void {
  initFlyoutToggle();
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

  void listenToEvent<boolean>('autostart-changed', enabled => {
    startup.checked = enabled;
    refreshPrivacy();
  });
  void listenToEvent<string>('navigate-settings-view', view => {
    if (VIEWS.includes(view)) {
      showView(view);
      location.hash = `#${view}`;
    }
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

function renderReleaseNotes(container: HTMLElement, body: string | undefined): void {
  container.replaceChildren();
  let list: HTMLUListElement | null = null;
  for (const block of parseReleaseNotes(body ?? '')) {
    if (block.kind === 'item') {
      if (!list) list = container.appendChild(document.createElement('ul'));
      list.appendChild(document.createElement('li')).textContent = block.text;
      continue;
    }
    list = null;
    container.appendChild(document.createElement(block.kind === 'heading' ? 'h3' : 'p')).textContent = block.text;
  }
  if (!container.hasChildNodes()) container.appendChild(document.createElement('p')).textContent = 'No release notes provided.';
}

function initUpdates(): void {
  const btnCheck = $<HTMLButtonElement>('btnCheckUpdates')!;
  const btnInstall = $<HTMLButtonElement>('btnInstallUpdate')!;
  const statusText = $('updateStatusText')!;
  const versionBadge = $('updateVersionBadge')!;
  const updateNotes = $('updateNotes')!;
  const progressWrap = $('updateProgressWrap')!;
  const progressBar = $<HTMLElement>('updateProgressBar')!;
  const progressLabel = $('updateProgressLabel')!;

  let installing = false;

  function showPendingUpdate(info: UpdateInfo | null): void {
    if (installing) return;
    if (info) {
      statusText.textContent = `Update available: v${info.version}.`;
      versionBadge.textContent = `v${info.version}`;
      renderReleaseNotes(updateNotes, info.body);
    } else if (updateAvailable) {
      statusText.textContent = 'Check for updates.';
    }
    setUpdateAvailable(info !== null);
  }

  if (!isTauri()) {
    btnCheck.disabled = true;
    statusText.textContent = 'Updates are available in the desktop app.';
    return;
  }

  // The background check runs in the island window and reports through Rust
  void listenToEvent<UpdateInfo | null>('pending-update-changed', showPendingUpdate);
  invokeCommand<UpdateInfo | null>('get_pending_update').then(showPendingUpdate).catch(() => {});

  btnCheck.addEventListener('click', async () => {
    btnCheck.disabled = true;
    const originalText = btnCheck.textContent;
    btnCheck.textContent = 'Checking...';
    statusText.textContent = 'Checking for updates...';

    try {
      const update = await checkForUpdates(true);
      showPendingUpdate(update);
      if (!update) statusText.textContent = `Tuck v${pkg.version} is up to date.`;
      markSaved(btnCheck);
    } catch (err) {
      console.warn('[updater] Manual check failed:', err);
      statusText.textContent = 'Could not reach update server. Check your connection.';
    } finally {
      btnCheck.disabled = false;
      btnCheck.textContent = originalText;
    }
  });

  // The tray item opens or focuses this window and asks for a fresh check
  void listenToEvent<null>('check-for-updates-requested', () => {
    showView('about');
    location.hash = '#about';
    btnCheck.click();
  });
  if (new URLSearchParams(window.location.search).get('check') === '1') btnCheck.click();

  btnInstall.addEventListener('click', async () => {
    installing = true;
    btnInstall.disabled = true;
    btnInstall.textContent = 'Downloading...';
    progressWrap.hidden = false;
    progressLabel.textContent = 'Downloading...';
    progressBar.style.width = '0%';

    try {
      await downloadAndInstallUpdate((downloaded, total) => {
        const mbDl = (downloaded / (1024 * 1024)).toFixed(1);
        if (total && total > 0) {
          const pct = Math.min(100, Math.round((downloaded / total) * 100));
          progressBar.style.width = `${pct}%`;
          progressLabel.textContent = `Downloading... ${pct}% (${mbDl} / ${(total / (1024 * 1024)).toFixed(1)} MB)`;
        } else {
          progressLabel.textContent = `Downloading... ${mbDl} MB`;
        }
      }, () => {
        btnInstall.textContent = 'Installing...';
        progressLabel.textContent = 'Installing update. Tuck will restart.';
      });
    } catch (err) {
      console.error('[updater] Download/install error:', err);
      installing = false;
      btnInstall.disabled = false;
      btnInstall.textContent = 'Download and install';
      progressLabel.textContent = typeof err === 'string' ? err : (err instanceof Error ? err.message : 'Installation failed. Please try again.');
    }
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
  initUpdates();
  mountExtensionSettings();
  mountLiveSettings();
  initChatGPT();
  mountPrivacy();
  mountShell();
  refreshPrivacy();
});
