// Screenshots of every open tab and state at both island widths, driven by a mocked
// native bridge. Every value on screen is a test fixture, never a real reading.
// Run after `bun run build`: node docs/screens/shoot-layout.mjs [output folder, default artifacts/layout]
import { chromium } from '@playwright/test';
import { mkdir, readFile } from 'node:fs/promises';
import path from 'node:path';

const root = process.cwd();
const out = path.resolve(process.argv[2] ?? 'artifacts/layout');
await mkdir(out, { recursive: true });

const bridge = () => {
  window.isTauri = true;
  let serial = 0;
  const callbacks = new Map(), listeners = new Map();
  window.__shelf = [];
  window.__emitNative = (event, payload) => { for (const id of listeners.get(event) || []) callbacks.get(id)?.({ event, id, payload }); };
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
    transformCallback(callback) { const id = ++serial; callbacks.set(id, callback); return id; },
    async invoke(command, args = {}) {
      if (command === 'plugin:event|listen') { const ids = listeners.get(args.event) || []; ids.push(args.handler); listeners.set(args.event, ids); return ++serial; }
      if (command === 'extension_snapshot') return { settings: { notifications: false }, shelf: window.__shelf, notificationStatus: 'Disabled', errors: [] };
      if (command === 'media_now') return { active: false, trackId: '', title: '', artist: '', playing: false, positionMs: 0, durationMs: 0, artwork: null };
      if (command === 'clipboard_get_state') return { enabled: true, persistOnExit: false, items: [] };
      if (command === 'usage_now') return [];
      if (command === 'get_autostart' || command === 'installed_usage_enabled' || command === 'fullscreen_now') return false;
      if (command === 'get_hide_system_flyout') return { enabled: false, state: 'off', message: null };
      if (command === 'lyrics_get') window.__lyricsRequest = args.requestId;
      return null;
    },
  };
};

const artwork = () => {
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 64;
  const ctx = canvas.getContext('2d');
  ctx.fillStyle = '#91203a'; ctx.fillRect(0, 0, 64, 64);
  ctx.fillStyle = '#e8a04c'; ctx.beginPath(); ctx.arc(32, 32, 18, 0, 7); ctx.fill();
  return canvas.toDataURL();
};

const now = Math.round(Date.now() / 1000);
const window5h = (id, provider, label, usedPct) => ({ id, provider, label, usedPct, spentUsd: null, resetsAt: now + 8130, source: 'official', stale: false, status: 'ok' });
const quotaProviders = [
  window5h('claude-5h', 'claude', 'Claude session (5 h)', 42), window5h('claude-week', 'claude', 'Claude weekly', 18),
  window5h('codex-primary', 'codex', 'Codex 5 h', 72), window5h('codex-secondary', 'codex', 'Codex weekly', 9),
];
const twoProviders = quotaProviders.slice(0, 4);
const shelf = [
  { id: 'a', name: 'Quarterly review notes.pdf', path: 'C:\\Fixture\\a.pdf', bytes: 2411724 },
  { id: 'b', name: 'A long file name that keeps going well past the width of the row.png', path: 'C:\\Fixture\\b.png', bytes: 845312 },
  { id: 'c', name: 'export.csv', path: 'C:\\Fixture\\c.csv', bytes: 20480 },
];
const clips = [
  { id: 'c1', text: 'npm run build && npm test', pinned: true, createdAt: 1 },
  { id: 'c2', text: 'https://example.com/a/very/long/link/that/keeps/going/and/going/until/it/has/to/end/in/an/ellipsis', pinned: false, createdAt: 2 },
  { id: 'c3', text: 'Meeting moved to Thursday at 3pm', pinned: false, createdAt: 3 },
];
const track = (title, extra = {}) => ({ active: true, trackId: title, title, artist: 'Synthetic test data', playing: true, positionMs: 83000, durationMs: 214000, ...extra });
const lrc = Array.from({ length: 12 }, (_, i) => `[01:${String(10 + i * 5).padStart(2, '0')}]Fixture lyric line number ${i + 1}`).join('\n');

