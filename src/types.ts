export type IslandState = 'idle' | 'peek' | 'open';
export type ShapeStyle = 'notch' | 'float';

export interface MediaState {
  active: boolean;
  trackId: string;
  title: string;
  artist: string;
  playing: boolean;
  positionMs: number;
  durationMs: number;
  artwork?: string | null;
  canPlayPause?: boolean;
  canSkipNext?: boolean;
  canSkipPrev?: boolean;
  canSeek?: boolean;
}

export interface PowerState {
  percentKnown?: boolean;
  statusKnown?: boolean;
  acConnected?: boolean | null;
  percent: number;
  charging: boolean;
  hasBattery: boolean;
  minutesLeft: number | null;
}

export interface Meter {
  id: string;
  provider: string; // Subscription provider
  label: string;
  usedPct: number | null; // 0..100
  spentUsd: number | null; // Reserved for native compatibility; always null
  resetsAt: number | null; // unix seconds
  source: string; // Subscription quota provenance
  stale: boolean;
  status?: string;
  error?: string;
}



export type ActivityType = 'timer' | 'stopwatch' | 'pomodoro';

export interface Activity {
  id: string;
  type: ActivityType;
  title: string;
  priority: number; // 0..100
  active: boolean;
  paused: boolean;
  totalDurationMs?: number;
  targetEndTime?: number; // timestamp in ms when timer ends
  remainingMs?: number; // remaining ms when paused
  startTime?: number; // timestamp in ms when stopwatch started
  accumulatedMs?: number; // elapsed ms for stopwatch
  pomodoroPhase?: 'work' | 'break';
}

export interface CompactViewData {
  leftHtml: string;
  centerText: string;
  rightHtml: string;
  priority: number;
}
