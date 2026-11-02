import { invokeCommand, isTauri } from './ipc.js';

/** Only desktop services may resolve native actions. Browser previews never fake them. */
export async function nativeService<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error('Open Tuck on Windows to use this feature.');
  const result = await invokeCommand<T>(command, args);
  if (result === null || result === undefined) throw new Error('This service is not available in this build.');
  return result;
}
export async function nativeAction(command: string, args?: Record<string, unknown>): Promise<void> {
  if (!isTauri()) throw new Error('Open Tuck on Windows to use this feature.');
  await invokeCommand(command, args);
}
export interface MediaSession { id: string; name: string; selected: boolean }
export interface LyricsSnapshot {
  trackId: string; status: 'disabled' | 'loading' | 'synced' | 'plain' | 'instrumental' | 'unavailable' | 'error';
  syncedLyrics?: string | null; plainLyrics?: string | null; error?: string | null;
}
export interface TelemetrySnapshot {
  sampledAt: number; cpuPct: number | null; ramUsedBytes: number | null; ramTotalBytes: number | null;
  gpus: { id: string; name: string; usedPct: number | null; dedicatedUsedBytes?: number | null; dedicatedTotalBytes?: number | null }[];
  stale?: boolean;
}
export interface DictationSnapshot {
  status: 'disabled' | 'idle' | 'recording' | 'transcribing' | 'review' | 'error';
  transcript: string; error?: string | null; modelReady?: boolean;
}
export interface ClipboardItem { id: string; text: string; pinned: boolean; createdAt: number }
export interface ClipboardSnapshot { enabled: boolean; items: ClipboardItem[] }
