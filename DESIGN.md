# Tuck visual system

Mode: Operate. User direction is pinned by the supplied build guide. A black island merges into the desktop display edge and remains dark under either Windows theme. Settings use a conventional readable desktop form.

Tokens: surface #000000; raised surface #1c1c1e; primary ink #ffffff; secondary ink #a1a1a6; accent from artwork with white fallback; charging #30d158; battery warning #ff453a. Segoe UI Variable Text, Segoe UI, system-ui. Minimum controls 36 pixels; play 44 pixels.

Idle 212x32, peek 300x38, open 440x180 logical pixels within 520x230 canvas. Notch is flush with the top edge with 14 pixel concave ears; float has a 10 pixel top gap. Open/peek 560ms cubic-bezier(.32,1.28,.4,1); close 340ms cubic-bezier(.2,.8,.2,1). Reduced motion disables transitions, marquees and equalizer animation.

Use consistent SVG icons; overflow-only marquee; tabular times; visible focus; state-synchronized ARIA and inert hidden layers. Real system and provider state always wins over illustrative content. Browser demo is explicitly labeled.
