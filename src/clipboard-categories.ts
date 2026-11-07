export type ClipCategory = 'text' | 'link' | 'image' | 'video' | 'file';
export function isClipboardLink(text: string): boolean {
  if (text !== text.trim() || /\s/.test(text)) return false;
  try { const url = new URL(text); return (url.protocol === 'http:' || url.protocol === 'https:') && !!url.hostname; }
  catch { return false; }
}
export function clipCategory(item: { text: string; kind?: ClipCategory }): ClipCategory {
  return item.kind ?? (isClipboardLink(item.text) ? 'link' : 'text');
}
export const CLIP_CATEGORIES = [
  ['all', 'All clips'], ['text', 'Text'], ['link', 'Links'],
  ['image', 'Images'], ['video', 'Videos'], ['file', 'Files'],
] as const;
