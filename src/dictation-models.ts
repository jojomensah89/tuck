/** Pure view logic for the speech model picker, so it can be tested without a window. */

export interface ModelRow {
  id: string; label: string; bytes: number; speed: string; accuracy: string; note: string | null;
  state: 'available' | 'downloading' | 'checking' | 'installed' | 'found';
  foundIn: string | null; active: boolean; canRemove?: boolean; memoryNote?: string; memoryWarning?: string | null;
}
export type ModelAction = 'download' | 'cancel' | 'use' | 'remove' | 'stop';
export interface ModelButton { action: ModelAction; label: string; danger?: boolean; primary?: boolean }

export function formatSize(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  return `${Math.round(bytes / 1e6)} MB`;
}

export function rowMeta(row: ModelRow): string {
  const basics = `${formatSize(row.bytes)} · Speed: ${row.speed.toLowerCase()} · Accuracy: ${row.accuracy.toLowerCase()}`;
  return [basics, row.memoryNote, row.note].filter(Boolean).join(' · ');
}

export function rowStateText(row: ModelRow, downloadedBytes: number): string {
  if (row.state === 'downloading') return `Downloading ${formatSize(downloadedBytes)} of ${formatSize(row.bytes)}`;
  if (row.state === 'checking') return 'Checking the file…';
  if (row.state === 'found') return `Found in ${row.foundIn ?? 'another app'}`;
  return '';
}

export function rowProgress(row: ModelRow, downloadedBytes: number): number | null {
  if (row.state !== 'downloading') return null;
  return Math.min(1, Math.max(0, downloadedBytes / row.bytes));
}

/** The buttons a row offers. Removing another app's file only ever stops using it. */
export function rowButtons(row: ModelRow): ModelButton[] {
  switch (row.state) {
    case 'available': return [{ action: 'download', label: 'Download' }];
    case 'downloading': return [{ action: 'cancel', label: 'Cancel' }];
    case 'checking': return [];
    case 'installed': if (row.canRemove === false) return row.active ? [{ action: 'stop', label: 'Stop using' }] : [{ action: 'use', label: 'Use', primary: true }];
    return [
      { action: 'remove', label: 'Remove', danger: true },
      ...(row.active ? [] : [{ action: 'use', label: 'Use', primary: true } as ModelButton]),
    ];
    case 'found': return row.active
      ? [{ action: 'stop', label: 'Stop using' }]
      : [{ action: 'use', label: 'Use', primary: true }];
  }
}

export function removeConfirmation(row: ModelRow): string {
  return row.active
    ? `Delete ${row.label}? Tuck uses another installed model if there is one. Otherwise dictation stops until you choose one.`
    : `Delete ${row.label}? You can download it again later.`;
}

/** Text for the picker's header row. */
export function modelHeader(activeLabel: string | null | undefined): string {
  return activeLabel ? `In use: ${activeLabel}` : 'No model selected. Use an existing model or download one below.';
}

/** Text for the island's Dictation page. */
export function islandModelLine(activeLabel: string | null | undefined): string {
  return activeLabel ? `Model: ${activeLabel}` : 'No model selected';
}

/** Changing the model while a session uses it is refused by the backend too. */
export function modelsLocked(status: string): boolean {
  return ['starting', 'recording', 'transcribing', 'processing', 'cancelling', 'inserting', 'review'].includes(status);
}
