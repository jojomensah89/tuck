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
// Speaker body shared by the volume glyphs; waves and the cross sit to its right
const SPEAKER = '<path d="M2.75 7.75h2.5l4-3.25v11l-4-3.25h-2.5z"/>';
const WAVE_NEAR = '<path d="M12 7.75a3.2 3.2 0 0 1 0 4.5"/>';
const WAVE_FAR = '<path d="M14.5 5.25a6.75 6.75 0 0 1 0 9.5"/>';
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
  // Volume HUD: level 0, below 50%, 50% and up, muted
  volumeOff: ico(SPEAKER),
  volumeLow: ico(SPEAKER + WAVE_NEAR),
  volumeHigh: ico(SPEAKER + WAVE_NEAR + WAVE_FAR),
  volumeMute: ico(`${SPEAKER}<path d="m12.75 7.75 4.5 4.5M17.25 7.75l-4.5 4.5"/>`),
  brightness: ico('<circle cx="10" cy="10" r="3.25"/><path d="M10 2.75v1.5M10 15.75v1.5M2.75 10h1.5M15.75 10h1.5M4.87 4.87l1.06 1.06M14.07 14.07l1.06 1.06M15.13 4.87l-1.06 1.06M5.93 14.07l-1.06 1.06"/>'),
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
  // Row and page actions
  pin: ico('<path class="pin-head" d="M7.25 2.75h5.5l-.75 5 2.75 3v1.5h-9.5v-1.5l2.75-3z"/><path d="M10 12.25v5"/>'),
  close: ico('<path d="m5.25 5.25 9.5 9.5M14.75 5.25l-9.5 9.5"/>'),
  trash: ico('<path d="M3.75 5.75h12.5M8 5.75v-2h4v2M5.5 5.75l.75 10.5a1.5 1.5 0 0 0 1.5 1.25h4.5a1.5 1.5 0 0 0 1.5-1.25l.75-10.5"/><path d="M8.5 8.75v5.5M11.5 8.75v5.5"/>'),
  plus: ico('<path d="M10 4.25v11.5M4.25 10h11.5"/>'),
  folder: ico('<path d="M2.75 6.25v-1a1.5 1.5 0 0 1 1.5-1.5h3.5l2 2h6a1.5 1.5 0 0 1 1.5 1.5v7.5a1.5 1.5 0 0 1-1.5 1.5H4.25a1.5 1.5 0 0 1-1.5-1.5z"/><path d="M2.75 8.25h14.5"/>'),
  grip: ico('<circle class="f" cx="7.5" cy="5" r=".75"/><circle class="f" cx="12.5" cy="5" r=".75"/><circle class="f" cx="7.5" cy="10" r=".75"/><circle class="f" cx="12.5" cy="10" r=".75"/><circle class="f" cx="7.5" cy="15" r=".75"/><circle class="f" cx="12.5" cy="15" r=".75"/>'),
  video: ico('<rect x="2.75" y="5.25" width="10.5" height="9.5" rx="2"/><path d="m13.25 9 4-2.5v7l-4-2.5"/>'),
  phone: ico('<path d="M5.5 2.75h2l1.5 4-2 1.25a8.5 8.5 0 0 0 5 5l1.25-2 4 1.5v2a2 2 0 0 1-2.25 2A13.5 13.5 0 0 1 3.5 5a2 2 0 0 1 2-2.25z"/>'),
  download: ico('<path d="M10 3.25v9.5M6 9l4 4 4-4"/><path d="M3.75 13.75v1.5a1.5 1.5 0 0 0 1.5 1.5h9.5a1.5 1.5 0 0 0 1.5-1.5v-1.5"/>'),
  power: ico('<path d="M10 2.75V10"/><path d="M6 5.25a6.25 6.25 0 1 0 8 0"/>'),
  stop: ico('<rect class="f" x="5.25" y="5.25" width="9.5" height="9.5" rx="1.5"/>'),
  record: ico('<circle class="f" cx="10" cy="10" r="4.75"/>'),
  insert: ico('<path d="M16.25 3.75v5.5a2 2 0 0 1-2 2H4.25"/><path d="m8 7.25-3.75 4L8 15.25"/>'),
  plug: ico('<path d="M7.25 2.75v4M12.75 2.75v4M5 6.75h10v2.5a5 5 0 0 1-10 0z"/><path d="M10 14.25v3"/>'),
  // Provider marks: simplified monochrome glyphs for identification only
  anthropic: ico('<path d="M10 2.75v4.5M10 12.75v4.5M2.75 10h4.5M12.75 10h4.5M4.9 4.9l3.2 3.2M11.9 11.9l3.2 3.2M15.1 4.9l-3.2 3.2M8.1 11.9l-3.2 3.2"/>', 'mark'),
  openai: ico('<rect x="7.25" y="2.75" width="5.5" height="14.5" rx="2.75"/><rect x="7.25" y="2.75" width="5.5" height="14.5" rx="2.75" transform="rotate(60 10 10)"/><rect x="7.25" y="2.75" width="5.5" height="14.5" rx="2.75" transform="rotate(120 10 10)"/>', 'mark'),
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
