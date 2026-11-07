import { describe, expect, test } from 'bun:test';
import { describeResult, filterSettings, isRowHidden, matchesTokens, navTarget, normalizeText, queryTokens } from '../settings-search';
import { parseReleaseNotes } from '../update-notes';

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

describe('update availability and search', () => {
  const rows = [
    { id: 'row-update-check', view: 'about', text: 'About Updates Software update Check for updates update version' },
    { id: 'row-update-available', view: 'about', text: 'About Updates Update available update install changelog' },
    { id: 'row-refresh', view: 'usage', text: 'Usage Refresh limits update' },
  ];
  const noUpdate = new Set(['row-update-available']);
  const hasUpdate = new Set<string>();

  test('a hidden update row is not counted and cannot be the only result', () => {
    const changelog = filterSettings(rows, 'changelog', noUpdate);
    expect(changelog.ids).toEqual([]);
    expect(changelog.views.size).toBe(0);
    expect(describeResult(changelog, 'changelog')).toBe('No settings match "changelog".');
    expect(describeResult(filterSettings(rows, 'update', noUpdate), 'update')).toBe('2 settings match "update".');
  });

  test('the update row counts once an update is available', () => {
    expect(describeResult(filterSettings(rows, 'changelog', hasUpdate), 'changelog')).toBe('1 setting matches "changelog".');
    expect(describeResult(filterSettings(rows, 'update', hasUpdate), 'update')).toBe('3 settings match "update".');
  });

  test('every counted row is visible and every uncounted row is hidden', () => {
    for (const unavailable of [noUpdate, hasUpdate]) {
      for (const query of ['update', 'changelog', 'zzz']) {
        const matched = new Set(filterSettings(rows, query, unavailable).ids);
        const visible = rows.filter(row => !isRowHidden(row.id, true, matched, unavailable)).map(row => row.id);
        expect(visible).toEqual([...matched]);
      }
    }
  });

  test('without a search only unavailable rows are hidden', () => {
    const matched = new Set(filterSettings(rows, '', noUpdate).ids);
    expect(isRowHidden('row-update-available', false, matched, noUpdate)).toBe(true);
    expect(isRowHidden('row-update-check', false, matched, noUpdate)).toBe(false);
    expect(isRowHidden('row-update-available', false, new Set(), hasUpdate)).toBe(false);
  });
});

describe('release notes for Settings', () => {
  test('turns the published notes into headings and list items', () => {
    expect(parseReleaseNotes('### Added\n- Quiet updates\n- Tray menu\n\n### Fixed\n* A crash\n')).toEqual([
      { kind: 'heading', text: 'Added' },
      { kind: 'item', text: 'Quiet updates' },
      { kind: 'item', text: 'Tray menu' },
      { kind: 'heading', text: 'Fixed' },
      { kind: 'item', text: 'A crash' },
    ]);
  });

  test('drops the version heading and handles Windows line endings', () => {
    expect(parseReleaseNotes('## [0.2.0] - 2026-11-04\r\n\r\n### Added\r\n- One')).toEqual([
      { kind: 'heading', text: 'Added' },
      { kind: 'item', text: 'One' },
    ]);
  });

  test('keeps plain text and returns nothing for empty notes', () => {
    expect(parseReleaseNotes('Just a sentence.')).toEqual([{ kind: 'text', text: 'Just a sentence.' }]);
    expect(parseReleaseNotes('  \n ')).toEqual([]);
  });
});
