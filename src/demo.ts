/**
 * The website demo: tuck.jojomensah.com frames the real island with `?embed`.
 * Everything here is invented sample data (tracks, lyrics, clips, files, quota)
 * answered through the same commands the desktop app uses. It never runs in the
 * desktop app, and the plain browser preview (no `?embed`) stays as it was.
 */
import { dispatchMockEvent, isTauri, setBrowserHandler } from './ipc.js';
import type { MediaState, Meter, PowerState } from './types.js';
import type { ClipboardItem } from './live-services.js';
import type { ShelfItem, Thumb } from './extensions-types.js';

export function embedRequested(): boolean {
  if (isTauri() || typeof location === 'undefined') return false;
  return new URLSearchParams(location.search).has('embed');
}

interface DemoTrack { id: string; title: string; artist: string; durationMs: number; ground: string; mark: string; lyrics: string[] }

// Invented songs; every lyric line is written for this demo
const TRACKS: DemoTrack[] = [
  {
    id: 'demo-northbound', title: 'Northbound Static', artist: 'Juniper Wells', durationMs: 222000, ground: '#1d5c58', mark: '#f2a541',
    lyrics: ['Radio hums along the motorway', 'every station fading into grey', 'I keep the window down to hear you', 'over the static, over the rain',
      'northbound lights are blinking slowly', 'counting exits one by one', 'if the signal finds you tonight', 'tell me where the road has gone'],
  },
  {
    id: 'demo-copper', title: 'Copper Afternoon', artist: 'The Quiet Hours', durationMs: 245000, ground: '#7a2e1f', mark: '#f4d9b0',
    lyrics: ['Kettle on and the blinds half open', 'copper light across the floor', 'nobody calling, nothing broken', 'just the hour and nothing more',
      'we could stay here till the evening', 'let the dust do what it does', 'hold the cup and keep the feeling', 'of a day that simply was'],
  },
  {
    id: 'demo-harbour', title: 'Glass Harbour', artist: 'Mara Okafor', durationMs: 198000, ground: '#233f7a', mark: '#9fd3e6',
    lyrics: ['Boats asleep in the glass harbour', 'ropes are creaking soft and low', 'I wrote your name on the water', 'watched the little letters go',
      'tide comes in to read the message', 'carries every word away', 'meet me where the lighthouse flickers', 'at the quiet end of day'],
  },
];
const LINE_MS = 4200;

function drawArtwork(track: DemoTrack): string {
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 240;
  const g = canvas.getContext('2d');
  if (!g) return '';
  g.fillStyle = track.ground; g.fillRect(0, 0, 240, 240);
  g.fillStyle = track.mark;
  if (track.id === 'demo-northbound') {
    // A low sun over a road
    g.beginPath(); g.arc(120, 150, 62, Math.PI, 0); g.fill();
    g.fillRect(0, 150, 240, 8);
    g.beginPath(); g.moveTo(104, 240); g.lineTo(118, 166); g.lineTo(122, 166); g.lineTo(136, 240); g.fill();
  } else if (track.id === 'demo-copper') {
    // Light through blinds
    for (let i = 0; i < 6; i++) g.fillRect(40, 36 + i * 30, 160, 14);
  } else {
    // A lighthouse beam over water
    g.beginPath(); g.moveTo(150, 70); g.lineTo(240, 40); g.lineTo(240, 100); g.closePath(); g.globalAlpha = 0.55; g.fill(); g.globalAlpha = 1;
    g.fillRect(138, 70, 16, 96);
    for (let i = 0; i < 4; i++) g.fillRect(24 + i * 8, 182 + i * 14, 192 - i * 16, 4);
  }
  return canvas.toDataURL('image/png');
}

