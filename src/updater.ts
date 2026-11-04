import { check, type Update } from '@tauri-apps/plugin-updater';
import { invokeCommand, isTauri } from './ipc.js';

export interface UpdateInfo {
  version: string;
  currentVersion: string;
  body?: string;
  date?: string;
}

let cachedUpdate: Update | null = null;
let updateCheckInProgress = false;

/**
 * Check for updates.
 * If manual is true, errors are thrown so the UI can report them.
 * If manual is false (background), errors are logged quietly.
 */
export async function checkForUpdates(manual = false): Promise<UpdateInfo | null> {
  if (!isTauri()) return null;
  if (updateCheckInProgress) return null;
  updateCheckInProgress = true;

  try {
    const update = await check();
    if (update?.available) {
      cachedUpdate = update;
      const info: UpdateInfo = {
        version: update.version,
        currentVersion: update.currentVersion,
        body: update.body ?? '',
        date: update.date ?? '',
      };
      try {
        await invokeCommand('set_pending_update', { update: info });
      } catch (e) {
        console.warn('[updater] Failed to update pending update in Rust:', e);
      }
      return info;
    } else {
      cachedUpdate = null;
      try {
        await invokeCommand('set_pending_update', { update: null });
      } catch (e) {
        console.warn('[updater] Failed to reset pending update in Rust:', e);
      }
      return null;
    }
  } catch (err) {
    if (manual) {
      throw err;
    }
    console.debug('[updater] Background check skipped/offline:', err);
    return null;
  } finally {
    updateCheckInProgress = false;
  }
}

/**
 * Get the cached available update info if one was previously detected.
 */
export function getCachedUpdate(): UpdateInfo | null {
  if (!cachedUpdate || !cachedUpdate.available) return null;
  return {
    version: cachedUpdate.version,
    currentVersion: cachedUpdate.currentVersion,
    body: cachedUpdate.body ?? '',
    date: cachedUpdate.date ?? '',
  };
}

/**
 * Download and install the available update with progress tracking.
 * Splits execution into download, Rust shutdown preparation, and install.
 * Never downloads without an explicit user action.
 */
export async function downloadAndInstallUpdate(
  onProgress: (downloaded: number, total: number | null) => void
): Promise<void> {
  if (!isTauri()) throw new Error('Not running in Tauri desktop environment');

  if (!cachedUpdate) {
    const update = await check();
    if (update?.available) {
      cachedUpdate = update;
    } else {
      throw new Error('No update available to install');
    }
  }

  let downloaded = 0;
  let totalLength: number | null = null;

  await cachedUpdate.download((event) => {
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
  });

  // Verify safe exit and run shutdown cleanup in Rust before installation terminates process
  await invokeCommand('prepare_update_install');

  // Proceed with installer launch and process exit
  await cachedUpdate.install();
}

/**
 * Starts the 6-hour background update scheduler with a one-shot setTimeout.
 * Does zero work between checks.
 */
export function startBackgroundUpdateScheduler(initialDelayMs = 20_000): void {
  if (!isTauri()) return;

  const SIX_HOURS_MS = 6 * 60 * 60 * 1000;

  function scheduleNext(): void {
    window.setTimeout(() => {
      checkForUpdates(false)
        .catch(() => {})
        .finally(() => {
          scheduleNext();
        });
    }, SIX_HOURS_MS);
  }

  // Initial check after startup delay
  window.setTimeout(() => {
    checkForUpdates(false)
      .catch(() => {})
      .finally(() => {
        scheduleNext();
      });
  }, initialDelayMs);
}
