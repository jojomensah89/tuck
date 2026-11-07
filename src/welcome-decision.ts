export function hasSavedPreferences(storage: Pick<Storage, 'length' | 'key'>): boolean {
  try {
    for (let i = 0; i < storage.length; i++) {
      const key = storage.key(i);
      if (key?.startsWith('tuck.') || key?.startsWith('notch.')) return true;
    }
    return false;
  } catch { return true; }
}
export const hadSavedPreferences = (() => { try { return hasSavedPreferences(localStorage); } catch { return true; } })();
