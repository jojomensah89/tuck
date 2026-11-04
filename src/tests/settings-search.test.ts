import { describe, expect, test } from 'bun:test';
import { describeResult, filterSettings, matchesTokens, navTarget, normalizeText, queryTokens } from '../settings-search';

const entries = [
  { id: 'autostart', view: 'general', text: 'General Startup Launch at sign-in Start Tuck when you sign in to Windows' },
  { id: 'openDelay', view: 'island', text: 'Island Opening Expand delay Wait before the island opens milliseconds hover' },
  { id: 'closeDelay', view: 'island', text: 'Island Opening Collapse delay After the pointer leaves milliseconds' },
  { id: 'onlineLyrics', view: 'media', text: 'Media and lyrics Look up lyrics with LRCLIB online privacy' },
  { id: 'kAnthropic', view: 'keys', text: 'API keys Anthropic admin key Credential Manager secret' },
  { id: 'kOpenai', view: 'keys', text: 'API keys OpenAI admin key Credential Manager secret' },
];

describe('settings search', () => {
  test('normalizes case, accents and punctuation', () => {
    expect(normalizeText('  Café — Sign-in!  ')).toBe('cafe sign in');
    expect(queryTokens('API  api Key')).toEqual(['api', 'key']);
    expect(queryTokens('   ')).toEqual([]);
  });

  test('tokens match word prefixes, not word middles', () => {
    expect(matchesTokens('Expand delay', ['del'])).toBe(true);
    expect(matchesTokens('Expand delay', ['lay'])).toBe(false);
    expect(matchesTokens('Expand delay', [])).toBe(true);
  });

  test('every token must match', () => {
    const result = filterSettings(entries, 'anthropic key');
    expect(result.ids).toEqual(['kAnthropic']);
    expect(filterSettings(entries, 'openai lyrics').ids).toEqual([]);
  });

  test('counts matches per view in document order', () => {
    const result = filterSettings(entries, 'delay');
    expect(result.ids).toEqual(['openDelay', 'closeDelay']);
    expect([...result.views]).toEqual([['island', 2]]);
    expect(filterSettings(entries, 'key').views.get('keys')).toBe(2);
  });

  test('a blank query keeps every row', () => {
    const result = filterSettings(entries, '');
    expect(result.ids.length).toBe(entries.length);
    expect(describeResult(result, '')).toBe('');
  });

  test('describes the result for the status line', () => {
    expect(describeResult(filterSettings(entries, 'lrclib'), 'lrclib')).toBe('1 setting matches "lrclib".');
    expect(describeResult(filterSettings(entries, 'delay'), ' delay ')).toBe('2 settings match "delay".');
    expect(describeResult(filterSettings(entries, 'zzz'), 'zzz')).toBe('No settings match "zzz".');
  });
});

describe('sidebar keys', () => {
  test('arrows wrap and Home/End jump', () => {
    expect(navTarget(0, 'ArrowDown', 3)).toBe(1);
    expect(navTarget(2, 'ArrowDown', 3)).toBe(0);
    expect(navTarget(0, 'ArrowUp', 3)).toBe(2);
    expect(navTarget(1, 'Home', 3)).toBe(0);
    expect(navTarget(1, 'End', 3)).toBe(2);
  });

  test('other keys and empty lists do nothing', () => {
    expect(navTarget(1, 'Enter', 3)).toBeNull();
    expect(navTarget(0, 'ArrowDown', 0)).toBeNull();
    expect(navTarget(-1, 'ArrowDown', 3)).toBe(1);
  });
});

describe('update availability and search visibility', () => {
  function computeRowHidden(id: string, isAvailable: boolean, searching: boolean, matched: Set<string>): boolean {
    if (id === 'row-update-available') {
      return !isAvailable || (searching && !matched.has(id));
    }
    return searching && !matched.has(id);
  }

  test('row-update-available remains hidden when no update is available, even on matching search', () => {
    const matched = new Set(['row-update-available', 'row-update-check']);
    // When no update is available (isAvailable = false)
    expect(computeRowHidden('row-update-available', false, false, matched)).toBe(true);
    expect(computeRowHidden('row-update-available', false, true, matched)).toBe(true);
  });

  test('row-update-available appears only when an update is available', () => {
    const matched = new Set(['row-update-available']);
    // When update IS available (isAvailable = true)
    expect(computeRowHidden('row-update-available', true, false, matched)).toBe(false);
    expect(computeRowHidden('row-update-available', true, true, matched)).toBe(false);
    // When searching for something else
    expect(computeRowHidden('row-update-available', true, true, new Set(['kAnthropic']))).toBe(true);
  });
});

