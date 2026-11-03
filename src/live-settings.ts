import { LIVE_SETTINGS_KEY, normalizePreferences, readPreferences } from './live-state.js';
import { nativeAction, nativeService } from './live-services.js';
import { listenToEvent } from './ipc.js';

export function mountLiveSettings(): void {
  const section = document.createElement('section'); section.id = 'liveSettings';
  section.innerHTML = `<h2>Music and interaction</h2><div class="card">
    <label><input id="hoverExpand" type="checkbox"> Expand when the pointer enters</label>
    <label><input id="pinOpen" type="checkbox"> Keep open until dismissed</label>
    <label><input id="lyricsOpen" type="checkbox"> Show lyrics beside the player</label>
    <label><input id="desktopPeeks" type="checkbox"> Show volume, brightness, connection and key peeks</label>
    <label for="compactContent">Collapsed content</label><select id="compactContent"><option value="lyrics">Current lyric, with track title fallback</option><option value="title">Track title</option></select>
    <label for="openDelay">Expand delay (milliseconds)</label><input id="openDelay" type="number" min="0" max="2000" step="50">
    <label for="closeDelay">Collapse delay (milliseconds)</label><input id="closeDelay" type="number" min="100" max="10000" step="100">
    <label for="openWidth">Expanded width (logical pixels)</label><input id="openWidth" type="number" min="440" max="500" step="10">
    <button id="saveLive">Save interaction settings</button><p id="interactionStatus" role="status"></p>
  </div>
  <h2>Lyrics and visualizer</h2><div class="card">
    <label><input id="onlineLyrics" type="checkbox" disabled> Look up lyrics with LRCLIB</label>
    <p>When enabled, the song title, artist and duration are sent directly to LRCLIB. Lyrics may be unavailable for some tracks.</p>
    <label><input id="audioVisualizer" type="checkbox" disabled> Analyze system playback for the visualizer</label>
    <p>Playback audio is analyzed in memory and is not saved. Reduced motion hides the spectrum.</p><p id="mediaServiceStatus" role="status"></p>
  </div>
  <h2>Offline dictation</h2><div class="card">
    <p>Click a text input, then use your shortcut. Tuck inserts the finished text without sending the message. Clicking a field never starts recording.</p>
    <label for="dictationShortcut">Press to start / stop shortcut</label><input id="dictationShortcut" type="text" placeholder="Ctrl+Shift+Space" disabled>
    <label for="dictationHoldShortcut">Hold to talk shortcut</label><input id="dictationHoldShortcut" type="text" placeholder="Ctrl+Alt+Space" disabled>
    <label><input id="dictationReview" type="checkbox" disabled> Review before inserting</label>
    <label><input id="dictationPreview" type="checkbox" disabled> Show partial transcript while recording</label>
    <button id="saveDictation" disabled>Save dictation settings</button>
    <button id="downloadModel" disabled>Download offline model</button><button id="cancelModelDownload">Cancel download</button><button id="removeModel" disabled>Remove model</button><p id="dictationSetupStatus" role="status">Reading native model status…</p>
  </div>
  <h2>Clipboard history</h2><div class="card">
    <label><input id="clipboardEnabled" type="checkbox" disabled> Keep clipboard history</label>
    <label><input id="clipboardPersist" type="checkbox" disabled> Save history across restarts</label>
    <p>Off by default. History clears on exit unless persistence is enabled. Pins organize items within that retention choice. Marked-private clipboard formats and Tuck's temporary dictation paste are excluded; sensitive text is not always identifiable.</p>
    <button id="saveClipboard" disabled>Save clipboard settings</button><p id="clipboardSetupStatus" role="status"></p>
  </div>`;
  document.body.append(section);
  const input = (id: string) => section.querySelector<HTMLInputElement>(`#${id}`)!;
  const select = (id: string) => section.querySelector<HTMLSelectElement>(`#${id}`)!;
  const control = (id: string) => section.querySelector<HTMLButtonElement>(`#${id}`)!;
  const status = (id: string, text: string) => { section.querySelector(`#${id}`)!.textContent = text; };
  const prefs = readPreferences();
  input('hoverExpand').checked = prefs.hoverExpand; input('pinOpen').checked = prefs.pinned;
  input('lyricsOpen').checked = prefs.lyricsOpen; select('compactContent').value = prefs.compactContent;
  input('desktopPeeks').checked = prefs.desktopPeeks;
  input('openDelay').value = String(prefs.openDelayMs); input('closeDelay').value = String(prefs.closeDelayMs); input('openWidth').value = String(prefs.openWidth);
  control('saveLive').onclick = () => {
    if (!['openDelay', 'closeDelay', 'openWidth'].every(id => input(id).value !== '' && input(id).checkValidity())) {
      status('interactionStatus', 'Enter values within the displayed ranges.'); return;
    }
    const next = normalizePreferences({ hoverExpand: input('hoverExpand').checked, pinned: input('pinOpen').checked,
      lyricsOpen: input('lyricsOpen').checked, desktopPeeks: input('desktopPeeks').checked, compactContent: select('compactContent').value,
      openDelayMs: Number(input('openDelay').value), closeDelayMs: Number(input('closeDelay').value), openWidth: Number(input('openWidth').value), lastTab: readPreferences().lastTab });
    try {
      localStorage.setItem(LIVE_SETTINGS_KEY, JSON.stringify(next));
      window.dispatchEvent(new StorageEvent('storage', { key: LIVE_SETTINGS_KEY, newValue: JSON.stringify(next) }));
      status('interactionStatus', 'Interaction settings saved.');
    } catch { status('interactionStatus', 'Could not save settings. Check available storage and try again.'); }
  };
  for (const [id, family] of [['onlineLyrics', 'lyrics'], ['audioVisualizer', 'visualizer']] as const) {
    void nativeService<{ enabled: boolean }>(`${family}_settings`).then(data => {
      input(id).checked = data.enabled === true; input(id).disabled = false;
    }).catch(() => status('mediaServiceStatus', 'One or more native media services are unavailable in this build.'));
    input(id).onchange = async () => {
      const enabled = input(id).checked; input(id).disabled = true;
      try { await nativeAction(`${family}_set_enabled`, { enabled }); status('mediaServiceStatus', enabled ? 'Enabled.' : 'Disabled.'); }
      catch { input(id).checked = !enabled; status('mediaServiceStatus', 'Could not change service preference.'); }
      finally { input(id).disabled = false; }
    };
  }
  const dictationControls = ['dictationShortcut', 'dictationHoldShortcut', 'dictationReview', 'dictationPreview', 'saveDictation', 'downloadModel'];
  void nativeService<{ toggleShortcut: string; holdToTalkShortcut: string; partialPreview: boolean; reviewBeforeInsert: boolean; modelReady: boolean }>('dictation_settings').then(data => {
    input('dictationShortcut').value = data.toggleShortcut || 'Ctrl+Shift+Space'; input('dictationHoldShortcut').value = data.holdToTalkShortcut || 'Ctrl+Alt+Space'; input('dictationReview').checked = data.reviewBeforeInsert; input('dictationPreview').checked = data.partialPreview;
    dictationControls.forEach(id => (section.querySelector(`#${id}`) as HTMLInputElement).disabled = false);
    status('dictationSetupStatus', data.modelReady ? 'Offline model ready.' : 'Download a model before recording. Audio stays on this computer.');
    control('removeModel').disabled = !data.modelReady;
  }).catch(() => status('dictationSetupStatus', 'Dictation service unavailable in this build.'));
  control('saveDictation').onclick = async () => {
    control('saveDictation').disabled = true;
    try { await nativeAction('dictation_set_settings', { settings: { toggleShortcut: input('dictationShortcut').value.trim(), holdToTalkShortcut: input('dictationHoldShortcut').value.trim(), reviewBeforeInsert: input('dictationReview').checked, partialPreview: input('dictationPreview').checked } }); status('dictationSetupStatus', 'Dictation settings saved.'); }
    catch { status('dictationSetupStatus', 'Could not save shortcut or dictation settings.'); }
    finally { control('saveDictation').disabled = false; }
  };
  control('downloadModel').onclick = async () => {
    control('downloadModel').disabled = true; status('dictationSetupStatus', 'Downloading and verifying offline model…');
    try { await nativeAction('dictation_download_model'); status('dictationSetupStatus', 'Download started. Model status updates below.'); }
    catch { status('dictationSetupStatus', 'Model download failed. Check your connection and retry.'); }
    finally { control('downloadModel').disabled = false; }
  };
  for (const [id, command] of [['cancelModelDownload', 'dictation_model_download_cancel'], ['removeModel', 'dictation_model_remove']] as const) {
    control(id).onclick = async () => {
      try { await nativeAction(command); status('dictationSetupStatus', id === 'removeModel' ? 'Model removed.' : 'Cancellation requested.'); }
      catch { status('dictationSetupStatus', 'Could not change the model. Stop dictation first.'); }
    };
  }
  void listenToEvent<{status: string; modelInstalled: boolean; modelDownloadedBytes: number; message?: string}>('dictation_state', data => {
    control('downloadModel').disabled = data.status === 'downloadingModel' || data.modelInstalled;
    control('removeModel').disabled = !data.modelInstalled || ['downloadingModel', 'starting', 'recording', 'transcribing', 'cancelling', 'inserting', 'review'].includes(data.status);
    status('dictationSetupStatus', data.message || (data.status === 'downloadingModel' ? `Downloading model · ${(data.modelDownloadedBytes / 1048576).toFixed(1)} MB` : data.modelInstalled ? 'Offline model ready.' : 'Download a model before recording.'));
  });
  void nativeService<{ enabled: boolean; persist: boolean }>('clipboard_settings').then(data => {
    input('clipboardEnabled').checked = data.enabled; input('clipboardPersist').checked = data.persist;
    ['clipboardEnabled', 'clipboardPersist', 'saveClipboard'].forEach(id => (section.querySelector(`#${id}`) as HTMLInputElement).disabled = false);
  }).catch(() => status('clipboardSetupStatus', 'Clipboard service unavailable in this build.'));
  control('saveClipboard').onclick = async () => {
    control('saveClipboard').disabled = true;
    try { await nativeAction('clipboard_set_settings', { settings: { enabled: input('clipboardEnabled').checked, persist: input('clipboardPersist').checked } }); status('clipboardSetupStatus', 'Clipboard settings saved.'); }
    catch {
      try {
        const current = await nativeService<{enabled: boolean; persist: boolean}>('clipboard_settings');
        input('clipboardEnabled').checked = current.enabled; input('clipboardPersist').checked = current.persist;
        status('clipboardSetupStatus', 'Could not save clipboard settings. Current preferences restored below.');
      } catch { status('clipboardSetupStatus', 'Could not save or read clipboard settings. Reopen Settings before trying again.'); }
    }
    finally { control('saveClipboard').disabled = false; }
  };
}
