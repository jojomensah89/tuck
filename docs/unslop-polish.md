# Listening console polish

Approved by Jojo on 2026-11-02. Applied with Unslop by Damir Sokolovsky ([@heydamir](https://x.com/heydamir)), CC BY 4.0: https://github.com/heydamir/unslop.

## Direction

- Product: Windows media and tools within reach
- Character: quiet, responsive, precise
- World: a dark listening console
- Moments: album artwork and the measured live audio spectrum
- Signature: artwork color rising subtly from the notch's bottom edge

## Audit and ordered passes

Light polish, preserving the existing product and layout. The media selector was 10px and truncated its automatic-player label; its toolbar crowded the split column. Track text approached the divider. Controls used inconsistent 5/7/13px spacing, and shell expansion used an overshooting spring.

1. Spacing: toolbar spans the media header; content begins 40px below it. Track and lyrics columns have a 12px gutter plus 16px inset. Related controls use 8px gaps; list rows use 4px vertical padding.
2. Color: preserved pure black and the explicitly requested artwork wash. Selector uses a quiet neutral surface and hairline. Adjacent lyrics use the existing secondary text token color.
3. Icons: preserved the existing SVG set and transport semantics; no new decorative icons.
4. Copy: automatic source is labeled Auto player. Full selected player name is available in its native select and title.
5. Type: retained Segoe UI; player and artist labels are 12px. Lyrics use weight 600. No font download.
6. Layout: retained media/lyrics split, all pages, configured 440–500px width and 210px expanded height within the native canvas. Artwork is 44px; transport controls are 36px.
7. Details: 8px button/artwork radii, consistent control padding and subtle neutral hover feedback. Existing keyboard focus handling remains intact.
8. Visuals: retained actual cover artwork and measured spectrum. No generated images or invented audio data.
9. Motion: replaced shell overshoot with a monotonic easing curve and 380ms expanded transitions. Press scale is 0.98. Width/height morphing remains because the user requested that interaction; existing reduced-motion support remains.

## Verification

Frontend build, 29 Bun tests and native-bridge UI smoke pass. Geometry assertions cover 440px and 500px media panels: toolbar does not overlap track details, track title stays outside the lyrics divider, selector stays clear of the lyrics button, and transport controls fit. Existing seeking, keyboard focus, hover, safety and settings smoke checks remain.

Synthetic fixture screenshots: `artifacts/island-unslop-440.png` and `artifacts/island-unslop-500.png`. These demonstrate layout, not live playback or native desktop hit testing. No Rust, native permissions or audio processing changes are included.
