// Screenshots for the island feedback PR: synthetic fixtures through a mocked native
// bridge, in a browser. Run after `bun run build`: node docs/screens/shoot-island-feedback.mjs
import { chromium } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import path from 'node:path';

const root = process.cwd();
const out = path.join(root, 'docs', 'screens');
const types = { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' };
let browser;
try { browser = await chromium.launch({ channel: 'msedge', headless: true }); } catch { browser = await chromium.launch({ headless: true }); }

async function open(prefs) {
  const context = await browser.newContext({ viewport: { width: 560, height: 260 }, deviceScaleFactor: 2, reducedMotion: 'reduce' });
  await context.addInitScript(value => localStorage.setItem('tuck.live.v2', value), JSON.stringify(prefs));
  await context.addInitScript(() => {
    window.isTauri = true;
    let serial = 0; const callbacks = new Map(); const listeners = new Map();
    window.__calls = [];
    window.__emitNative = (event, payload) => { for (const id of listeners.get(event) || []) callbacks.get(id)?.({ event, id, payload }); };
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
      transformCallback(callback) { const id = ++serial; callbacks.set(id, callback); return id; },
      async invoke(command, args = {}) {
        window.__calls.push({ command, args });
        if (command === 'plugin:event|listen') { const ids = listeners.get(args.event) || []; ids.push(args.handler); listeners.set(args.event, ids); return ++serial; }
        if (command === 'media_now') return { active: false, trackId: '', title: '', artist: '', playing: false, positionMs: 0, durationMs: 0, artwork: null };
        if (command === 'usage_now') return [];
        if (command === 'telemetry_snapshot') return { sampledAt: Date.now(), cpuPct: 23, ramUsedBytes: 11.6 * 1073741824, ramTotalBytes: 16 * 1073741824, gpus: [{ id: 'gpu0', name: 'Fixture GPU', usedPct: 41 }] };
        if (command === 'fullscreen_now') return false;
        if (command === 'extension_snapshot') return { settings: {}, downloads: [], call: null, calendar: [], shelf: [], radios: [], notificationStatus: 'Disabled', errors: [] };
        if (command === 'clipboard_get_state') return { enabled: false, persistOnExit: false, items: [] };
        return null;
      },
    };
  });
  await context.route('http://tuck.test/**', async route => {
    const pathname = new URL(route.request().url()).pathname;
    const file = path.join(root, 'dist', pathname === '/' ? 'index.html' : pathname);
    try { await route.fulfill({ body: await readFile(file), contentType: types[path.extname(file)] || 'application/octet-stream' }); }
    catch { await route.fulfill({ status: 404, body: 'nf' }); }
  });
  const page = await context.newPage();
  await page.goto('http://tuck.test/'); await page.waitForTimeout(200);
  await page.evaluate(() => {
    document.body.classList.add('dev-mode');
    window.__emitNative('power', { percent: 82, charging: false, hasBattery: true, minutesLeft: 240, statusKnown: true, percentKnown: true, acConnected: false });
  });
  return { context, page };
}

const track = { active: true, trackId: 'fixture-track', title: 'A synthetic track title', artist: 'Fixture artist', durationMs: 240000, artwork: null };
const art = () => {
  const canvas = document.createElement('canvas'); canvas.width = canvas.height = 32;
  const ctx = canvas.getContext('2d'); const g = ctx.createLinearGradient(0, 0, 32, 32);
  g.addColorStop(0, '#d9486f'); g.addColorStop(1, '#3b2a8c'); ctx.fillStyle = g; ctx.fillRect(0, 0, 32, 32);
  return canvas.toDataURL();
};

// Collapsed: lyric, title, artwork, long lyric, paused
{
  const { context, page } = await open({ compactContent: 'lyrics' });
  const artwork = await page.evaluate(art);
  await page.evaluate(([t, artwork]) => window.__emitNative('media', { ...t, playing: true, positionMs: 3000, artwork }), [track, artwork]);
  await page.waitForTimeout(150);
  const requestId = await page.evaluate(() => window.__calls.findLast(call => call.command === 'lyrics_get').args.requestId);
  await page.evaluate(id => window.__emitNative('lyrics', { trackId: 'fixture-track', requestId: id, status: 'synced', syncedLyrics: '[00:01]Line before\n[00:03]A synthetic lyric line\n[00:06]A much longer synthetic lyric line that cannot fit in the pill at all' }), requestId);
  await page.waitForTimeout(350);
  await page.screenshot({ path: path.join(out, 'collapsed-lyric.png') });
  await page.evaluate(t => window.__emitNative('media', { ...t, playing: true, positionMs: 6000 }), track);
  await page.waitForTimeout(350);
  await page.screenshot({ path: path.join(out, 'collapsed-long-lyric.png') });
  const setPref = update => page.evaluate(([key, update]) => { localStorage.setItem(key, JSON.stringify({ ...JSON.parse(localStorage.getItem(key)), ...update })); window.dispatchEvent(new StorageEvent('storage', { key })); }, ['tuck.live.v2', update]);
  await setPref({ compactContent: 'title' }); await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(out, 'collapsed-title.png') });
  await setPref({ compactContent: 'artwork' }); await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(out, 'collapsed-artwork.png') });
  await page.evaluate(t => window.__emitNative('media', { ...t, playing: false, positionMs: 3000 }), track);
  await setPref({ compactContent: 'lyrics' }); await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(out, 'collapsed-paused.png') });
  await context.close();
}