/** A small flat picture for clipboard and shelf thumbnails */
function drawThumb(kind: 'map' | 'photo' | 'chart'): string {
  const canvas = document.createElement('canvas');
  canvas.width = 320; canvas.height = 200;
  const g = canvas.getContext('2d');
  if (!g) return '';
  if (kind === 'map') {
    g.fillStyle = '#e8e4d8'; g.fillRect(0, 0, 320, 200);
    g.fillStyle = '#b9d3c2'; g.fillRect(0, 120, 320, 80);
    g.fillStyle = '#9cc0e0'; g.beginPath(); g.ellipse(250, 60, 90, 40, 0.3, 0, Math.PI * 2); g.fill();
    g.strokeStyle = '#d0532f'; g.lineWidth = 6; g.lineCap = 'round';
    g.beginPath(); g.moveTo(30, 170); g.bezierCurveTo(110, 120, 140, 160, 200, 90); g.lineTo(240, 70); g.stroke();
  } else if (kind === 'photo') {
    g.fillStyle = '#8fc1d9'; g.fillRect(0, 0, 320, 110);
    g.fillStyle = '#2f6f8f'; g.fillRect(0, 110, 320, 40);
    g.fillStyle = '#e9d6a8'; g.fillRect(0, 150, 320, 50);
    g.fillStyle = '#fff4d0'; g.beginPath(); g.arc(240, 60, 26, 0, Math.PI * 2); g.fill();
  } else {
    g.fillStyle = '#f4f4f2'; g.fillRect(0, 0, 320, 200);
    g.fillStyle = '#2456d6';
    [60, 92, 74, 120, 104, 146].forEach((h, i) => g.fillRect(36 + i * 44, 180 - h, 28, h));
  }
  return canvas.toDataURL('image/png');
}

