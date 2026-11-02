import type { Activity, CompactViewData } from './types.js';
import { SVG_ICONS } from './icons.js';

type ActivityListener = (activities: Activity[], topActivity: Activity | null) => void;

class ActivitiesRegistry {
  private activities: Map<string, Activity> = new Map();
  private listeners: Set<ActivityListener> = new Set();
  private ticker: ReturnType<typeof setInterval> | null = null;
  private audioCtx: AudioContext | null = null;

  constructor() {
    this.loadState();
    this.startTicker();
  }

  private loadState(): void {
    try {
      const saved = localStorage.getItem('tuck.activities');
      if (saved) {
        const parsed: Activity[] = JSON.parse(saved);
        parsed.forEach((act) => {
          // If timer expired while app was closed, handle it
          if (act.type === 'timer' || act.type === 'pomodoro') {
            if (act.active && !act.paused && act.targetEndTime) {
              if (Date.now() >= act.targetEndTime) {
                act.active = false;
                act.remainingMs = 0;
              }
            }
          }
          this.activities.set(act.id, act);
        });
      }
    } catch {
      // Ignore parse errors
    }
  }

  private saveState(): void {
    try {
      const list = Array.from(this.activities.values());
      localStorage.setItem('tuck.activities', JSON.stringify(list));
    } catch {
      // Ignore quota errors
    }
  }

  public subscribe(listener: ActivityListener): () => void {
    this.listeners.add(listener);
    listener(this.getAll(), this.getTopActivity());
    return () => {
      this.listeners.delete(listener);
    };
  }

  private notify(): void {
    const all = this.getAll();
    const top = this.getTopActivity();
    this.listeners.forEach((fn) => fn(all, top));
    this.saveState();
  }

  public getAll(): Activity[] {
    return Array.from(this.activities.values());
  }

  public getTopActivity(): Activity | null {
    let top: Activity | null = null;
    let highestPriority = -1;

    for (const act of this.activities.values()) {
      if (!act.active) continue;
      const effectivePriority = act.paused ? act.priority - 30 : act.priority;
      if (effectivePriority > highestPriority) {
        highestPriority = effectivePriority;
        top = act;
      }
    }

    return top;
  }

  private startTicker(): void {
    if (this.ticker) return;
    this.ticker = setInterval(() => {
      let changed = false;
      const now = Date.now();

      for (const act of this.activities.values()) {
        if (!act.active || act.paused) continue;

        if (act.type === 'timer') {
          if (act.targetEndTime && now >= act.targetEndTime) {
            act.active = false;
            act.remainingMs = 0;
            this.playChime();
            changed = true;
          }
        } else if (act.type === 'pomodoro') {
          if (act.targetEndTime && now >= act.targetEndTime) {
            this.playChime();
            if (act.pomodoroPhase === 'work') {
              // Switch to break (5 minutes)
              act.pomodoroPhase = 'break';
              act.totalDurationMs = 5 * 60 * 1000;
              act.targetEndTime = now + act.totalDurationMs;
              act.title = 'Pomodoro: Break';
            } else {
              // Switch back to work (25 minutes)
              act.pomodoroPhase = 'work';
              act.totalDurationMs = 25 * 60 * 1000;
              act.targetEndTime = now + act.totalDurationMs;
              act.title = 'Pomodoro: Focus';
            }
            changed = true;
          }
        }
      }

      if (changed || this.hasRunningActivities()) {
        this.notify();
      }
    }, 250);
  }

  private hasRunningActivities(): boolean {
    for (const act of this.activities.values()) {
      if (act.active && !act.paused) return true;
    }
    return false;
  }

  public playChime(): void {
    try {
      if (!this.audioCtx) {
        const AudioClass = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
        if (AudioClass) this.audioCtx = new AudioClass();
      }
      if (this.audioCtx && this.audioCtx.state === 'suspended') {
        void this.audioCtx.resume();
      }
      if (!this.audioCtx) return;

      const ctx = this.audioCtx;
      const now = ctx.currentTime;

      // Two-tone bell (D5: 587Hz -> A5: 880Hz)
      const osc1 = ctx.createOscillator();
      const gain1 = ctx.createGain();
      osc1.type = 'sine';
      osc1.frequency.setValueAtTime(587.33, now);
      gain1.gain.setValueAtTime(0.2, now);
      gain1.gain.exponentialRampToValueAtTime(0.001, now + 0.5);
      osc1.connect(gain1);
      gain1.connect(ctx.destination);
      osc1.start(now);
      osc1.stop(now + 0.5);

      const osc2 = ctx.createOscillator();
      const gain2 = ctx.createGain();
      osc2.type = 'sine';
      osc2.frequency.setValueAtTime(880, now + 0.2);
      gain2.gain.setValueAtTime(0.25, now + 0.2);
      gain2.gain.exponentialRampToValueAtTime(0.001, now + 0.8);
      osc2.connect(gain2);
      gain2.connect(ctx.destination);
      osc2.start(now + 0.2);
      osc2.stop(now + 0.8);
    } catch {
      // Audio playback best effort
    }
  }

