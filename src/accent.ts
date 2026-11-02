/**
 * Extracts a vibrant accent color from an image (album art),
 * lifting dark colors so they remain legible and aesthetic against a pure black background.
 */

export interface RgbColor {
  r: number;
  g: number;
  b: number;
}

export function extractAccentColor(
  src: string,
  onColor: (color: RgbColor | null) => void
): void {
  if (!src) {
    onColor(null);
    return;
  }

  const img = new Image();
  img.crossOrigin = 'anonymous';

  img.onload = () => {
    try {
      const canvas = document.createElement('canvas');
      canvas.width = 8;
      canvas.height = 8;
      const ctx = canvas.getContext('2d');
      if (!ctx) {
        onColor(null);
        return;
      }

      ctx.drawImage(img, 0, 0, 8, 8);
      const imgData = ctx.getImageData(0, 0, 8, 8).data;
      let r = 0,
        g = 0,
        b = 0,
        weightSum = 0;

      for (let i = 0; i < imgData.length; i += 4) {
        const pr = imgData[i];
        const pg = imgData[i + 1];
        const pb = imgData[i + 2];
        const pa = imgData[i + 3];

        if (pa < 128) continue; // ignore transparent pixels

        // Saturation weighting: prioritize colorful pixels
        const sat = Math.max(pr, pg, pb) - Math.min(pr, pg, pb);
        const w = 1 + sat / 32;

        r += pr * w;
        g += pg * w;
        b += pb * w;
        weightSum += w;
      }

      if (weightSum === 0) {
        onColor(null);
        return;
      }

      r /= weightSum;
      g /= weightSum;
      b /= weightSum;

      // Calculate relative luminance:
      const L = 0.2126 * r + 0.7152 * g + 0.0722 * b;

      // A muted artwork wash, not the pale grey produced by lifting every cover to 130.
      if (L < 65) {
        const t = (65 - L) / (255 - L);
        r += (255 - r) * t;
        g += (255 - g) * t;
        b += (255 - b) * t;
      }

      onColor({
        r: Math.round(Math.min(255, Math.max(0, r))),
        g: Math.round(Math.min(255, Math.max(0, g))),
        b: Math.round(Math.min(255, Math.max(0, b))),
      });
    } catch {
      onColor(null);
    }
  };

  img.onerror = () => {
    onColor(null);
  };

  img.src = src;
}