const browser = await chromium.launch({ channel: 'msedge', headless: true });
try {
  for (const width of [440, 500]) {
    const context = await browser.newContext({ viewport: { width: 520, height: 288 }, deviceScaleFactor: 2, reducedMotion: 'reduce' });
    await context.route('http://tuck.test/**', async route => {
      const pathname = new URL(route.request().url()).pathname;
      const file = path.join(root, 'dist', pathname === '/' ? 'index.html' : pathname);
      const types = { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' };
      try { await route.fulfill({ body: await readFile(file), contentType: types[path.extname(file)] || 'application/octet-stream' }); }
      catch { await route.fulfill({ status: 404, body: 'Not found' }); }
    });
    await context.addInitScript(bridge);
    await context.addInitScript(width => localStorage.setItem('tuck.live.v2', JSON.stringify({ openWidth: width, pinned: true })), width);
    const page = await context.newPage();
    page.on('pageerror', error => { console.error('Page error', error.message); process.exitCode = 1; });
    await page.goto('http://tuck.test/');
    await page.waitForTimeout(200);
    await page.evaluate(() => window.__emitNative('open-island', null));
    await page.waitForFunction(() => document.getElementById('isl').dataset.state === 'open');

    const emit = (event, payload) => page.evaluate(([event, payload]) => window.__emitNative(event, payload), [event, payload]);
    const shot = async name => { await page.waitForTimeout(80); await page.locator('#isl').screenshot({ path: path.join(out, `${name}-${width}.png`), omitBackground: true }); };
    const tab = name => page.getByRole('tab', { name, exact: true }).click();
    const part = async name => { await page.getByRole('tab', { name, exact: true }).click(); await page.waitForTimeout(60); };

    await emit('power', { statusKnown: true, percentKnown: true, hasBattery: true, percent: 82, charging: true, acConnected: true, minutesLeft: null });

    // Media
    await tab('Media');
    await shot('media-nothing-playing');
    const art = await page.evaluate(artwork);
    await emit('media', track('Artwork color fixture', { artwork: art }));
    await page.waitForTimeout(50);
    await shot('media-no-lyrics');
    await emit('media', track('A title far too long to fit on the track title row however wide the island gets', { artwork: art }));
    await page.waitForTimeout(50);
    await shot('media-long-title');
    await emit('media', track('Artwork color fixture', { artwork: art }));
    await page.waitForTimeout(50);
    await page.evaluate(lrc => window.__emitNative('lyrics', { trackId: 'Artwork color fixture', requestId: window.__lyricsRequest, status: 'synced', syncedLyrics: lrc }), lrc);
    await shot('media-lyrics');

    // Today
    await tab('Today');
    await shot('today-clock');
    await part('Timer');
    await shot('today-timer');

    // Dictation
    await tab('Dictation');
    await emit('dictation_state', { revision: 1, status: 'idle', modelInstalled: true, activeModelLabel: 'Tiny' });
    await shot('dictation-ready');
    await emit('dictation_partial', { revision: 2, status: 'recording', preview: 'Live words appear here while you speak, and they may change as the model hears more', modelInstalled: true, activeModelLabel: 'Tiny' });
    await shot('dictation-recording');
    await emit('dictation_state', { revision: 3, status: 'review', transcript: 'The finished transcript waits here for review before it is inserted into the app you were using.', modelInstalled: true, activeModelLabel: 'Tiny' });
    await shot('dictation-review');
    await emit('dictation_state', { revision: 4, status: 'cancelling', modelInstalled: true, activeModelLabel: 'Tiny' });
    await shot('dictation-cancelling');
    await emit('dictation_state', { revision: 5, status: 'error', message: 'The microphone could not be opened. Check that another app is not holding it, then try again.', modelInstalled: true, activeModelLabel: 'Tiny' });
    await shot('dictation-error');
    await emit('dictation_state', { revision: 6, status: 'idle', modelInstalled: false, activeModelLabel: null });
    await shot('dictation-no-model');

    // Tools
    await tab('Tools');
    await part('Files');
    await shot('tools-files-empty');
    await page.evaluate(() => window.dispatchEvent(new CustomEvent('tuck-drop-active', { detail: true })));
    await shot('tools-files-drag');
    await page.evaluate(() => window.dispatchEvent(new CustomEvent('tuck-drop-active', { detail: false })));
    await emit('extension-state', { settings: { notifications: false }, shelf, notificationStatus: 'Disabled', errors: [] });
    await shot('tools-files-list');
    await page.evaluate(() => window.dispatchEvent(new CustomEvent('tuck-drop-active', { detail: true })));
    await shot('tools-files-list-drag');
    await page.evaluate(() => window.dispatchEvent(new CustomEvent('tuck-drop-active', { detail: false })));
    await part('Clipboard');
    await shot('tools-clipboard-empty');
    await emit('clipboard_history_changed', { enabled: true, persistOnExit: false, items: clips });
    await shot('tools-clipboard-list');

    // Usage
    await emit('usage', twoProviders);
    await tab('Usage');
    await part('AI');
    await shot('usage-ai-two');
    await emit('usage', quotaProviders);
    await shot('usage-ai-two');
    await part('System');
    await emit('telemetry', { sampledAt: Date.now(), cpuPct: 43, ramUsedBytes: 8589934592, ramTotalBytes: 17179869184, gpus: [{ id: 'gpu0', name: 'Fixture GPU', usedPct: 12 }] });
    await shot('usage-system');
    await emit('usage', []);
    await shot('usage-system-only');
    // The floating shape keeps the same layout below its gap
    await page.evaluate(() => localStorage.setItem('notch.shape', 'float'));
    await page.reload();
    await page.waitForTimeout(200);
    await page.evaluate(() => window.__emitNative('open-island', null));
    await page.waitForFunction(() => document.getElementById('isl').dataset.state === 'open');
    await tab('Tools');
    await shot('float-tools');
    await context.close();
  }
  console.log(`Saved to ${out}`);
} finally { await browser.close(); }
