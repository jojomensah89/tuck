import { modelHeader, modelsLocked, removeConfirmation, rowButtons, rowMeta, rowProgress, rowStateText, type ModelAction, type ModelButton, type ModelRow } from './dictation-models.js';
import { displayShortcut, DUAL_SHIFT, isDualShift, recordKey } from './shortcut-recorder.js';
import { listenToEvent } from './ipc.js';
import { nativeAction, nativeService, type DictationSettings, type DictationSnapshot } from './live-services.js';
import { clearSaved, confirmThen, markSaved, setStatus } from './settings-ui.js';

type ShortcutField = 'dictationShortcut' | 'dictationHoldShortcut';
const SHORTCUT_FIELDS: ShortcutField[] = ['dictationShortcut', 'dictationHoldShortcut'];
const RECORD_LIMIT_MS = 25_000;
const RELEASE_WAIT_MS = 2_000;
const ACTION_ERROR_MS = 12_000;

/** Tauri rejects with the Rust error string. This is the reason the owner needs to read. */
function reason(error: unknown, fallback: string): string {
  if (typeof error === 'string' && error) return error;
  if (error instanceof Error && error.message) return error.message;
  return fallback;
}

/** Binds Settings > Dictation: shortcuts, switches and the speech model picker. */
export function mountDictationSettings(): void {
  const el = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
  const toggle = (id: string) => el<HTMLInputElement>(id);
  const button = (id: string) => el<HTMLButtonElement>(id);
  const controls = () => [...SHORTCUT_FIELDS.map(button), button('dictationUseBothShifts'), toggle('dictationReview'), toggle('dictationReady')];

  let saved: DictationSettings | null = null;
  let latest: DictationSnapshot | null = null;
  let saving = false;
  const saveErrors: Partial<Record<ShortcutField, string>> = {};
  let saveStatus = '';
  let actionError = '';
  let actionErrorTimer = 0;

  const setFieldError = (field: ShortcutField, text: string) => {
    const target = el(`${field}Error`);
    target.textContent = text; target.hidden = !text;
    if (text) button(field).setAttribute('aria-invalid', 'true'); else button(field).removeAttribute('aria-invalid');
  };
  const renderFieldErrors = () => {
    const toggleProblem = latest?.shortcutProblems?.toggle || '';
    const holdProblem = latest?.shortcutProblems?.hold || '';
    setFieldError('dictationShortcut', saveErrors.dictationShortcut || toggleProblem);
    setFieldError('dictationHoldShortcut', saveErrors.dictationHoldShortcut || holdProblem);
    // Try again only helps a shortcut that was already saved, not one just refused
    button('dictationShortcutTryAgain').hidden = !toggleProblem || Boolean(saveErrors.dictationShortcut);
    button('dictationHoldShortcutTryAgain').hidden = !holdProblem || Boolean(saveErrors.dictationHoldShortcut);
  };
  const renderGroupStatus = () => {
    const text = saveStatus || latest?.settingsNotice || '';
    setStatus('dictationStatus', text, text ? 'error' : 'info');
  };
  const applyValues = (data: DictationSettings) => {
    for (const [field, value] of [['dictationShortcut', data.toggleShortcut || 'Ctrl+Shift+Space'], ['dictationHoldShortcut', data.holdToTalkShortcut || DUAL_SHIFT]] as const) {
      button(field).dataset.value = value;
      button(field).textContent = displayShortcut(value);
    }
    syncHoldExtras();
    toggle('dictationReview').checked = data.reviewBeforeInsert;
    toggle('dictationReady').checked = data.keepMicrophoneReady ?? true;
  };
  const syncHoldExtras = () => {
    const dual = isDualShift(button('dictationHoldShortcut').dataset.value ?? '');
    button('dictationUseBothShifts').hidden = dual;
    el('dictationFilterKeysNote').hidden = !dual;
  };
  const setLocked = (locked: boolean) => { for (const control of controls()) control.disabled = locked; };
  const readValues = (): DictationSettings => ({
    toggleShortcut: button('dictationShortcut').dataset.value ?? '',
    holdToTalkShortcut: button('dictationHoldShortcut').dataset.value ?? '',
    reviewBeforeInsert: toggle('dictationReview').checked,
    keepMicrophoneReady: toggle('dictationReady').checked,
  });

  /** Fields start disabled in the HTML. This is the one place they unlock. */
  const adopt = (data: DictationSnapshot) => {
    if (!data.settings) return;
    saved = data.settings;
    applyValues(saved);
    setLocked(false);
    el('dictationRetryRow').hidden = true;
    saveStatus = '';
    button('scanModels').disabled = false;
  };

  const save = async (changed: HTMLElement, field: ShortcutField | null) => {
    // One save at a time, so two cannot finish out of order and restore stale values
    saving = true; setLocked(true);
    if (field) delete saveErrors[field];
    try {
      const settings = readValues();
      await nativeAction('dictation_set_settings', { settings });
      saved = settings; saveStatus = '';
      markSaved(changed);
    } catch (error) {
      if (saved) applyValues(saved);
      clearSaved(changed);
      // The backend ends the recorder's pause itself. This covers a save that never reached it.
      void nativeAction('dictation_shortcuts_pause', { paused: false }).catch(() => { /* the backend's own limit ends the pause */ });
      const text = reason(error, 'The change could not be saved.');
      if (field) { saveErrors[field] = `${text} The previous shortcut is back.`; saveStatus = ''; }
      else saveStatus = `${text} The previous value is back.`;
    } finally {
      saving = false; setLocked(false); syncHoldExtras();
      renderFieldErrors(); renderGroupStatus();
    }
  };
  for (const id of ['dictationReview', 'dictationReady']) toggle(id).addEventListener('change', () => { void save(toggle(id), null); });

  /* ---------- Key recorder ---------- */
  const saveWhenReleased = (pressed: Set<string>, commit: () => void) => {
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      window.removeEventListener('keyup', onRelease, true);
      window.clearTimeout(limit);
      commit();
    };
    const onRelease = (event: KeyboardEvent) => {
      event.preventDefault(); pressed.delete(event.code);
      if (pressed.size === 0) finish();
    };
    window.addEventListener('keyup', onRelease, true);
    const limit = window.setTimeout(finish, RELEASE_WAIT_MS);
  };
  let stopRecording: (() => void) | null = null;
  const record = async (field: ShortcutField) => {
    stopRecording?.();
    const control = button(field);
    const allowDual = field === 'dictationHoldShortcut';
    // Tuck's own shortcuts would fire and hide the keys from this page.
    try { await nativeAction('dictation_shortcuts_pause', { paused: true }); }
    catch (error) { saveErrors[field] = reason(error, 'Could not listen for keys.'); renderFieldErrors(); return; }
    const pressed = new Set<string>();
    let finished = false;
    control.setAttribute('aria-pressed', 'true');
    control.textContent = 'Press keys, Esc to cancel';
    delete saveErrors[field]; renderFieldErrors();
    const end = () => {
      finished = true; stopRecording = null;
      window.removeEventListener('keydown', onDown, true);
      window.removeEventListener('keyup', onUp, true);
      control.removeEventListener('blur', cancel);
      window.removeEventListener('pagehide', cancel);
      window.clearTimeout(limit);
      control.removeAttribute('aria-pressed');
    };
    const cancel = () => {
      if (finished) return;
      end();
      if (saved) applyValues({ ...readValues(), toggleShortcut: saved.toggleShortcut, holdToTalkShortcut: saved.holdToTalkShortcut });
      void nativeAction('dictation_shortcuts_pause', { paused: false }).catch(() => { /* the backend's own limit ends the pause */ });
    };
    const onDown = (event: KeyboardEvent) => {
      event.preventDefault(); event.stopPropagation();
      if (event.key === 'Escape' && !event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey) { cancel(); return; }
      pressed.add(event.code);
      const result = recordKey({ code: event.code, ctrl: event.ctrlKey, alt: event.altKey, shift: event.shiftKey, meta: event.metaKey }, pressed, allowDual);
      if (result.kind === 'partial') { control.textContent = `${result.preview}+…`; return; }
      if (result.kind === 'refused') { setFieldError(field, result.reason); return; }
      end();
      control.dataset.value = result.value;
      control.textContent = displayShortcut(result.value);
      // Saving re-arms the shortcuts. Doing that while the keys are still down would start
      // a recording from the very press that defined the shortcut.
      saveWhenReleased(pressed, () => { void save(control, field); });
    };
    const onUp = (event: KeyboardEvent) => { event.preventDefault(); pressed.delete(event.code); };
    window.addEventListener('keydown', onDown, true);
    window.addEventListener('keyup', onUp, true);
    control.addEventListener('blur', cancel);
    window.addEventListener('pagehide', cancel);
    const limit = window.setTimeout(cancel, RECORD_LIMIT_MS);
    stopRecording = cancel;
  };
  for (const field of SHORTCUT_FIELDS) button(field).addEventListener('click', () => { void record(field); });
  button('dictationUseBothShifts').addEventListener('click', () => {
    stopRecording?.();
    const hold = button('dictationHoldShortcut');
    hold.dataset.value = DUAL_SHIFT; hold.textContent = displayShortcut(DUAL_SHIFT);
    void save(hold, 'dictationHoldShortcut');
  });

  /* ---------- Model picker ---------- */
  const rows = new Map<string, { root: HTMLElement; meta: HTMLElement; state: HTMLElement; warning: HTMLElement; bar: HTMLElement; actions: HTMLElement; signature: string }>();
  // A failed action explains itself for a moment, not for the rest of the session
  const showActionError = (text: string) => {
    actionError = text;
    window.clearTimeout(actionErrorTimer);
    actionErrorTimer = window.setTimeout(() => { actionError = ''; renderNotice(); }, ACTION_ERROR_MS);
  };
  const runModelAction = async (row: ModelRow, action: ModelAction) => {
    actionError = '';
    const commands: Record<ModelAction, [string, Record<string, unknown>?]> = {
      download: ['dictation_model_download', { id: row.id }],
      cancel: ['dictation_model_download_cancel'],
      use: ['dictation_model_use', { id: row.id }],
      remove: ['dictation_model_remove', { id: row.id }],
      stop: ['dictation_model_remove', { id: row.id }],
    };
    const [command, args] = commands[action];
    try { await nativeAction(command, args); }
    catch (error) { showActionError(reason(error, 'The model could not be changed.')); }
    renderNotice();
  };
  const renderNotice = () => {
    const text = actionError || latest?.modelNotice || (latest?.status === 'error' ? latest.message ?? '' : '') || '';
    const notice = el('dictationModelNotice');
    notice.textContent = text; notice.hidden = !text;
  };
  const makeButton = (row: ModelRow, spec: ModelButton, locked: boolean) => {
    const control = document.createElement('button');
    control.type = 'button';
    control.className = `button${spec.danger ? ' danger' : ''}${spec.primary ? ' primary' : ''}`;
    control.textContent = spec.label;
    control.setAttribute('aria-label', `${spec.label} ${row.label}`);
    control.disabled = locked && spec.action !== 'download' && spec.action !== 'cancel';
    if (spec.action === 'remove') confirmThen(control, removeConfirmation(row), 'Delete model', () => runModelAction(row, 'remove'));
    else control.addEventListener('click', () => { void runModelAction(row, spec.action); });
    return control;
  };
  const renderRows = (models: ModelRow[], downloaded: number, status: string) => {
    const host = el('modelRows');
    const locked = modelsLocked(status);
    const seen = new Set<string>();
    for (const row of [...models].sort((a, b) => Number(b.state === 'installed' || b.state === 'found') - Number(a.state === 'installed' || a.state === 'found'))) {
      seen.add(row.id);
      let view = rows.get(row.id);
      if (!view) {
        const root = document.createElement('div');
        root.className = 'model-row'; root.dataset.id = row.id;
        const text = document.createElement('div'); text.className = 'row-text';
        const name = document.createElement('span'); name.className = 'model-name'; name.textContent = row.label;
        const meta = document.createElement('p'); meta.className = 'row-desc';
        const state = document.createElement('p'); state.className = 'row-desc';
        const warning = document.createElement('p'); warning.className = 'row-desc row-error'; warning.hidden = true;
        const track = document.createElement('div'); track.className = 'model-progress';
        const bar = document.createElement('i'); track.append(bar);
        const actions = document.createElement('div'); actions.className = 'row-actions';
        text.append(name, meta, state, warning, track); root.append(text, actions);
        view = { root, meta, state, warning, bar, actions, signature: '' };
        rows.set(row.id, view);
      }
      host.append(view.root); // Keeps catalog order without rebuilding the row
      view.meta.textContent = rowMeta(row);
      const stateText = rowStateText(row, downloaded);
      view.state.textContent = stateText; view.state.hidden = !stateText;
      view.warning.textContent = row.memoryWarning ?? ''; view.warning.hidden = !row.memoryWarning;
      const progress = rowProgress(row, downloaded);
      (view.bar.parentElement as HTMLElement).hidden = progress === null;
      view.bar.style.transform = `scaleX(${progress ?? 0})`;
      // Buttons are rebuilt only when they change, so progress ticks keep focus and an
      // open confirmation in place.
      // The row can change file (a found Base replaces the one to download), so the id counts.
      const signature = `${row.id}|${row.state}|${row.active}|${row.canRemove}|${locked}`;
      if (signature !== view.signature) {
        view.signature = signature;
        view.actions.replaceChildren(...rowButtons(row).map(spec => makeButton(row, spec, locked)));
        const name = view.root.querySelector('.model-name') as HTMLElement;
        name.querySelector('.model-in-use')?.remove();
        if (row.active) {
          const tag = document.createElement('span'); tag.className = 'model-in-use';
          tag.innerHTML = '<svg class="i" aria-hidden="true"><use href="#i-check"/></svg>In use';
          name.append(tag);
        }
      }
    }
    for (const [id, view] of rows) if (!seen.has(id)) { view.root.remove(); rows.delete(id); }
  };

  /** The state event only describes the model and session. It never overwrites a save
   * result or a typed value, which is what made older errors flash and vanish. */
  const render = (data: DictationSnapshot) => {
    latest = data;
    el('dictationBackend').textContent = data.backend ?? 'CPU (model not loaded)';
    if (!saved && data.settings) adopt(data);
    el('dictationSetupStatus').textContent = data.scanningModels
      ? `${modelHeader(data.activeModelLabel)}. Looking for models on this computer…`
      : modelHeader(data.activeModelLabel);
    button('scanModels').disabled = Boolean(data.scanningModels) || !saved;
    if (data.models) renderRows(data.models, data.modelDownloadedBytes ?? 0, data.status);
    renderNotice(); renderFieldErrors(); renderGroupStatus();
    if (saving) setLocked(true);
  };

  /* A shortcut another app held may be free now. Checked when Settings opens or returns. */
  const retryShortcuts = () => {
    delete saveErrors.dictationShortcut; delete saveErrors.dictationHoldShortcut;
    void nativeAction('dictation_shortcuts_retry').catch(() => { /* the problem stays shown */ });
  };
  const hasShortcutProblem = () => Boolean(latest?.shortcutProblems?.toggle || latest?.shortcutProblems?.hold);
  button('dictationShortcutTryAgain').addEventListener('click', retryShortcuts);
  button('dictationHoldShortcutTryAgain').addEventListener('click', retryShortcuts);
  window.addEventListener('focus', () => { if (hasShortcutProblem() && !stopRecording) retryShortcuts(); });

  const load = () => {
    setStatus('dictationStatus', '');
    return nativeService<DictationSnapshot>('dictation_snapshot').then(data => { adopt(data); render(data); if (onScreen) scan(); if (hasShortcutProblem()) retryShortcuts(); }).catch((error: unknown) => {
      // Never locked without saying why, and never a guess at the cause
      el('dictationSetupStatus').textContent = 'Model status is unavailable.';
      setStatus('dictationStatus', reason(error, 'The dictation service did not answer.'), 'error');
      el('dictationRetryRow').hidden = false;
    });
  };
  button('dictationRetry').addEventListener('click', () => { void load(); });
  void load();
  void listenToEvent<DictationSnapshot>('dictation_state', render);

  /* Detection runs when the picker is on screen, and only then. */
  const scan = () => { void nativeAction('dictation_models_scan').catch((error: unknown) => { showActionError(reason(error, 'Could not look for models.')); renderNotice(); }); };
  button('scanModels').addEventListener('click', scan);
  let onScreen = false;
  if ('IntersectionObserver' in window) {
    new IntersectionObserver(entries => {
      const visible = entries.some(entry => entry.isIntersecting);
      if (visible && !onScreen && saved) scan();
      onScreen = visible;
    }).observe(el('row-dictationModel'));
  }
}
