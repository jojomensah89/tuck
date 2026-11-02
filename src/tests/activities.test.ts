import './setup.js';
import { describe, it, expect, beforeEach } from 'bun:test';
import { activitiesManager } from '../activities.js';

describe('Activities Manager & Priority Registry', () => {
  beforeEach(() => {
    localStorage.clear();
    activitiesManager.resetTimer('timer-main');
    activitiesManager.resetStopwatch();
    activitiesManager.resetPomodoro();
  });

  it('starts and manages a countdown timer', () => {
    activitiesManager.startTimer(300, 'Test Timer');
    const top = activitiesManager.getTopActivity();

    expect(top).not.toBeNull();
    expect(top?.type).toBe('timer');
    expect(top?.title).toBe('Test Timer');
    expect(top?.priority).toBe(80);
    expect(top?.active).toBe(true);
    expect(top?.paused).toBe(false);

    // Pause timer
    activitiesManager.pauseTimer('timer-main');
    const pausedTop = activitiesManager.getTopActivity();
    expect(pausedTop?.paused).toBe(true);

    // Format remaining time
    const formatted = activitiesManager.formatRemaining(pausedTop!);
    expect(formatted).toBe('05:00');

    // Resume timer
    activitiesManager.resumeTimer('timer-main');
    expect(activitiesManager.getTopActivity()?.paused).toBe(false);

    // Reset timer
    activitiesManager.resetTimer('timer-main');
    expect(activitiesManager.getTopActivity()).toBeNull();
  });

  it('prioritizes active timers above lower-priority activities', () => {
    // Start stopwatch (priority 60)
    activitiesManager.startStopwatch();
    expect(activitiesManager.getTopActivity()?.type).toBe('stopwatch');

    // Start timer (priority 80) -> should win over stopwatch
    activitiesManager.startTimer(600, 'Important');
    expect(activitiesManager.getTopActivity()?.type).toBe('timer');

    // Start pomodoro (priority 75)
    activitiesManager.startPomodoro(25);
    // Timer (80) still beats pomodoro (75)
    expect(activitiesManager.getTopActivity()?.type).toBe('timer');

    // Reset timer -> Pomodoro (75) becomes top
    activitiesManager.resetTimer('timer-main');
    expect(activitiesManager.getTopActivity()?.type).toBe('pomodoro');
  });

  it('generates compact view data with appropriate icons', () => {
    activitiesManager.startTimer(120, 'Quick Timer');
    const compact = activitiesManager.getCompactView();

    expect(compact).not.toBeNull();
    expect(compact?.centerText).toBe('Quick Timer');
    expect(compact?.rightHtml).toContain('02:00');
    expect(compact?.leftHtml).toContain('act-compact-icon');
  });
});
