/** Turns key presses into the shortcut strings the native side accepts. */

export const DUAL_SHIFT = 'Shift+Shift';
export const RIGHT_ALT = 'RightAlt';
export const BOTH_ALTS = 'Alt+Alt';

export interface KeyPress { code: string; ctrl: boolean; alt: boolean; shift: boolean; meta: boolean }
export type Recorded =
  | { kind: 'partial'; preview: string }
  | { kind: 'done'; value: string }
  | { kind: 'refused'; reason: string };

const MODIFIER_CODES = new Set(['ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight', 'MetaLeft', 'MetaRight']);
// Windows owns these, so a shortcut built on them would never reach Tuck.
const RESERVED = new Set(['Ctrl+Alt+Delete', 'Super+L', 'Alt+Tab', 'Alt+F4', 'Ctrl+Escape']);

function keyName(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  if (/^Numpad[0-9]$/.test(code)) return code;
  if (/^Arrow(Up|Down|Left|Right)$/.test(code)) return code;
  const named = ['Space', 'Enter', 'Tab', 'Backspace', 'Delete', 'Insert', 'Home', 'End', 'PageUp', 'PageDown',
    'Minus', 'Equal', 'BracketLeft', 'BracketRight', 'Backslash', 'Semicolon', 'Quote', 'Backquote', 'Comma', 'Period', 'Slash'];
  return named.includes(code) ? code : null;
}

function modifiers(press: KeyPress): string[] {
  return [press.ctrl && 'Ctrl', press.alt && 'Alt', press.shift && 'Shift', press.meta && 'Super'].filter((name): name is string => Boolean(name));
}

/** `pressed` holds the codes currently down, which is the only way to see both Shift keys. */
export function recordKey(press: KeyPress, pressed: ReadonlySet<string>, allowDualShift: boolean): Recorded {
  if (pressed.has('AltLeft') && pressed.has('AltRight')) {
    return allowDualShift
      ? { kind: 'done', value: BOTH_ALTS }
      : { kind: 'refused', reason: 'Both Alt keys only work for Hold to talk.' };
  }
  if (pressed.has('ShiftLeft') && pressed.has('ShiftRight')) {
    return allowDualShift
      ? { kind: 'done', value: DUAL_SHIFT }
      : { kind: 'refused', reason: 'Both Shift keys only work for Hold to talk.' };
  }
  const mods = modifiers(press);
  if (MODIFIER_CODES.has(press.code)) return { kind: 'partial', preview: mods.join('+') };
  const key = keyName(press.code);
  if (!key) return { kind: 'refused', reason: 'That key cannot be used in a shortcut.' };
  const isFunctionKey = /^F\d+$/.test(key);
  if (!isFunctionKey && !mods.some(mod => mod !== 'Shift')) {
    return { kind: 'refused', reason: 'Add Ctrl, Alt or Win, or this shortcut would fire while you type.' };
  }
  const value = [...mods, key].join('+');
  if (RESERVED.has(value)) return { kind: 'refused', reason: `Windows keeps ${displayShortcut(value)} for itself.` };
  return { kind: 'done', value };
}

/**
 * Right Alt pressed and released on its own is the hold shortcut. `seen` is every code
 * pressed since recording began; AltGr layouts add a fake Left Ctrl to each Right Alt.
 */
export function recordRelease(code: string, seen: ReadonlySet<string>, allowHoldKeys: boolean): Recorded | null {
  if (code !== 'AltRight' || ![...seen].every(key => key === 'AltRight' || key === 'ControlLeft')) return null;
  return allowHoldKeys
    ? { kind: 'done', value: RIGHT_ALT }
    : { kind: 'refused', reason: 'Right Alt alone only works for Hold to talk.' };
}

export function isBothAlts(value: string): boolean {
  return /^(alt\+alt|bothalts|twoalts|(left|l)alt\+(right|r)alt|(right|r)alt\+(left|l)alt)$/.test(value.toLowerCase().replace(/ /g, ''));
}

export function isRightAlt(value: string): boolean {
  return /^(rightalt|ralt|altright)$/.test(value.toLowerCase().replace(/ /g, ''));
}

export function isDualShift(value: string): boolean {
  return /^(shift\+shift|twoshifts?|dualshifts?|(left|l)shift\+(right|r)shift|(right|r)shift\+(left|l)shift)$/.test(value.toLowerCase().replace(/ /g, ''));
}

export function displayShortcut(value: string): string {
  if (isDualShift(value)) return 'Left + Right Shift';
  if (isRightAlt(value)) return 'Right Alt';
  if (isBothAlts(value)) return 'Left + Right Alt';
  return value.replace(/\bSuper\b/g, 'Win').replace(/\bArrow(Up|Down|Left|Right)\b/g, '$1').replace(/\bBackquote\b/g, '`');
}
