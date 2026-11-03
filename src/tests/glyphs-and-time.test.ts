import './setup.js';
import { describe, expect, it } from 'bun:test';
import { batteryGlyph } from '../icons.js';
import { formatClockTime } from '../time-format.js';

describe('battery glyph', () => {
  it('marks low battery only when the level is known', () => {
    expect(batteryGlyph(12, false)).toContain('battery-glyph low');
    expect(batteryGlyph(Number.NaN, false)).toContain('battery-glyph unknown');
    expect(batteryGlyph(50, false, false)).toContain('battery-glyph unknown');
  });

  it('prefers the charging state over low', () => {
    expect(batteryGlyph(5, true)).toContain('battery-glyph charging');
    expect(batteryGlyph(5, true)).toContain('battery-bolt');
  });
});

describe('clock time', () => {
  it('never pads the hour in 12-hour locales', () => {
    const text = formatClockTime(new Date(2026, 10, 3, 7, 33));
    if (/AM|PM/i.test(text)) expect(text).not.toMatch(/^0\d/);
    else expect(text).toContain('07:33');
  });
});
