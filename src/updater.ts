import { invokeCommand, isTauri, listenToEvent } from './ipc.js';
export interface UpdateInfo { version: string; currentVersion: string; body?: string; date?: string }
export interface UpdateState { info: UpdateInfo | null; phase: 'available' | 'downloading' | 'restart' | 'installing'; progress: number | null; error: string | null; blocker: string | null }
export async function isStoreBuild(): Promise<boolean> {
  return isTauri() ? invokeCommand<boolean>('is_store_build') : false;
}
const SIX_HOURS_MS = 6 * 60 * 60 * 1000;
export async function checkForUpdates(manual = false): Promise<UpdateInfo | null> {
  if (!isTauri() || await isStoreBuild()) return null;
  try { return await invokeCommand<UpdateInfo | null>('update_check'); }
  catch (e) { if (manual) throw e; console.debug('[updater] Check skipped', e); return null; }
}
export const updateAction = async () => { if (!await isStoreBuild()) await invokeCommand<void>('update_action'); };
export async function subscribeUpdate(handler: (state: UpdateState) => void): Promise<void> {
  if (await isStoreBuild()) return;
  let received = false;
  await listenToEvent<UpdateState>('update-state-changed', state => { received = true; handler(state); });
  const snapshot = await invokeCommand<UpdateState>('update_snapshot');
  if (!received && snapshot) handler(snapshot);
}
/**
 * Checks shortly after startup and then every six hours, with a one-shot timer rescheduled
 * after each check. Does no work in between.
 */
export async function startBackgroundUpdateScheduler(initialDelayMs = 20_000): Promise<void> {
  if (!isTauri() || await isStoreBuild()) return;

  function checkThenSchedule(): void {
    void checkForUpdates(false).finally(() => {
      window.setTimeout(checkThenSchedule, SIX_HOURS_MS);
    });
  }

  window.setTimeout(checkThenSchedule, initialDelayMs);
}
