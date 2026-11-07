import { check, type Update } from '@tauri-apps/plugin-updater';
import { invokeCommand, isTauri } from './ipc.js';

export interface UpdateInfo {
  version: string;
  currentVersion: string;
  body?: string;
  date?: string;
}

const SIX_HOURS_MS = 6 * 60 * 60 * 1000;

let cachedUpdate: Update | null = null;
let downloadedUpdate: Update | null = null;
let installInProgress = false;
let checkInFlight: Promise<UpdateInfo | null> | null = null;

/** Each check holds a native resource with the update's data, so the previous one must be closed. */
async function replaceCachedUpdate(update: Update | null): Promise<void> {
  const previous = cachedUpdate;
  cachedUpdate = update;
  if (previous && previous !== update) await previous.close().catch(() => {});
}

function describeUpdate(update: Update): UpdateInfo {
  return {
    version: update.version,
    currentVersion: update.currentVersion,
    body: update.body ?? '',
    date: update.date ?? '',
  };
}

/** A hung request must not stop later checks or leave a button on "Checking...". */
const CHECK_TIMEOUT_MS = 30_000;
const DOWNLOAD_TIMEOUT_MS = 10 * 60_000;

async function runCheck(): Promise<UpdateInfo | null> {
  const update = await check({ timeout: CHECK_TIMEOUT_MS });
  await replaceCachedUpdate(update);
  const info = update ? describeUpdate(update) : null;
  await invokeCommand('set_pending_update', { update: info }).catch((e) => {
    console.warn('[updater] Failed to store pending update in Rust:', e);
  });
  return info;
}

/**
 * Check for updates. Callers during a running check share its result. A manual check
 * reports errors to its caller; a background check stays quiet when offline.
 */
export async function checkForUpdates(manual = false): Promise<UpdateInfo | null> {
  if (!isTauri()) return null;
  // The update being installed must not be replaced under the download
  if (installInProgress) return cachedUpdate ? describeUpdate(cachedUpdate) : null;

  checkInFlight ??= runCheck().finally(() => {
    checkInFlight = null;
  });

  try {
    return await checkInFlight;
  } catch (err) {
    if (manual) throw err;
    console.debug('[updater] Background check skipped/offline:', err);
    return null;
  }
}

/**
 * Download and install the available update. Runs only from an explicit click.
 * The app exits inside `install()` when it succeeds, so `onInstalling` is the last
 * chance to tell the person what is happening.
 */
export async function downloadAndInstallUpdate(
  onProgress: (downloaded: number, total: number | null) => void,
  onInstalling: () => void,
): Promise<void> {
  if (!isTauri()) throw new Error('Not running in Tauri desktop environment');
  await invokeCommand('check_update_allowed');

  installInProgress = true;
  try {
    if (!cachedUpdate) {
      const found = await check({ timeout: CHECK_TIMEOUT_MS });
      if (!found) throw new Error('No update available to install');
      await replaceCachedUpdate(found);
    }
    const update = cachedUpdate!;

    // A retry after dictation blocked the install does not download again
    if (downloadedUpdate !== update) {
      let downloaded = 0;
      let totalLength: number | null = null;
      await update.download((event) => {
        switch (event.event) {
          case 'Started':
            totalLength = event.data.contentLength ?? null;
            onProgress(0, totalLength);
            break;
          case 'Progress':
            downloaded += event.data.chunkLength;
            onProgress(downloaded, totalLength);
            break;
          case 'Finished':
            onProgress(downloaded, downloaded);
            break;
        }
      }, { timeout: DOWNLOAD_TIMEOUT_MS });
      downloadedUpdate = update;
    }

    onInstalling();
    // Refuses while dictation is busy. Otherwise this stops every background service.
    await invokeCommand('prepare_update_install');
    try {
      await update.install();
    } catch (err) {
      console.error('[updater] Install failed, restarting to bring services back:', err);
      await invokeCommand('restart_after_failed_update');
      throw err;
    }
  } finally {
    installInProgress = false;
  }
}

/**
 * Checks shortly after startup and then every six hours, with a one-shot timer rescheduled
 * after each check. Does no work in between.
 */
export function startBackgroundUpdateScheduler(initialDelayMs = 20_000): void {
  if (!isTauri()) return;

  function checkThenSchedule(): void {
    void checkForUpdates(false).finally(() => {
      window.setTimeout(checkThenSchedule, SIX_HOURS_MS);
    });
  }

  window.setTimeout(checkThenSchedule, initialDelayMs);
}
