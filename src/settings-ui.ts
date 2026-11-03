/** Small DOM helpers shared by the settings window controllers. */

const stampTimers = new WeakMap<Element, number>();

/** Show a quiet "Saved" stamp beside the row title of a control, and announce it once. */
export function markSaved(control: Element | null, announcement = 'Saved'): void {
  const row = control?.closest('.set-row');
  if (!row) return;
  const title = row.querySelector('.row-title');
  if (title && !title.querySelector('.stamp')) {
    const stamp = document.createElement('span');
    stamp.className = 'stamp';
    stamp.setAttribute('aria-hidden', 'true');
    stamp.innerHTML = '<svg class="i"><use href="#i-check"/></svg>Saved';
    title.append(stamp);
  }
  row.classList.add('saved');
  window.clearTimeout(stampTimers.get(row));
  stampTimers.set(row, window.setTimeout(() => row.classList.remove('saved'), 1600));
  const live = document.getElementById('announce');
  if (live) { live.textContent = ''; live.textContent = announcement; }
}

/** Clear a row's saved stamp at once, for example after a failed save. */
export function clearSaved(control: Element | null): void {
  control?.closest('.set-row')?.classList.remove('saved');
}

/** Write a group status line; errors take the warning ink. */
export function setStatus(target: string | HTMLElement | null, text: string, kind: 'info' | 'error' = 'info'): void {
  const el = typeof target === 'string' ? document.getElementById(target) : target;
  if (!el) return;
  el.textContent = text;
  if (kind === 'error' && text) el.dataset.kind = 'error';
  else delete el.dataset.kind;
}

/**
 * Destructive actions ask once, inline, under their row. The confirmation
 * names the object and the consequence; Escape or Cancel backs out.
 */
export function confirmThen(trigger: HTMLButtonElement, message: string, confirmLabel: string, action: () => void | Promise<void>): void {
  trigger.addEventListener('click', () => {
    const row = trigger.closest('.set-row');
    if (!row) { void action(); return; }
    row.querySelector('.confirm')?.remove();
    const strip = document.createElement('div');
    strip.className = 'confirm';
    strip.setAttribute('role', 'group');
    strip.setAttribute('aria-label', confirmLabel);
    const text = document.createElement('p');
    text.textContent = message;
    const cancel = document.createElement('button');
    cancel.type = 'button'; cancel.className = 'button'; cancel.textContent = 'Cancel';
    const confirm = document.createElement('button');
    confirm.type = 'button'; confirm.className = 'button danger'; confirm.textContent = confirmLabel;
    const close = (refocus: boolean) => { strip.remove(); if (refocus) trigger.focus(); };
    cancel.addEventListener('click', () => close(true));
    confirm.addEventListener('click', () => { close(true); void action(); });
    strip.addEventListener('keydown', event => { if (event.key === 'Escape') { event.stopPropagation(); close(true); } });
    strip.append(text, cancel, confirm);
    row.append(strip);
    confirm.focus();
  });
}

/** Commit a field on change (blur or Enter), not on every keystroke. */
export function onCommit(input: HTMLInputElement, commit: () => void): void {
  input.addEventListener('change', commit);
  input.addEventListener('keydown', event => { if (event.key === 'Enter') input.blur(); });
}