const now = () => Math.round(Date.now() / 1000);
const claude = n => [
  { id: 'claude-5h', provider: 'claude', label: 'Claude session (5 h)', usedPct: 42, spentUsd: null, resetsAt: n + 8130, source: 'official', stale: false, status: 'ok' },
  { id: 'claude-week', provider: 'claude', label: 'Claude weekly', usedPct: 18, spentUsd: null, resetsAt: n + 172800, source: 'official', stale: false, status: 'ok' },
];
const codex = n => [
  { id: 'codex-primary', provider: 'codex', label: 'Codex 5 h', usedPct: 93, spentUsd: null, resetsAt: n + 3630, source: 'official', stale: false, status: 'ok' },
  { id: 'codex-secondary', provider: 'codex', label: 'Codex weekly', usedPct: 74, spentUsd: null, resetsAt: n + 432000, source: 'official', stale: false, status: 'ok' },
];
const showUsage = async page => {
  await page.locator('#isl').focus(); await page.keyboard.press('Enter');
  await page.locator('#tab-usage').click(); await page.waitForTimeout(400);
};

for (const width of [500, 440]) {
  const { context, page } = await open({ pinned: true, openWidth: width, lastTab: 'usage' });
  await showUsage(page);
  await page.evaluate(([a, b]) => window.__emitNative('usage', [...a, ...b]), [claude(now()), codex(now())]);
  await page.waitForTimeout(400);
  await page.screenshot({ path: path.join(out, `usage-two-providers-${width}.png`) });
  if (width === 500) {
    await page.evaluate(a => window.__emitNative('usage', a), claude(now())); await page.waitForTimeout(300);
    await page.screenshot({ path: path.join(out, 'usage-one-provider.png') });
    await page.evaluate(() => window.__emitNative('usage', []));
    await page.waitForTimeout(1500);
    await page.screenshot({ path: path.join(out, 'usage-none-connected.png') });
  }
  const stale = codex(now()).map(m => ({ ...m, stale: true, status: 'error', error: 'Installed client usage request timed out' }));
  await page.evaluate(([a, b, n]) => window.__emitNative('usage', [...a, ...b,
    { id: 'anthropic-api', provider: 'anthropic-api', label: 'Claude API', usedPct: null, spentUsd: 12.4, resetsAt: n + 86400, source: 'api', stale: false, status: 'ok' },
    { id: 'openai-api', provider: 'openai-api', label: 'OpenAI API', usedPct: null, spentUsd: null, resetsAt: n + 86400, source: 'api', stale: true, status: 'error', error: 'Could not reach the provider' },
  ]), [claude(now()), stale, now()]);
  await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(out, `usage-four-with-stale-${width}.png`) });
  await page.evaluate(() => { localStorage.setItem('notch.chatgpt', JSON.stringify({ label: 'ChatGPT (estimate)', cap: 40, hours: 3 })); window.dispatchEvent(new StorageEvent('storage', { key: 'notch.chatgpt' })); });
  await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(out, `usage-five-dense-${width}.png`) });
  await context.close();
}

// Indicator focused by keyboard
{
  const { context, page } = await open({ pinned: true, lastTab: 'usage' });
  await showUsage(page);
  await page.evaluate(a => window.__emitNative('usage', a), claude(now())); await page.waitForTimeout(200);
  await page.locator('.section-index-button').first().focus();
  await page.keyboard.press('Tab'); await page.waitForTimeout(200);
  await page.screenshot({ path: path.join(out, 'indicator-focused.png') });
  await page.locator('#tab-tools').click(); await page.waitForTimeout(300);
  await page.locator('.section-index-button').nth(1).hover(); await page.waitForTimeout(200);
  await page.screenshot({ path: path.join(out, 'indicator-hover-tools.png') });
  await context.close();
}
await browser.close();
