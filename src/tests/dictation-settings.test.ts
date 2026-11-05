import { describe, expect, test } from 'bun:test';
import { formatSize, islandModelLine, modelHeader, modelsLocked, removeConfirmation, rowButtons, rowMeta, rowProgress, rowStateText, type ModelRow } from '../dictation-models.js';
import { displayShortcut, DUAL_SHIFT, isDualShift, recordKey, type KeyPress } from '../shortcut-recorder.js';

const row = (over: Partial<ModelRow> = {}): ModelRow => ({ id: 'ggml-base-q5_1.bin', label: 'Base', bytes: 59_707_625, speed: 'Very fast', accuracy: 'Good', note: null, state: 'available', foundIn: null, active: false, ...over });
const actions = (over: Partial<ModelRow>) => rowButtons(row(over)).map(button => button.action);

describe('model rows', () => {
  test('sizes read in plain units', () => {
    expect(formatSize(59_707_625)).toBe('60 MB');
    expect(formatSize(1_624_555_275)).toBe('1.6 GB');
  });
  test('a row names its size, speed and accuracy', () => {
    expect(rowMeta(row())).toBe('60 MB · Speed: very fast · Accuracy: good');
    expect(rowMeta(row({ memoryNote: 'Uses about 400 MB of memory.', note: 'English only.' }))).toBe('60 MB · Speed: very fast · Accuracy: good · Uses about 400 MB of memory. · English only.');
  });
  test('each state offers only what it can do', () => {
    expect(actions({ state: 'available' })).toEqual(['download']);
    expect(actions({ state: 'downloading' })).toEqual(['cancel']);
    expect(actions({ state: 'checking' })).toEqual([]);
    expect(actions({ state: 'installed' })).toEqual(['remove', 'use']);
    expect(actions({ state: 'installed', active: true })).toEqual(['remove']);
  });
  test('a file from another app can never be removed, only stopped', () => {
    expect(actions({ state: 'found' })).toEqual(['use']);
    expect(actions({ state: 'found', active: true })).toEqual(['stop']);
    expect(rowStateText(row({ state: 'found', foundIn: 'podcli' }), 0)).toBe('Found in podcli');
  });
  test('download progress is bounded and only present while downloading', () => {
    const downloading = row({ state: 'downloading' });
    expect(rowProgress(downloading, 29_853_812)).toBeCloseTo(0.5, 2);
    expect(rowProgress(downloading, downloading.bytes * 2)).toBe(1);
    expect(rowProgress(row(), 100)).toBeNull();
    expect(rowStateText(downloading, 30_000_000)).toBe('Downloading 30 MB of 60 MB');
  });
  test('the model in use is always named', () => {
    expect(modelHeader('Tiny')).toBe('In use: Tiny');
    expect(modelHeader(null)).toContain('No model');
    expect(islandModelLine('Large v3 Turbo')).toBe('Model: Large v3 Turbo');
    expect(islandModelLine(undefined)).toBe('No model selected');
  });
  test('removing the active model warns about the consequence', () => {
    expect(removeConfirmation(row({ active: true }))).toContain('dictation stops');
    expect(removeConfirmation(row())).toContain('download it again');
  });
  test('models lock only while a session needs them', () => {
    expect(modelsLocked('recording')).toBe(true);
    expect(modelsLocked('review')).toBe(true);
    expect(modelsLocked('idle')).toBe(false);
    expect(modelsLocked('error')).toBe(false);
  });
});

const press = (code: string, mods: Partial<KeyPress> = {}): KeyPress => ({ code, ctrl: false, alt: false, shift: false, meta: false, ...mods });
const none = new Set<string>();

describe('shortcut recorder', () => {
  test('records a modifier and key in the form the native parser reads', () => {
    expect(recordKey(press('Space', { ctrl: true, shift: true }), none, true)).toEqual({ kind: 'done', value: 'Ctrl+Shift+Space' });
    expect(recordKey(press('KeyA', { ctrl: true, alt: true }), none, true)).toEqual({ kind: 'done', value: 'Ctrl+Alt+A' });
    expect(recordKey(press('Digit1', { meta: true, shift: true }), none, true)).toEqual({ kind: 'done', value: 'Shift+Super+1' });
    expect(recordKey(press('F13'), none, true)).toEqual({ kind: 'done', value: 'F13' });
  });
  test('modifiers alone keep listening', () => {
    expect(recordKey(press('ControlLeft', { ctrl: true }), new Set(['ControlLeft']), true)).toEqual({ kind: 'partial', preview: 'Ctrl' });
  });
  test('both Shift keys finish a hold shortcut and are refused for start and stop', () => {
    const both = new Set(['ShiftLeft', 'ShiftRight']);
    expect(recordKey(press('ShiftRight', { shift: true }), both, true)).toEqual({ kind: 'done', value: DUAL_SHIFT });
    const refused = recordKey(press('ShiftRight', { shift: true }), both, false);
    expect(refused.kind).toBe('refused');
  });
  test('a key that would fire while typing is refused with a reason', () => {
    for (const attempt of [press('KeyA'), press('KeyA', { shift: true }), press('Space')]) {
      const result = recordKey(attempt, none, true);
      expect(result.kind).toBe('refused');
      if (result.kind === 'refused') expect(result.reason).toContain('while you type');
    }
    expect(recordKey(press('MediaPlayPause', { ctrl: true }), none, true).kind).toBe('refused');
    expect(recordKey(press('F4', { alt: true }), none, true).kind).toBe('refused');
  });
  test('shortcuts display in Windows words', () => {
    expect(displayShortcut(DUAL_SHIFT)).toBe('Left + Right Shift');
    expect(displayShortcut('two shifts')).toBe('Left + Right Shift');
    expect(displayShortcut('Super+Shift+1')).toBe('Win+Shift+1');
    expect(displayShortcut('Ctrl+Alt+ArrowUp')).toBe('Ctrl+Alt+Up');
    expect(isDualShift('Ctrl+Shift+Space')).toBe(false);
  });
});

 test('unowned local files only offer use or stop, never deletion', () => {
  const row = { id:'ggml-base.bin', label:'Base', bytes:1, speed:'Fast', accuracy:'Good', note:null, state:'installed' as const, foundIn:null, active:true, canRemove:false };
  expect(rowButtons(row)).toEqual([{action:'stop',label:'Stop using'}]);
  expect(rowButtons({...row,active:false})).toEqual([{action:'use',label:'Use',primary:true}]);
});
