/** Persist the claim before displaying so reopening the island cannot nag. */
export function claimUpdateVersion(storage: Pick<Storage, 'getItem' | 'setItem'>, version: string): boolean {
  const key = `tuck.update-announced.${version}`;
  try { if (storage.getItem(key)) return false; storage.setItem(key, '1'); return true; }
  catch { return false; }
}