export function startEmbedDemo(): void {
  if (!embedRequested()) return;
  document.documentElement.classList.add('embed');
  const art = new Map<string, string>();
  for (const track of TRACKS) art.set(track.id, drawArtwork(track));
  const thumbs = { map: drawThumb('map'), photo: drawThumb('photo'), chart: drawThumb('chart') };

  let index = 0;
  let playing = true;
  let anchorMs = 61000;
  let anchorAt = performance.now();
  const position = () => Math.min(TRACKS[index].durationMs, anchorMs + (playing ? performance.now() - anchorAt : 0));
  const media = (withArtwork: boolean): void => {
    const track = TRACKS[index];
    const state: MediaState = {
      active: true, trackId: track.id, title: track.title, artist: track.artist, playing,
      positionMs: Math.round(position()), durationMs: track.durationMs, artwork: withArtwork ? art.get(track.id) ?? null : null,
      canPlayPause: true, canSkipNext: true, canSkipPrev: true, canSeek: true,
    };
    dispatchMockEvent<MediaState>('media', state);
  };
  const skip = (step: number): void => {
    index = (index + step + TRACKS.length) % TRACKS.length;
    anchorMs = 0; anchorAt = performance.now(); playing = true;
    media(true);
  };
  window.setInterval(() => { if (playing && position() >= TRACKS[index].durationMs) skip(1); }, 1000);

  let clips: ClipboardItem[] = [
    { id: 'c1', kind: 'link', text: 'https://github.com/jojomensah89/tuck/releases/latest', pinned: false, createdAt: Date.now() - 40_000 },
    { id: 'c2', kind: 'text', text: 'Moved the review to Thursday at 14:00, same room.', pinned: false, createdAt: Date.now() - 6 * 60_000 },
    { id: 'c3', kind: 'image', text: 'Screenshot 2026-11-09 101422.png', thumbnail: thumbs.chart, pinned: false, createdAt: Date.now() - 18 * 60_000 },
    { id: 'c4', kind: 'text', text: 'Thanks, I will send the draft tonight.', pinned: true, createdAt: Date.now() - 3 * 3600_000 },
    { id: 'c5', kind: 'file', text: 'invoice-0423.pdf', paths: ['C:\\Users\\you\\Documents\\invoice-0423.pdf'], pinned: false, createdAt: Date.now() - 5 * 3600_000 },
  ];
  const clipboard = () => ({ enabled: true, persist: false, items: clips });
  const pushClips = () => dispatchMockEvent('clipboard_history_changed', clipboard());

  let shelf: ShelfItem[] = [
    { id: 's1', name: 'weekend-route.png', path: 'C:\\Users\\you\\Pictures\\weekend-route.png', bytes: 1_843_200 },
    { id: 's2', name: 'beach.jpg', path: 'C:\\Users\\you\\Pictures\\beach.jpg', bytes: 2_516_582 },
    { id: 's3', name: 'q3-numbers.png', path: 'C:\\Users\\you\\Desktop\\q3-numbers.png', bytes: 412_876 },
    { id: 's4', name: 'Project brief.docx', path: 'C:\\Users\\you\\Documents\\Project brief.docx', bytes: 88_064 },
  ];
  const shelfThumb: Record<string, Thumb> = {
    s1: { kind: 'image', url: thumbs.map }, s2: { kind: 'image', url: thumbs.photo }, s3: { kind: 'image', url: thumbs.chart }, s4: { kind: 'none' },
  };
  const snapshot = () => ({ settings: { notifications: false }, shelf, notificationStatus: 'Disabled', errors: [] });

  const SAID = 'Remind me to send Ama the slides before the Thursday review, and book the small room.';
  let dictationStatus = 'idle';
  let dictationTimers: number[] = [];
  const dictation = (status: string, extra: Record<string, unknown> = {}) => {
    dictationStatus = status;
    dispatchMockEvent('dictation_state', {
      status, transcript: status === 'review' ? SAID : '', modelReady: true, activeModelLabel: 'Whisper Base',
      settings: { toggleShortcut: 'Ctrl+Shift+Space', holdToTalkShortcut: 'Shift+Shift', reviewBeforeInsert: true }, ...extra,
    });
  };
  const stopDictation = () => { dictationTimers.forEach(id => window.clearTimeout(id)); dictationTimers = []; };
  const finishRecording = () => {
    stopDictation();
    dictation('transcribing');
    dictationTimers.push(window.setTimeout(() => dictation('review'), 900));
  };

  const started = performance.now();
  const wave = (period: number, low: number, high: number) => low + (high - low) * (0.5 + 0.5 * Math.sin((performance.now() - started) / period));

  const meters = (): Meter[] => {
    const now = Date.now() / 1000;
    return [
      { id: 'claude-5h', provider: 'claude', label: 'Claude session (5 h)', usedPct: 62, spentUsd: null, resetsAt: Math.round(now + 8100), source: 'official', stale: false },
      { id: 'claude-7d', provider: 'claude', label: 'Claude weekly (7 d)', usedPct: 31, spentUsd: null, resetsAt: Math.round(now + 86400 * 3), source: 'official', stale: false },
      { id: 'codex-5h', provider: 'openai', label: 'Codex session (5 h)', usedPct: 72, spentUsd: null, resetsAt: Math.round(now + 3600 * 4), source: 'official', stale: false },
      { id: 'codex-7d', provider: 'openai', label: 'Codex weekly (7 d)', usedPct: 9, spentUsd: null, resetsAt: Math.round(now + 86400 * 5), source: 'official', stale: false },
    ];
  };

  setBrowserHandler(async (cmd, args) => {
    switch (cmd) {
      case 'media_toggle':
        anchorMs = position(); anchorAt = performance.now(); playing = !playing; media(false); return undefined;
      case 'media_next': skip(1); return undefined;
      case 'media_prev': if (position() > 3000) { anchorMs = 0; anchorAt = performance.now(); media(false); } else skip(-1); return undefined;
      case 'media_seek': anchorMs = Math.max(0, Number(args?.ms) || 0); anchorAt = performance.now(); media(false); return undefined;
      case 'usage_now': case 'refresh_usage': return meters();
      case 'media_sessions': return [{ id: 'demo', name: 'Spotify', selected: true }];
      case 'lyrics_lookup': {
        const track = TRACKS.find(t => t.id === args?.trackId) ?? TRACKS[index];
        const lines = Array.from({ length: Math.ceil(track.durationMs / LINE_MS) }, (_, i) => ({ atMs: i * LINE_MS, text: track.lyrics[i % track.lyrics.length] }));
        return { trackId: track.id, requestId: args?.requestId, status: 'synced', lines };
      }
      case 'telemetry_snapshot':
        return {
          sampledAt: Date.now(), cpuPct: Math.round(wave(2300, 7, 16)), ramUsedBytes: Math.round(wave(5000, 11.1, 11.6) * 2 ** 30), ramTotalBytes: 32 * 2 ** 30,
          gpus: [{ id: '0:GPU', name: 'NVIDIA GeForce RTX 4060', usedPct: Math.round(wave(1700, 3, 11)) }],
          networkDownBps: Math.round(wave(1300, 180_000, 2_400_000)), networkUpBps: Math.round(wave(2100, 20_000, 160_000)),
        };
      case 'dictation_snapshot':
        return { status: dictationStatus, transcript: dictationStatus === 'review' ? SAID : '', modelReady: true, activeModelLabel: 'Whisper Base',
          settings: { toggleShortcut: 'Ctrl+Shift+Space', holdToTalkShortcut: 'Shift+Shift', reviewBeforeInsert: true } };
      case 'dictation_toggle': {
        if (dictationStatus === 'recording') { finishRecording(); return undefined; }
        stopDictation();
        dictation('recording', { audioLevel: 0.4 });
        for (let i = 1; i < 18; i++) dictationTimers.push(window.setTimeout(() => dictation('recording', { audioLevel: 0.25 + Math.abs(Math.sin(i * 1.7)) * 0.75 }), i * 150));
        dictationTimers.push(window.setTimeout(finishRecording, 2800));
        return undefined;
      }
      case 'dictation_stop': finishRecording(); return undefined;
      case 'dictation_cancel': stopDictation(); dictation('idle'); return undefined;
      case 'dictation_insert':
        stopDictation(); dictation('idle', { message: 'Copied' });
        clips = [{ id: `d${Date.now()}`, kind: 'text', text: SAID, pinned: false, createdAt: Date.now() }, ...clips]; pushClips();
        return undefined;
      case 'dictation_keep_warm': case 'dictation_models_scan': case 'dictation_shortcuts_pause': case 'dictation_shortcuts_retry': return undefined;
      case 'clipboard_snapshot': case 'clipboard_settings': return clipboard();
      case 'clipboard_thumbnail': return args?.id === 'c3' ? { kind: 'image', url: thumbs.chart } : { kind: 'none' };
      case 'clipboard_copy': return undefined;
      case 'clipboard_pin': clips = clips.map(c => c.id === args?.id ? { ...c, pinned: Boolean(args?.pinned) } : c); return undefined;
      case 'clipboard_remove': clips = clips.filter(c => c.id !== args?.id); return undefined;
      case 'clipboard_clear': clips = clips.filter(c => c.pinned); return undefined;
      case 'extension_snapshot': return snapshot();
      case 'shelf_thumbnail': return shelfThumb[String(args?.id)] ?? { kind: 'none' };
      case 'shelf_remove': shelf = args?.id ? shelf.filter(item => item.id !== args.id) : []; return undefined;
      case 'shelf_pick': case 'shelf_open': case 'shelf_drag_out': case 'quick_action': return undefined;
      default: return undefined;
    }
  });

  // ?quiet shows the island with nothing playing (the website's resting picture)
  const quiet = new URLSearchParams(location.search).has('quiet');
  window.setTimeout(() => {
    if (!quiet) media(true);
    dispatchMockEvent<PowerState>('power', { percent: 82, charging: true, hasBattery: true, minutesLeft: null });
    dispatchMockEvent<Meter[]>('usage', meters());
  }, 60);
}
