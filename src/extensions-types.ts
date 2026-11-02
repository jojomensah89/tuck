/**
 * Tuck v1 Native Extensions & Feature Types.
 * 
 * Defines core contracts for:
 * 1. Live activity downloads monitor (opt-in only, indeterminate total, never fabricated %).
 * 2. Manual calls activity (highest priority 95, real elapsed counter, no auto-detection claims).
 * 3. Local ICS calendar import, 5-minute meeting reminders & strictly validated http(s) join links.
 * 4. WinRT UserNotificationListener opt-in access state (acknowledging MSIX constraints).
 * 5. File shelf with allowlisted UUIDs & native open (no deleting underlying files).
 * 6. Real WinRT Radios (WiFi / Bluetooth) & Focus Assist official settings action.
 */

/* ========================================================================= */
/* Extension Opt-in Settings                                                 */
/* ========================================================================= */

export interface ExtensionSettings {
  /** Opt-in only: monitor user's Downloads directory for active downloads (.crdownload / .part) */
  downloadsMonitorEnabled: boolean;
  /** Opt-in only: enable local ICS calendar parsing and 5-minute meeting reminders */
  calendarEnabled: boolean;
  /** Opt-in only: WinRT UserNotificationListener access request */
  notificationsEnabled: boolean;
  /** Opt-in only: monitor and control WiFi / Bluetooth radios */
  radioMonitorEnabled: boolean;
  /** Reminder notification threshold before meeting start (default 5 minutes) */
  calendarReminderMinutes: number;
}

export const DEFAULT_EXTENSION_SETTINGS: Readonly<ExtensionSettings> = Object.freeze({
  downloadsMonitorEnabled: false,
  calendarEnabled: false,
  notificationsEnabled: false,
  radioMonitorEnabled: false,
  calendarReminderMinutes: 5,
});

/* ========================================================================= */
/* 1. Live Activity Downloads Monitor                                        */
/* ========================================================================= */

export interface ActiveDownloadItem {
  id: string;
  fileName: string;
  bytesDownloaded: number;
  downloadRateBps: number;
  /**
   * Strictly indeterminate for active .crdownload / .part files.
   * Total bytes MUST be null (never fabricate a false percentage!).
   */
  totalBytes: null;
  status: 'downloading' | 'paused' | 'completed';
  lastUpdatedMs: number;
}

export interface DownloadsState {
  enabled: boolean;
  downloads: ActiveDownloadItem[];
  activeCount: number;
  aggregateRateBps: number;
}

/* ========================================================================= */
/* 2. Manual Calls Activity                                                  */
/* ========================================================================= */

export interface CallState {
  active: boolean;
  title: string;
  startTimeMs: number | null;
  elapsedMs: number;
  muted: boolean;
  /** Highest priority in the Tuck dynamic island (95) */
  priority: number;
}

export const DEFAULT_CALL_STATE: Readonly<CallState> = Object.freeze({
  active: false,
  title: 'Call in Progress',
  startTimeMs: null,
  elapsedMs: 0,
  muted: false,
  priority: 95,
});

/* ========================================================================= */
/* 3. Calendar & Meeting Reminders (Local ICS)                              */
/* ========================================================================= */

export interface CalendarEvent {
  id: string;
  uid: string;
  summary: string;
  description: string;
  location: string;
  startTimeMs: number;
  endTimeMs: number;
  /** Strictly validated http:// or https:// URL */
  joinUrl: string | null;
  isAllDay: boolean;
}

export interface CalendarReminderPayload {
  eventId: string;
  title: string;
  startTimeMs: number;
  minutesUntil: number;
  joinUrl: string | null;
  location: string;
}

/* ========================================================================= */
/* 4. Windows Notifications (WinRT UserNotificationListener)                 */
/* ========================================================================= */

export type NotificationAccessStatus =
  | 'Allowed'
  | 'DeniedBySystem'
  | 'DeniedByUser'
  | 'Unspecified'
  | 'Unavailable';

export interface NotificationListenerState {
  enabled: boolean;
  accessStatus: NotificationAccessStatus;
  available: boolean;
  reason: string;
  activeNotificationsCount: number;
}

/* ========================================================================= */
/* 5. File Shelf                                                             */
/* ========================================================================= */

export interface ShelfItem {
  /** Internal allowlisted UUID - launching is ONLY permitted via known ID */
  id: string;
  fileName: string;
  filePath: string;
  fileSizeBytes: number;
  fileExtension: string;
  addedAtMs: number;
}

/* ========================================================================= */
/* 6. Hardware Radios & Quick System Actions                                 */
/* ========================================================================= */

export type RadioState = 'On' | 'Off' | 'Disabled' | 'Unknown';
export type RadioKind = 'WiFi' | 'Bluetooth' | 'MobileBroadband' | 'Other';

export interface RadioInfo {
  id: string;
  name: string;
  kind: RadioKind;
  state: RadioState;
  error?: string;
}

export interface RadiosPayload {
  wifi: RadioInfo | null;
  bluetooth: RadioInfo | null;
  accessAllowed: boolean;
  error?: string;
}

/* ========================================================================= */
/* Full Combined Extension State DTO                                         */
/* ========================================================================= */

export interface ExtensionStateDTO {
  settings: ExtensionSettings;
  downloads: DownloadsState;
  call: CallState;
  upcoming_event: CalendarEvent | null;
  active_reminder: CalendarReminderPayload | null;
  notifications: NotificationListenerState;
  shelf_items: ShelfItem[];
  radios: RadiosPayload;
}

/* ========================================================================= */
/* Compact Island Activity Integration Event                                 */
/* ========================================================================= */

export interface TuckActivityEventDetail {
  id: string;
  type: 'call' | 'download' | 'meeting' | 'shelf' | 'radio';
  title: string;
  text: string;
  priority: number;
  active: boolean;
}

/* ========================================================================= */
/* Pure Validation & Formatting Utilities                                    */
/* ========================================================================= */

/**
 * Validates that a string is strictly an http:// or https:// URL.
 * Strictly rejects javascript:, file:, data:, or malformed protocols.
 */
export function validateHttpUrl(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;

  // Strip wrapping quotes or angles if present
  const clean = trimmed.replace(/^[<"']+|[>"']+$/g, '');
  const lower = clean.toLowerCase();

  if (!lower.startsWith('http://') && !lower.startsWith('https://')) {
    return null;
  }

  try {
    const url = new URL(clean);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') {
      return null;
    }
    if (!url.hostname || url.hostname.includes(' ')) {
      return null;
    }
    return url.href;
  } catch {
    return null;
  }
}

/**
 * Formats bytes into a human-readable string (KB, MB, GB).
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const clamped = Math.min(i, units.length - 1);
  const val = (bytes / Math.pow(1024, clamped)).toFixed(clamped === 0 ? 0 : 1);
  return `${val} ${units[clamped]}`;
}

/**
 * Formats transfer rate into bytes/sec string.
 */
export function formatRate(bps: number): string {
  if (!Number.isFinite(bps) || bps <= 0) return '0 KB/s';
  return `${formatBytes(bps)}/s`;
}

/**
 * Formats elapsed milliseconds into mm:ss or hh:mm:ss.
 */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '00:00';
  const totalSeconds = Math.floor(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  const pad = (n: number) => n.toString().padStart(2, '0');
  if (hours > 0) {
    return `${pad(hours)}:${pad(minutes)}:${pad(seconds)}`;
  }
  return `${pad(minutes)}:${pad(seconds)}`;
}