  /* ================= Actions ================= */

  public startTimer(seconds: number, label = 'Timer'): void {
    const id = 'timer-main';
    const totalMs = seconds * 1000;
    const act: Activity = {
      id,
      type: 'timer',
      title: label,
      priority: 80,
      active: true,
      paused: false,
      totalDurationMs: totalMs,
      targetEndTime: Date.now() + totalMs,
      remainingMs: totalMs,
    };
    this.activities.set(id, act);
    this.notify();
  }

  public pauseTimer(id = 'timer-main'): void {
    const act = this.activities.get(id);
    if (!act || !act.active || act.paused) return;

    if (act.targetEndTime) {
      act.remainingMs = Math.max(0, act.targetEndTime - Date.now());
    }
    act.paused = true;
    this.notify();
  }

  public resumeTimer(id = 'timer-main'): void {
    const act = this.activities.get(id);
    if (!act || !act.active || !act.paused) return;

    act.paused = false;
    act.targetEndTime = Date.now() + (act.remainingMs || 0);
    this.notify();
  }

  public resetTimer(id = 'timer-main'): void {
    this.activities.delete(id);
    this.notify();
  }

  public startStopwatch(): void {
    const id = 'stopwatch-main';
    const existing = this.activities.get(id);

    if (existing && existing.active && existing.paused) {
      existing.paused = false;
      existing.startTime = Date.now() - (existing.accumulatedMs || 0);
      this.notify();
      return;
    }

    const act: Activity = {
      id,
      type: 'stopwatch',
      title: 'Stopwatch',
      priority: 60,
      active: true,
      paused: false,
      startTime: Date.now(),
      accumulatedMs: 0,
    };
    this.activities.set(id, act);
    this.notify();
  }

  public pauseStopwatch(): void {
    const act = this.activities.get('stopwatch-main');
    if (!act || !act.active || act.paused) return;

    if (act.startTime) {
      act.accumulatedMs = Date.now() - act.startTime;
    }
    act.paused = true;
    this.notify();
  }

  public resetStopwatch(): void {
    this.activities.delete('stopwatch-main');
    this.notify();
  }

  public startPomodoro(workMinutes = 25): void {
    const id = 'pomodoro-main';
    const totalMs = workMinutes * 60 * 1000;
    const act: Activity = {
      id,
      type: 'pomodoro',
      title: 'Pomodoro: Focus',
      priority: 75,
      active: true,
      paused: false,
      pomodoroPhase: 'work',
      totalDurationMs: totalMs,
      targetEndTime: Date.now() + totalMs,
      remainingMs: totalMs,
    };
    this.activities.set(id, act);
    this.notify();
  }

  public pausePomodoro(): void {
    this.pauseTimer('pomodoro-main');
  }

  public resumePomodoro(): void {
    this.resumeTimer('pomodoro-main');
  }

  public resetPomodoro(): void {
    this.activities.delete('pomodoro-main');
    this.notify();
  }

  /* ================= Formatting & Compact Presentation ================= */

  public formatRemaining(act: Activity): string {
    let ms = 0;
    if (act.type === 'stopwatch') {
      if (act.paused) {
        ms = act.accumulatedMs || 0;
      } else if (act.startTime) {
        ms = Date.now() - act.startTime;
      }
      const totalSeconds = Math.floor(ms / 1000);
      const minutes = Math.floor(totalSeconds / 60);
      const seconds = totalSeconds % 60;
      return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
    }

    if (act.paused) {
      ms = act.remainingMs || 0;
    } else if (act.targetEndTime) {
      ms = Math.max(0, act.targetEndTime - Date.now());
    }

    // Ceil so 299.9s displays as 05:00
    const totalSeconds = Math.ceil(ms / 1000);
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
  }

  public getCompactView(): CompactViewData | null {
    const top = this.getTopActivity();
    if (!top) return null;

    const timeStr = this.formatRemaining(top);
    let iconHtml = SVG_ICONS.timer;
    if (top.type === 'stopwatch') iconHtml = SVG_ICONS.stopwatch;
    if (top.type === 'pomodoro') iconHtml = SVG_ICONS.pomodoro;

    return {
      leftHtml: `<span class="act-compact-icon ${top.paused ? 'paused' : 'pulsing'}">${iconHtml}</span>`,
      centerText: top.title,
      rightHtml: `<span class="act-compact-time">${timeStr}</span>`,
      priority: top.paused ? top.priority - 30 : top.priority,
    };
  }
}

export const activitiesManager = new ActivitiesRegistry();
