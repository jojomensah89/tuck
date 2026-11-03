import { LIVE_SETTINGS_KEY, normalizePreferences, PREFS_VERSION, readPreferences } from './live-state.js';
import { nativeAction, nativeService } from './live-services.js';
import { listenToEvent } from './ipc.js';
import { clearSaved, confirmThen, markSaved, onCommit, setStatus } from './settings-ui.js';

/** Binds the island, media, dictation and clipboard rows in settings.html. Every control saves itself. */
export function mountLiveSettings(): void {
  const input = (id: string) => document.getElementById(id) as HTMLInputElement;
  const control = (id: string) => document.getElementById(id) as HTMLButtonElement;
  const compact = () => [...document.querySelectorAll<HTMLInputElement>('input[name="compactContent"]')];

  /* ---------- Island and player preferences (local storage) ---------- */
  const prefs = readPreferences();
  input('hoverExpand').checked = prefs.hoverExpand; input('pinOpen').checked = prefs.pinned;
  input('lyricsOpen').checked = prefs.lyricsOpen; input('desktopPeeks').checked = prefs.desktopPeeks;
  for (const radio of compact()) radio.checked = radio.value === prefs.compactContent;
  input('openDelay').value = String(prefs.openDelayMs); input('closeDelay').value = String(prefs.closeDelayMs); input('openWidth').value = String(prefs.openWidth);

  const numbers = ['openDelay', 'closeDelay', 'openWidth'];
  const statusFor = (id: string) => id === 'openWidth' ? 'shapeStatus' : 'interactionStatus';
  const savePreferences = (changed: HTMLInputElement) => {
    const invalid = numbers.find(id => input(id).value === '' || !input(id).checkValidity());
    for (const id of numbers) {
      if (id === invalid) input(id).setAttribute('aria-invalid', 'true'); else input(id).removeAttribute('aria-invalid');
    }
    if (invalid) {
      const field = input(invalid);
      // A blocked save must not leave a switch showing a state that was never stored
      if (changed.type === 'checkbox') changed.checked = !changed.checked;
      if (changed.type === 'radio') {
        const stored = readPreferences().compactContent;
        for (const radio of compact()) radio.checked = radio.value === stored;
      }
      clearSaved(changed);
      setStatus(statusFor(invalid), `Enter a value from ${field.min} to ${field.max}${field.step && field.step !== '1' ? ` in steps of ${field.step}` : ''}.`, 'error');
      return;
    }
    const next = normalizePreferences({ hoverExpand: input('hoverExpand').checked, pinned: input('pinOpen').checked,
      lyricsOpen: input('lyricsOpen').checked, desktopPeeks: input('desktopPeeks').checked,
      compactContent: compact().find(radio => radio.checked)?.value ?? 'lyrics',
      openDelayMs: Number(input('openDelay').value), closeDelayMs: Number(input('closeDelay').value), openWidth: Number(input('openWidth').value), lastTab: readPreferences().lastTab, prefsVersion: PREFS_VERSION });
    try {
      localStorage.setItem(LIVE_SETTINGS_KEY, JSON.stringify(next));
      window.dispatchEvent(new StorageEvent('storage', { key: LIVE_SETTINGS_KEY, newValue: JSON.stringify(next) }));
      setStatus('interactionStatus', ''); setStatus('shapeStatus', '');
      markSaved(changed);
    } catch {
      clearSaved(changed);
      setStatus(statusFor(changed.id), 'Could not save settings. Check available storage and try again.', 'error');
    }
  };
  for (const id of ['hoverExpand', 'pinOpen', 'lyricsOpen', 'desktopPeeks']) input(id).addEventListener('change', () => savePreferences(input(id)));
  for (const radio of compact()) radio.addEventListener('change', () => savePreferences(radio));
  for (const id of numbers) onCommit(input(id), () => savePreferences(input(id)));

  /* ---------- Native media services ---------- */
  for (const [id, family] of [['onlineLyrics', 'lyrics'], ['audioVisualizer', 'visualizer']] as const) {
    void nativeService<{ enabled: boolean }>(`${family}_settings`).then(data => {
      input(id).checked = data.enabled === true; input(id).disabled = false;
      window.dispatchEvent(new Event('settings-loaded'));
    }).catch(() => setStatus('mediaServiceStatus', 'Lyrics lookup and the visualizer need the desktop app.'));
    input(id).onchange = async () => {
      const enabled = input(id).checked; input(id).disabled = true;
      try { await nativeAction(`${family}_set_enabled`, { enabled }); setStatus('mediaServiceStatus', ''); markSaved(input(id)); }
      catch { input(id).checked = !enabled; setStatus('mediaServiceStatus', 'Could not change service preference.', 'error'); }
      finally { input(id).disabled = false; }
    };
  }

  /* ---------- Dictation ---------- */
  const dictationFields = ['dictationShortcut', 'dictationHoldShortcut', 'dictationReview', 'dictationPreview'];
  const dictationStatus = (text: string, kind: 'info' | 'error' = 'info') => setStatus('dictationSetupStatus', text, kind);
  type DictationSettings = { toggleShortcut: string; holdToTalkShortcut: string; partialPreview: boolean; reviewBeforeInsert: boolean };
  let savedDictation: DictationSettings | null = null;
  const applyDictation = (data: DictationSettings) => {
    input('dictationShortcut').value = data.toggleShortcut || 'Ctrl+Shift+Space'; input('dictationHoldShortcut').value = data.holdToTalkShortcut || 'Ctrl+Alt+Space';
    input('dictationReview').checked = data.reviewBeforeInsert; input('dictationPreview').checked = data.partialPreview;
  };
  void nativeService<DictationSettings & { modelReady: boolean }>('dictation_settings').then(data => {
    savedDictation = data; applyDictation(data);
    dictationFields.forEach(id => { input(id).disabled = false; });
    control('downloadModel').disabled = data.modelReady;
    control('removeModel').disabled = !data.modelReady;
    dictationStatus(data.modelReady ? 'Offline model ready.' : 'Download the model before recording.');
  }).catch(() => dictationStatus('Dictation needs the desktop app.'));
  const saveDictation = async (changed: HTMLInputElement) => {
    // Lock the fields so two saves can't finish out of order and revert to stale values
    dictationFields.forEach(id => { input(id).disabled = true; });
    const settings: DictationSettings = { toggleShortcut: input('dictationShortcut').value.trim(), holdToTalkShortcut: input('dictationHoldShortcut').value.trim(), reviewBeforeInsert: input('dictationReview').checked, partialPreview: input('dictationPreview').checked };
    try { await nativeAction('dictation_set_settings', { settings }); savedDictation = settings; setStatus('dictationStatus', ''); markSaved(changed); }
    catch {
      if (savedDictation) applyDictation(savedDictation);
      clearSaved(changed);
      setStatus('dictationStatus', 'Could not save shortcut or dictation settings. The previous values are back.', 'error');
    } finally {
      dictationFields.forEach(id => { input(id).disabled = false; });
    }
  };
  for (const id of ['dictationReview', 'dictationPreview']) input(id).addEventListener('change', () => { void saveDictation(input(id)); });
  for (const id of ['dictationShortcut', 'dictationHoldShortcut']) onCommit(input(id), () => { void saveDictation(input(id)); });

  control('downloadModel').onclick = async () => {
    control('downloadModel').disabled = true; control('cancelModelDownload').disabled = false; dictationStatus('Downloading and verifying the offline model…');
    try { await nativeAction('dictation_download_model'); dictationStatus('Download started.'); }
    catch { dictationStatus('Model download failed. Check your connection and retry.', 'error'); control('downloadModel').disabled = false; control('cancelModelDownload').disabled = true; }
  };
  control('cancelModelDownload').onclick = async () => {
    try { await nativeAction('dictation_model_download_cancel'); dictationStatus('Cancellation requested.'); }
    catch { dictationStatus('Could not change the model. Stop dictation first.', 'error'); }
  };
  confirmThen(control('removeModel'), 'Remove the offline speech model? Dictation stops working until you download it again.', 'Remove model', async () => {
    try { await nativeAction('dictation_model_remove'); dictationStatus('Model removed.'); }
    catch { dictationStatus('Could not change the model. Stop dictation first.', 'error'); }
  });
  void listenToEvent<{status: string; modelInstalled: boolean; modelDownloadedBytes: number; message?: string}>('dictation_state', data => {
    control('downloadModel').disabled = data.status === 'downloadingModel' || data.modelInstalled;
    control('cancelModelDownload').disabled = data.status !== 'downloadingModel';
    control('removeModel').disabled = !data.modelInstalled || ['downloadingModel', 'starting', 'recording', 'transcribing', 'cancelling', 'inserting', 'review'].includes(data.status);
    dictationStatus(data.message || (data.status === 'downloadingModel' ? `Downloading model · ${(data.modelDownloadedBytes / 1048576).toFixed(1)} MB` : data.modelInstalled ? 'Offline model ready.' : 'Download the model before recording.'));
  });

  /* ---------- Clipboard: both flags in one native mutation ---------- */
  const clipboardIds = ['clipboardEnabled', 'clipboardPersist'];
  void nativeService<{ enabled: boolean; persist: boolean }>('clipboard_settings').then(data => {
    window.dispatchEvent(new Event('settings-loaded'));
    input('clipboardEnabled').checked = data.enabled; input('clipboardPersist').checked = data.persist;
    clipboardIds.forEach(id => { input(id).disabled = false; });
  }).catch(() => setStatus('clipboardSetupStatus', 'Clipboard history needs the desktop app.'));
  const saveClipboard = async (changed: HTMLInputElement) => {
    clipboardIds.forEach(id => { input(id).disabled = true; });
    try {
      await nativeAction('clipboard_set_settings', { settings: { enabled: input('clipboardEnabled').checked, persist: input('clipboardPersist').checked } });
      setStatus('clipboardSetupStatus', ''); markSaved(changed);
    } catch {
      clearSaved(changed);
      try {
        const current = await nativeService<{enabled: boolean; persist: boolean}>('clipboard_settings');
        input('clipboardEnabled').checked = current.enabled; input('clipboardPersist').checked = current.persist;
        setStatus('clipboardSetupStatus', 'Could not save clipboard settings. Current preferences restored below.', 'error');
      } catch { setStatus('clipboardSetupStatus', 'Could not save or read clipboard settings. Reopen Settings before trying again.', 'error'); }
    } finally {
      clipboardIds.forEach(id => { input(id).disabled = false; });
    }
  };
  for (const id of clipboardIds) input(id).addEventListener('change', () => { void saveClipboard(input(id)); });
}
