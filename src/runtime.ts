import type { MediaState } from './types.js';
export function currentPositionMs(media: MediaState, receivedAt: number, now: number): number {
  if (!media.active) return 0;
  const position = Math.max(0, media.positionMs + (media.playing ? Math.max(0, now - receivedAt) : 0));
  return media.durationMs > 0 ? Math.min(media.durationMs, position) : position;
}
export function retainedArtwork(previous: string | null, trackChanged: boolean, artwork?: string | null): string | null {
  // Only native image data URLs may reach CSS; never inject media-supplied strings into HTML.
  const safeArt = artwork && /^data:image\/(png|jpeg|jpg|webp|gif|bmp);base64,[A-Za-z0-9+/=]+$/.test(artwork) ? artwork : null;
  return safeArt || (trackChanged ? null : previous);
}
export function hitGeometry(w: number, h: number, top: number): {w: number; h: number} {
  return { w: Math.round(w), h: Math.round(h + top) };
}
