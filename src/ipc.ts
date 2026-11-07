import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen as tauriListen, type UnlistenFn } from '@tauri-apps/api/event';
import type { Meter } from './types.js';

export type { MediaState, PowerState, Meter } from './types.js';

/**
 * Checks whether the application is running inside a Tauri webview.
 */
export function isTauri(): boolean {
  if (typeof window === 'undefined') return false;
  return Boolean(
    (window as unknown as { __TAURI__?: unknown }).__TAURI__ ||
      (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
  );
}

// In-browser mock event bus for testing & browser demo mode
type EventHandler<T> = (payload: T) => void;
const browserListeners: Record<string, Set<EventHandler<unknown>>> = {};

/**
 * Dispatches an event locally (used in browser demo mode & tests).
 */
export function dispatchMockEvent<T>(eventName: string, payload: T): void {
  const set = browserListeners[eventName];
  if (set) {
    set.forEach((handler) => {
      try {
        handler(payload);
      } catch (err) {
        console.error(`Error in mock listener for ${eventName}:`, err);
      }
    });
  }
}

/**
 * Subscribes to a native or mock event.
 */
export async function listenToEvent<T>(
  eventName: string,
  handler: (payload: T) => void
): Promise<UnlistenFn> {
  if (isTauri()) {
    try {
      return await tauriListen<T>(eventName, (event) => handler(event.payload));
    } catch (err) {
      console.warn(`[IPC] Failed to attach Tauri listener for ${eventName}:`, err);
    }
  }

  if (!browserListeners[eventName]) {
    browserListeners[eventName] = new Set();
  }
  browserListeners[eventName].add(handler as EventHandler<unknown>);

  return () => {
    browserListeners[eventName]?.delete(handler as EventHandler<unknown>);
  };
}

/**
 * Generic invoke wrapper that safely falls back when in a browser environment.
 */
export async function invokeCommand<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri()) {
    return await tauriInvoke<T>(cmd, args);
  }
  // Safe mock in browser
  return undefined as unknown as T;
}

/* ================= Specific IPC Commands ================= */

export async function mediaToggle(): Promise<void> {
  await invokeCommand('media_toggle');
}

export async function mediaNext(): Promise<void> {
  await invokeCommand('media_next');
}

export async function mediaPrev(): Promise<void> {
  await invokeCommand('media_prev');
}

export async function mediaSeek(ms: number): Promise<void> {
  await invokeCommand('media_seek', { ms: Math.max(0, Math.round(ms)) });
}

/**
 * Reports current island hit geometry to Rust.
 * Per the guide: hit height must include top offset (`h + top`).
 */
export async function setHit(w: number, h: number): Promise<void> {
  await invokeCommand('set_hit', { w: Math.round(w), h: Math.round(h) });
}

export async function fetchUsageNow(): Promise<Meter[]> {
  if (isTauri()) {
    try {
      const res = await invokeCommand<Meter[]>('usage_now');
      return Array.isArray(res) ? res : [];
    } catch (err) {
      console.warn('[IPC] usage_now error:', err);
      throw err;
    }
  }
  return [];
}

export async function openSettingsCmd(view?: 'tools'): Promise<void> {
  if (isTauri()) {
    await invokeCommand('open_settings_cmd', view ? { view } : undefined);
  } else {
    // In browser demo, open settings.html in a new tab/window
    window.open(view ? `settings.html#${view}` : 'settings.html', '_blank', 'width=460,height=700');
  }
}

/**
 * Native screen snip action; never navigates the webview.
 */
export async function triggerQuickAction(actionName: string): Promise<void> {
  if (!['capture'].includes(actionName)) throw new Error('Unknown action');
  if (!isTauri()) throw new Error('Windows actions are available in the desktop app');
  await invokeCommand('quick_action', { name: actionName });
}
