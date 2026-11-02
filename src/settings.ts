import {mountExtensionSettings} from './extensions.js';
import { mountLiveSettings } from './live-settings.js';
import { isTauri, setSecret, invokeCommand } from './ipc.js';
import { STORAGE_KEYS, getJson, saveJson } from './usage.js';
import type { ShapeStyle, ChatGPTConfig } from './types.js';

function $(id: string): HTMLElement | null {
  return document.getElementById(id);
}

function flash(id: string): void {
  const el = $(id);
  if (!el) return;
  el.classList.add('show');
  setTimeout(() => el.classList.remove('show'), 2000);
}

/* ================= 1. Shape Style ================= */

function markShape(): void {
  const current = (localStorage.getItem(STORAGE_KEYS.SHAPE) as ShapeStyle) === 'float' ? 'float' : 'notch';
  $('pickNotch')?.setAttribute('aria-pressed', String(current === 'notch'));
  $('pickFloat')?.setAttribute('aria-pressed', String(current === 'float'));
}

function setShape(style: ShapeStyle): void {
  try { localStorage.setItem(STORAGE_KEYS.SHAPE, style); }
  catch { $('settingsStatus')!.textContent = 'Could not save style. Check available storage and try again.'; return; }
  markShape();
  // Dispatch custom storage event for in-window testing
  window.dispatchEvent(
    new StorageEvent('storage', {
      key: STORAGE_KEYS.SHAPE,
      newValue: style,
    })
  );
}

/* ================= 2. Budgets ================= */

function initBudgets(): void {
  const budgets = getJson<Record<string, number>>(STORAGE_KEYS.BUDGETS, {});
  const bAnthropic = $('bAnthropic') as HTMLInputElement | null;
  const bOpenai = $('bOpenai') as HTMLInputElement | null;

  if (bAnthropic && budgets['anthropic-api'] != null) {
    bAnthropic.value = String(budgets['anthropic-api']);
  }
  if (bOpenai && budgets['openai-api'] != null) {
    bOpenai.value = String(budgets['openai-api']);
  }

  $('saveBudgets')?.addEventListener('click', () => {
    const anthropicVal = Number(bAnthropic?.value) || 0;
    const openaiVal = Number(bOpenai?.value) || 0;

    if (!Number.isFinite(anthropicVal) || !Number.isFinite(openaiVal) || anthropicVal < 0 || openaiVal < 0) {
      $('settingsStatus')!.textContent = 'Budgets must be zero or a positive amount.'; return;
    }
    const saved = saveJson(STORAGE_KEYS.BUDGETS, {
      'anthropic-api': anthropicVal,
      'openai-api': openaiVal,
    });
    if (saved) flash('s1');
    else $('settingsStatus')!.textContent = 'Could not save budgets. Check available storage and try again.';
  });
}

/* ================= 3. Admin Keys (Windows Credential Manager) ================= */

function initAdminKeys(): void {
  const status = $('settingsStatus')!;
  const fields = ['kAnthropic', 'kOpenai'].map(id => $(id) as HTMLInputElement);
  const controls = ['saveKeys', 'removeAnthropic', 'removeOpenai'].map(id => $(id) as HTMLButtonElement);
  async function update(changes: [string, string][]): Promise<void> {
    fields.forEach(field => { field.value = ''; });
    controls.forEach(button => { button.disabled = true; });
    status.textContent = '';
    try {
      if (!isTauri()) throw new Error('Desktop app required');
      for (const [name, value] of changes) await setSecret(name, value);
      status.textContent = changes.length ? 'Credentials updated.' : 'Enter a key to update, or choose Remove.';
    } catch {
      status.textContent = 'Credential update failed. Open the desktop app and try again. Some earlier updates may have succeeded.';
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
  $('removeAnthropic')?.addEventListener('click', () => { void update([['anthropic-admin', '']]); });
  $('removeOpenai')?.addEventListener('click', () => { void update([['openai-admin', '']]); });
}

/* ================= 4. ChatGPT Estimate ================= */

function initChatGPT(): void {
  const cfg = getJson<Partial<ChatGPTConfig>>(STORAGE_KEYS.CHATGPT, {});
  const cCap = $('cCap') as HTMLInputElement | null;
  const cHours = $('cHours') as HTMLInputElement | null;
  const cLabel = $('cLabel') as HTMLInputElement | null;

  if (cCap && cfg.cap != null) cCap.value = String(cfg.cap);
  if (cHours && cfg.hours != null) cHours.value = String(cfg.hours);
  if (cLabel && cfg.label != null) cLabel.value = cfg.label;

  $('saveChat')?.addEventListener('click', () => {
    const cap = Number(cCap?.value) || 0;
    const hours = Number(cHours?.value) || 0;
    const label = cLabel?.value.trim() || 'ChatGPT (estimate)';

    if (!Number.isInteger(cap) || cap <= 0 || !Number.isFinite(hours) || hours <= 0) {
      $('settingsStatus')!.textContent = 'Set a positive whole message cap and a positive window in hours.'; return;
    }
    if (saveJson<ChatGPTConfig>(STORAGE_KEYS.CHATGPT, { cap, hours, label })) flash('s3');
    else $('settingsStatus')!.textContent = 'Could not save estimate settings. Check available storage and try again.';
  });

  $('clearChatLog')?.addEventListener('click', () => {
    if (saveJson(STORAGE_KEYS.CHATGPT_LOG, [])) flash('s3');
    else $('settingsStatus')!.textContent = 'Could not clear the message log. Check available storage and try again.';
  });
}

/* ================= Boot ================= */

document.addEventListener('DOMContentLoaded', () => {
  $('pickNotch')?.addEventListener('click', () => setShape('notch'));
  $('pickFloat')?.addEventListener('click', () => setShape('float'));
  markShape();

  const startup = $('autostart') as HTMLInputElement;
  if (isTauri()) {
    void invokeCommand<boolean>('get_autostart').then(enabled => { startup.checked = enabled; }).catch(() => {
      $('autostartStatus')!.textContent = 'Could not read startup preference.';
    });
    startup.addEventListener('change', async () => {
      startup.disabled = true;
      try { await invokeCommand('set_autostart', { enable: startup.checked }); }
      catch { startup.checked = !startup.checked; $('autostartStatus')!.textContent = 'Could not update startup preference.'; }
      finally { startup.disabled = false; }
    });
  } else { startup.disabled = true; }
  const installed=$('installedUsage') as HTMLInputElement;
  if(isTauri()) {
    void invokeCommand<boolean>('installed_usage_enabled').then(value=>{installed.checked=value;}).catch(()=>{$('installedUsageStatus')!.textContent='Could not read usage preference.';});
    installed.addEventListener('change',async()=>{installed.disabled=true;try{await invokeCommand('set_installed_usage_enabled',{enabled:installed.checked});$('installedUsageStatus')!.textContent=installed.checked?'Reading installed accounts. Limits refresh every five minutes.':'Installed account monitoring disabled.';}catch{installed.checked=!installed.checked;$('installedUsageStatus')!.textContent='Could not save usage preference.';}finally{installed.disabled=false;}});
    $('refreshInstalledUsage')!.addEventListener('click',()=>{void invokeCommand('refresh_installed_usage').then(()=>{$('installedUsageStatus')!.textContent='Refresh requested.';}).catch(()=>{$('installedUsageStatus')!.textContent='Refresh request failed.';});});
  }else{installed.disabled=true;($('refreshInstalledUsage') as HTMLButtonElement).disabled=true;}
  mountExtensionSettings();
  mountLiveSettings();
  initBudgets();
  initAdminKeys();
  initChatGPT();
});
