import { setSecret } from './ipc.js';
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
  localStorage.setItem(STORAGE_KEYS.SHAPE, style);
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

    saveJson(STORAGE_KEYS.BUDGETS, {
      'anthropic-api': anthropicVal,
      'openai-api': openaiVal,
    });
    flash('s1');
  });
}

/* ================= 3. Admin Keys (Windows Credential Manager) ================= */

function initAdminKeys(): void {
  $('saveKeys')?.addEventListener('click', async () => {
    const kAnthropic = $('kAnthropic') as HTMLInputElement | null;
    const kOpenai = $('kOpenai') as HTMLInputElement | null;

    if (kAnthropic && kAnthropic.value.trim() !== '') {
      await setSecret('anthropic-admin', kAnthropic.value.trim());
      kAnthropic.value = '';
    }

    if (kOpenai && kOpenai.value.trim() !== '') {
      await setSecret('openai-admin', kOpenai.value.trim());
      kOpenai.value = '';
    }

    flash('s2');
  });
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

    saveJson<ChatGPTConfig>(STORAGE_KEYS.CHATGPT, { cap, hours, label });
    flash('s3');
  });

  $('clearChatLog')?.addEventListener('click', () => {
    saveJson(STORAGE_KEYS.CHATGPT_LOG, []);
    flash('s3');
  });
}

/* ================= Boot ================= */

document.addEventListener('DOMContentLoaded', () => {
  $('pickNotch')?.addEventListener('click', () => setShape('notch'));
  $('pickFloat')?.addEventListener('click', () => setShape('float'));
  markShape();

  initBudgets();
  initAdminKeys();
  initChatGPT();
});
