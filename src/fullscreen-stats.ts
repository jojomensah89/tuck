import type { TelemetrySnapshot } from './live-services.js';

/** Wide enough for three readings at 100% with room to spare */
export const FULLSCREEN_STATS_WIDTH = 236;

export type FullscreenStatsIslandState = 'idle' | 'peek' | 'open';

export interface FullscreenStatsInput {
  /** Settings > Island > Stats over fullscreen apps */
  enabled: boolean;
  /** Native reports a fullscreen app in front (already false when "Hide in fullscreen" is off) */
  fullscreen: boolean;
  state: FullscreenStatsIslandState;
}

/** The strip replaces the collapsed island only; a peek or open island stays hidden as before. */
export function fullscreenStatsVisible({ enabled, fullscreen, state }: FullscreenStatsInput): boolean {
  return enabled && fullscreen && state === 'idle';
}

export interface FullscreenStat { label: 'CPU' | 'GPU' | 'RAM'; text: string; spoken: string }

const percent = (value: number | null | undefined): number | null =>
  typeof value === 'number' && Number.isFinite(value) ? Math.round(Math.max(0, Math.min(100, value))) : null;

/** Busiest adapter, so a laptop's idle integrated GPU does not hide the one running the game */
function gpuPercent(snapshot: TelemetrySnapshot): number | null {
  let busiest: number | null = null;
  for (const gpu of snapshot.gpus) {
    const value = percent(gpu.usedPct);
    if (value !== null && (busiest === null || value > busiest)) busiest = value;
  }
  return busiest;
}

function ramPercent(snapshot: TelemetrySnapshot): number | null {
  const { ramUsedBytes: used, ramTotalBytes: total } = snapshot;
  if (typeof used !== 'number' || typeof total !== 'number' || !(total > 0) || used < 0) return null;
  return percent((used / total) * 100);
}

const cell = (label: FullscreenStat['label'], spoken: string, value: number | null): FullscreenStat => ({
  label,
  text: value === null ? '–' : `${value}%`,
  spoken: value === null ? `${spoken} unavailable` : `${spoken} ${value}%`,
});

/** CPU, GPU and RAM for the strip; a missing reading shows an en dash instead of a stale number. */
export function fullscreenStats(snapshot: TelemetrySnapshot | null): FullscreenStat[] {
  if (!snapshot || snapshot.stale) return [cell('CPU', 'CPU', null), cell('GPU', 'GPU', null), cell('RAM', 'Memory', null)];
  return [
    cell('CPU', 'CPU', percent(snapshot.cpuPct)),
    cell('GPU', 'GPU', gpuPercent(snapshot)),
    cell('RAM', 'Memory', ramPercent(snapshot)),
  ];
}

export const FULLSCREEN_STATS_INTERVAL_MS = 1000;

export interface FullscreenStatsSamplerOptions {
  read: () => Promise<TelemetrySnapshot>;
  onSample: (snapshot: TelemetrySnapshot | null) => void;
  intervalMs?: number;
  now?: () => number;
  setTimer?: (callback: () => void, ms: number) => unknown;
  clearTimer?: (handle: unknown) => void;
}

/**
 * Samples telemetry while the strip is visible. Requests never overlap and start at least
 * `intervalMs` apart; `stop` ends the loop at once and discards a reply still in flight.
 */
export class FullscreenStatsSampler {
  private running = false;
  private generation = 0;
  private handle: unknown = null;
  private lastStart = Number.NEGATIVE_INFINITY;
  private readonly intervalMs: number;
  private readonly now: () => number;
  private readonly setTimer: (callback: () => void, ms: number) => unknown;
  private readonly clearTimer: (handle: unknown) => void;

  constructor(private readonly options: FullscreenStatsSamplerOptions) {
    this.intervalMs = options.intervalMs ?? FULLSCREEN_STATS_INTERVAL_MS;
    this.now = options.now ?? (() => Date.now());
    this.setTimer = options.setTimer ?? ((callback, ms) => globalThis.setTimeout(callback, ms));
    this.clearTimer = options.clearTimer ?? (handle => globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>));
  }

  get active(): boolean { return this.running; }

  start(): void {
    if (this.running) return;
    this.running = true;
    const generation = ++this.generation;
    this.schedule(generation, Math.max(0, this.lastStart + this.intervalMs - this.now()));
  }

  stop(): void {
    this.running = false;
    this.generation++;
    if (this.handle !== null) this.clearTimer(this.handle);
    this.handle = null;
  }

  private schedule(generation: number, delay: number): void {
    this.handle = this.setTimer(() => { void this.sample(generation); }, delay);
  }

  private async sample(generation: number): Promise<void> {
    this.handle = null;
    if (!this.running || generation !== this.generation) return;
    this.lastStart = this.now();
    let snapshot: TelemetrySnapshot | null = null;
    try { snapshot = await this.options.read(); } catch { snapshot = null; }
    if (!this.running || generation !== this.generation) return;
    this.options.onSample(snapshot);
    this.schedule(generation, Math.max(0, this.lastStart + this.intervalMs - this.now()));
  }
}
