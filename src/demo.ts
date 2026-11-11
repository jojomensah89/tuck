/**
 * The website demo: tuck.jojomensah.com frames the real island with `?embed`.
 * The songs are public domain (see TRACKS); clips, files and quota are invented
 * sample data. All of it is answered through the same commands the desktop app
 * uses. It never runs in the desktop app, and the plain browser preview (no
 * `?embed`) stays as it was.
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

// Public-domain songs in public-domain recordings (Library of Congress National Jukebox):
// the words are long out of copyright, and the recordings predate 1923. No audio is played or shipped.
// Durations are approximate; the artwork is drawn here, not taken from the records.
const TRACKS: DemoTrack[] = [
  {
    // Victor, 26 September 1917. Words by Frederic Weatherly (1910). loc.gov/item/jukebox-189331
    id: 'pd-danny-boy', title: 'Danny Boy', artist: 'Ernestine Schumann-Heink', durationMs: 198000, ground: '#1f5a3a', mark: '#e9c46a',
    lyrics: ['Oh, Danny boy, the pipes, the pipes are calling', 'From glen to glen, and down the mountain side', 'The summer’s gone, and all the roses falling',
      'It’s you, it’s you must go and I must bide', 'But come ye back when summer’s in the meadow', 'Or when the valley’s hushed and white with snow',
      'It’s I’ll be here in sunshine or in shadow', 'Oh, Danny boy, oh Danny boy, I love you so'],
  },
  {
    // Victor, 11 April 1913. Words and music by Queen Liliʻuokalani (1878). loc.gov/item/jukebox-133081
    id: 'pd-aloha-oe', title: 'Aloha ʻOe', artist: 'Hawaiian Quintette', durationMs: 186000, ground: '#1f4f86', mark: '#f4a261',
    lyrics: ['Haʻaheo ka ua i nā pali', 'Ke nihi aʻela i ka nahele', 'E hahai ana paha i ka liko', 'Pua ʻāhihi lehua o uka',
      'Aloha ʻoe, aloha ʻoe', 'E ke onaona noho i ka lipo', 'One fond embrace, a hoʻi aʻe au', 'Until we meet again'],
  },
  {
    // Victor, 19 March 1918. Words by Robert Burns (1788). loc.gov/item/jukebox-28325
    id: 'pd-auld-lang-syne', title: 'Auld Lang Syne', artist: 'Peerless Quartet', durationMs: 172000, ground: '#7b2d26', mark: '#f2d0a4',
    lyrics: ['Should auld acquaintance be forgot', 'And never brought to mind?', 'Should auld acquaintance be forgot', 'And auld lang syne?',
      'For auld lang syne, my dear', 'For auld lang syne', 'We’ll tak a cup o’ kindness yet', 'For auld lang syne'],
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
  if (track.id === 'pd-danny-boy') {
    // A sun going down behind two hills, the glen between them
    g.beginPath(); g.arc(120, 138, 40, 0, Math.PI * 2); g.fill();
    g.fillStyle = '#143d27';
    g.beginPath(); g.moveTo(0, 240); g.lineTo(0, 150); g.quadraticCurveTo(70, 100, 140, 170); g.lineTo(140, 240); g.fill();
    g.beginPath(); g.moveTo(240, 240); g.lineTo(240, 140); g.quadraticCurveTo(170, 110, 100, 190); g.lineTo(100, 240); g.fill();
  } else if (track.id === 'pd-aloha-oe') {
    // The sun on the sea, its path on the water
    g.beginPath(); g.arc(120, 112, 44, Math.PI, 0); g.fill();
    for (let i = 0; i < 5; i++) g.fillRect(120 - (60 - i * 10), 124 + i * 20, (60 - i * 10) * 2, 6);
  } else {
    // Two cups raised
    for (const x of [62, 134]) { g.fillRect(x, 92, 44, 64); g.fillRect(x + 16, 156, 12, 30); g.fillRect(x + 4, 186, 36, 8); }
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
      settings: { toggleShortcut: 'Ctrl+Shift+Space', holdToTalkShortcut: 'RightAlt', reviewBeforeInsert: true }, ...extra,
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
          settings: { toggleShortcut: 'Ctrl+Shift+Space', holdToTalkShortcut: 'RightAlt', reviewBeforeInsert: true } };
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
