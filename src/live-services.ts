import { invokeCommand, isTauri } from './ipc.js';

/** Only desktop services may resolve native actions. Browser previews never fake them. */
export async function nativeService<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error('Open Tuck on Windows to use this feature.');
  const names: Record<string, string> = { lyrics_settings: 'lyrics_status', visualizer_settings: 'visualizer_status', telemetry_snapshot: 'telemetry_now', lyrics_lookup: 'lyrics_get', dictation_snapshot: 'dictation_get_state', dictation_settings: 'dictation_get_state', clipboard_snapshot: 'clipboard_get_state', clipboard_settings: 'clipboard_get_state' };
  let result: unknown = await invokeCommand(names[command] || command, args);
  if (result === null || result === undefined) throw new Error('This service is not available in this build.');
  if (command === 'telemetry_snapshot') result = normalizeTelemetry(result);
  if (command === 'dictation_snapshot') result = normalizeDictation(result);
  if (command === 'dictation_settings') {
    const data = result as NativeDictation;
    result = { ...data.settings, modelReady: data.modelInstalled };
  }
  if (command === 'clipboard_snapshot' || command === 'clipboard_settings') result = normalizeClipboard(result);
  if (command === 'media_sessions') result = (result as {id: string; sourceAppId: string; isSelected: boolean}[]).map(session => ({ id: session.id, name: session.sourceAppId, selected: session.isSelected }));
  if (command === 'lyrics_lookup') {
    const data = result as { status: string; trackId: string; syncedLyrics: {timeMs: number; text: string}[] | null; plainLyrics: string | null };
    result = { ...data, status: data.status === 'not_found' ? 'unavailable' : data.status, requestId: args?.requestId,
      syncedLyrics: null, lines: data.syncedLyrics?.map(line => ({ atMs: line.timeMs, text: line.text })) || [] };
  }
  return result as T;
}
export async function nativeAction(command: string, args?: Record<string, unknown>): Promise<void> {
  if (!isTauri()) throw new Error('Open Tuck on Windows to use this feature.');
  if (command === 'media_select_session') args = { sessionId: args?.id || null };
  if (command === 'clipboard_set_settings') {
    const settings = args?.settings as { enabled: boolean; persist: boolean };
    await invokeCommand('clipboard_set_persistence', { persistOnExit: settings.persist });
    await invokeCommand('clipboard_set_enabled', { enabled: settings.enabled }); return;
  }
  const names: Record<string, string> = { dictation_stop: 'dictation_hold_stop', dictation_insert: 'dictation_review_insert', dictation_download_model: 'dictation_model_download', clipboard_remove: 'clipboard_delete' };
  await invokeCommand(names[command] || command, args);
}
export interface MediaSession { id: string; name: string; selected: boolean }
export interface LyricsSnapshot {
  trackId: string; requestId: string; status: 'disabled' | 'loading' | 'synced' | 'plain' | 'instrumental' | 'unavailable' | 'error';
  syncedLyrics?: string | null; plainLyrics?: string | null; error?: string | null;
  lines?: { atMs: number; text: string }[];
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

interface NativeDictation { status: string; transcript: string | null; message: string | null; modelInstalled: boolean; modelDownloadedBytes: number; settings: { toggleShortcut: string; holdToTalkShortcut: string; reviewBeforeInsert: boolean; partialPreview: boolean } }
export function normalizeDictation(value: unknown): DictationSnapshot {
  const data = value as NativeDictation & DictationSnapshot;
  return { ...data, transcript: data.transcript || '', error: data.message || data.error, modelReady: data.modelInstalled ?? data.modelReady };
}
export function normalizeTelemetry(value: unknown): TelemetrySnapshot {
  if (value && typeof value === 'object' && 'sampledAt' in value) return value as TelemetrySnapshot;
  const data = value as { sampleTimeMs: number; status: string; cpu: {usagePercent: number} | null; ram: {usedBytes: number; totalBytes: number} | null; gpus: { name: string; usagePercent: number | null }[] };
  return { sampledAt: data.sampleTimeMs, cpuPct: data.cpu?.usagePercent ?? null, ramUsedBytes: data.ram?.usedBytes ?? null, ramTotalBytes: data.ram?.totalBytes ?? null,
    stale: data.status === 'stale' || data.status === 'unavailable',
    gpus: (data.gpus || []).map((gpu, index) => ({ id: `${index}:${gpu.name}`, name: gpu.name, usedPct: gpu.usagePercent ?? null })) };
}
export function normalizeClipboard(value: unknown): ClipboardSnapshot & { persist: boolean } {
  const data = value as { enabled: boolean; persistOnExit: boolean; items: (ClipboardItem & {createdAtMs: number})[] };
  return { enabled: data.enabled, persist: data.persistOnExit, items: (data.items || []).map(item => ({ ...item, createdAt: item.createdAtMs ?? item.createdAt })) };
}
