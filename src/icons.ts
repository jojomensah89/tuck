/**
 * Tuck's icon family: hand-drawn line icons on a 20x20 grid with a uniform
 * 1.5px stroke, round caps and round joins (styled by `svg.ico` in styles.css).
 * The stroke does not scale, so a 14px compact glyph and a 24px play button
 * carry the same line weight. Shapes with class `f` are filled as well as
 * stroked: the transport glyphs, which read as solid marks, and small dots.
 */

const ico = (body: string, cls = '') =>
  `<svg class="ico${cls ? ` ${cls}` : ''}" viewBox="0 0 20 20" aria-hidden="true">${body}</svg>`;

const PLAY = '<path class="f" d="M6.5 4.6v10.8l8.8-5.4z"/>';
const PAUSE = '<rect class="f" x="5.25" y="4.5" width="3" height="11" rx="1"/><rect class="f" x="11.75" y="4.5" width="3" height="11" rx="1"/>';

export const SVG_ICONS = {
  bolt: ico('<path class="f" d="M11 2.75 4.75 11H10l-1 6.25L15.25 9H10z"/>', 'bolt'),
  lyrics: ico('<path d="M4.75 3.75h10.5a1.5 1.5 0 0 1 1.5 1.5v7a1.5 1.5 0 0 1-1.5 1.5H9.5l-3.75 3v-3h-1a1.5 1.5 0 0 1-1.5-1.5v-7a1.5 1.5 0 0 1 1.5-1.5z"/><path d="M6.75 7.25h6.5M6.75 10.25h4"/>', 'lyrics-ico'),
  note: ico('<path d="M7.75 14.5V4.75l8.5-2v9.75"/><circle cx="5.75" cy="14.5" r="2"/><circle cx="14.25" cy="12.5" r="2"/>'),
  clock: ico('<circle cx="10" cy="10" r="7.25"/><path d="M10 6v4l2.75 1.75"/>'),
  mic: ico('<rect x="7.5" y="2.75" width="5" height="9" rx="2.5"/><path d="M4.75 9.5a5.25 5.25 0 0 0 10.5 0M10 14.75v2.5"/>'),
  play: ico(PLAY),
  pause: ico(PAUSE),
  playBig: ico(PLAY, 'big'),
  pauseBig: ico(PAUSE, 'big'),
  prev: ico('<path d="M4.75 4.75v10.5"/><path class="f" d="M15.25 4.9v10.2L7.75 10z"/>'),
  next: ico('<path d="M15.25 4.75v10.5"/><path class="f" d="M4.75 4.9v10.2L12.25 10z"/>'),
  gear: ico('<path d="M8.52 4.49 8.81 2.49A7.6 7.6 0 0 1 11.19 2.49L11.48 4.49A5.7 5.7 0 0 1 12.85 5.06L14.47 3.85A7.6 7.6 0 0 1 16.15 5.53L14.94 7.15A5.7 5.7 0 0 1 15.51 8.52L17.51 8.81A7.6 7.6 0 0 1 17.51 11.19L15.51 11.48A5.7 5.7 0 0 1 14.94 12.85L16.15 14.47A7.6 7.6 0 0 1 14.47 16.15L12.85 14.94A5.7 5.7 0 0 1 11.48 15.51L11.19 17.51A7.6 7.6 0 0 1 8.81 17.51L8.52 15.51A5.7 5.7 0 0 1 7.15 14.94L5.53 16.15A7.6 7.6 0 0 1 3.85 14.47L5.06 12.85A5.7 5.7 0 0 1 4.49 11.48L2.49 11.19A7.6 7.6 0 0 1 2.49 8.81L4.49 8.52A5.7 5.7 0 0 1 5.06 7.15L3.85 5.53A7.6 7.6 0 0 1 5.53 3.85L7.15 5.06A5.7 5.7 0 0 1 8.52 4.49Z"/><circle cx="10" cy="10" r="2.25"/>'),
  grid: ico('<rect x="3.25" y="3.25" width="5.5" height="5.5" rx="1.5"/><rect x="11.25" y="3.25" width="5.5" height="5.5" rx="1.5"/><rect x="3.25" y="11.25" width="5.5" height="5.5" rx="1.5"/><rect x="11.25" y="11.25" width="5.5" height="5.5" rx="1.5"/>'),
  wifi: ico('<path d="M2.75 7.75a10.25 10.25 0 0 1 14.5 0M5.25 10.75a6.75 6.75 0 0 1 9.5 0M7.75 13.75a3.2 3.2 0 0 1 4.5 0"/><circle class="f" cx="10" cy="16.5" r=".5"/>'),
  bluetooth: ico('<path d="m5.75 6.5 8.5 7L10 17.25V2.75l4.25 3.75-8.5 7"/>'),
  focus: ico('<path d="M16.5 12.25A6.75 6.75 0 0 1 7.75 3.5a6.75 6.75 0 1 0 8.75 8.75z"/>'),
  capture: ico('<path d="M3.25 7V5.25a2 2 0 0 1 2-2H7M13 3.25h1.75a2 2 0 0 1 2 2V7M16.75 13v1.75a2 2 0 0 1-2 2H13M7 16.75H5.25a2 2 0 0 1-2-2V13"/><circle cx="10" cy="10" r="2.5"/>'),
  timer: ico('<circle cx="10" cy="11" r="6.25"/><path d="M8 2.75h4M10 11V7.75"/>'),
  stopwatch: ico('<circle cx="10" cy="11" r="6.25"/><path d="M10 2.75v2M8.25 2.75h3.5M15.25 5.25l1 -1M10 11l2.25-2.25"/>'),
  pomodoro: ico('<circle cx="10" cy="11.25" r="6"/><path d="M10 5.25v-2.5M10 5.25c-1-1.25-2.5-1.5-3.75-1.25M10 5.25c1-1.25 2.5-1.5 3.75-1.25"/>'),
  calendar: ico('<rect x="3.25" y="4.25" width="13.5" height="12.5" rx="2.5"/><path d="M3.25 8.25h13.5M7 2.75v3M13 2.75v3"/><circle class="f" cx="7.25" cy="12.25" r=".75"/>'),
  gauge: ico('<path d="M4.4 15.25a7.25 7.25 0 1 1 11.2 0"/><path d="M10 11.5l3-3"/><circle class="f" cx="10" cy="11.5" r=".75"/>'),
  check: ico('<path d="m4.75 10.5 3.5 3.5 7-8"/>'),
  refresh: ico('<path d="M16.25 10a6.25 6.25 0 1 1-1.85-4.45"/><path d="M15.25 2.75v3.5h-3.5"/>'),
  external: ico('<path d="M11.25 3.25h5.5v5.5M16.75 3.25 9.5 10.5"/><path d="M14.25 12.25v2.5a2 2 0 0 1-2 2h-7a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h2.5"/>'),
  sparkle: ico('<path d="M10 3q.9 6.1 7 7-6.1.9-7 7-.9-6.1-7-7 6.1-.9 7-7z"/>'),
  alert: ico('<path d="M10 3.25 2.75 16.25h14.5z"/><path d="M10 8.25v3.5"/><circle class="f" cx="10" cy="14" r=".5"/>', 'warn'),
  eq: `<span class="eq" aria-hidden="true"><i></i><i></i><i></i></span>`,
  eqPlaying: `<span class="eq playing" aria-hidden="true"><i></i><i></i><i></i></span>`,
};

export function batteryGlyph(
  percent: number,
  charging: boolean,
  known = true,
  hasBattery = true,
): string {
  const levelKnown = known && hasBattery && Number.isFinite(percent);
  const normalized = levelKnown ? Math.max(0, Math.min(100, Math.round(percent))) : 0;
  const fillWidth = Math.max(normalized > 0 ? 1 : 0, 16 * normalized / 100);
  const stateClass = charging
    ? ' charging'
    : !levelKnown
      ? ' unknown'
      : normalized < 20
        ? ' low'
        : '';
  const bolt = charging
    ? '<path class="battery-bolt" d="m12.4 7.4-3.8 5h2.7l-.5 4.2 4.1-5.5h-2.6z"/>'
    : '';

  return `<svg class="battery-glyph${stateClass}" viewBox="0 0 24 18" aria-hidden="true"><rect class="battery-outline" x="1" y="3" width="20" height="12" rx="2"/><rect class="battery-terminal" x="22" y="7" width="2" height="4" rx="1"/><rect class="battery-fill" x="3" y="5" width="${fillWidth.toFixed(2)}" height="8" rx="1"/>${bolt}</svg>`;
}
