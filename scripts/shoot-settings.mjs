import { chromium } from '@playwright/test';
import { readFile, mkdir } from 'node:fs/promises';
import path from 'node:path';

const root = process.cwd();
const out = path.join(root, 'docs', 'screens');
await mkdir(out, { recursive: true });

let browser;
try {
  browser = await chromium.launch({ channel: 'msedge', headless: true });
} catch {
  browser = await chromium.launch({ headless: true });
}

const types = {
  '.html': 'text/html',
  '.js': 'application/javascript',
  '.css': 'text/css',
  '.svg': 'image/svg+xml',
  '.json': 'application/json',
};

const context = await browser.newContext({
  viewport: { width: 840, height: 620 },
  deviceScaleFactor: 2,
});

await context.route('http://tuck.test/**', async (route) => {
  const pathname = new URL(route.request().url()).pathname;
  const file = path.join(root, 'dist', pathname === '/' ? 'index.html' : pathname);
  try {
    await route.fulfill({
      body: await readFile(file),
      contentType: types[path.extname(file)] || 'application/octet-stream',
    });
  } catch {
    await route.fulfill({ status: 404, body: 'Not found' });
  }
});

// A minimal native bridge, so the screenshot comes from the real update code path
await context.addInitScript(() => {
  let serial = 0;
  const callbacks = new Map();
  const listeners = new Map();
  window.isTauri = true;
  window.__emitNative = (event, payload) => {
    for (const id of listeners.get(event) || []) callbacks.get(id)?.({ event, id, payload });
  };
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'settings' }, currentWebview: { label: 'settings' } },
    transformCallback(callback) {
      const id = ++serial;
      callbacks.set(id, callback);
      return id;
    },
    async invoke(command, args = {}) {
      if (command === 'plugin:event|listen') {
        listeners.set(args.event, [...(listeners.get(args.event) || []), args.handler]);
        return ++serial;
      }
      return null;
    },
  };
});

const page = await context.newPage();
await page.goto('http://tuck.test/settings.html#about');
await page.waitForTimeout(400);

await page.evaluate(() => {
  window.__emitNative('pending-update-changed', {
    version: '0.2.0',
    currentVersion: '0.1.0',
    body: '### Added\n- Release pipeline, in-app updater and full tray menu\n\n### Fixed\n- Example fix shown only in this screenshot',
  });
});

await page.waitForTimeout(300);
await page.screenshot({ path: path.join(out, 'settings-updates.png') });
console.log('Saved docs/screens/settings-updates.png');
await browser.close();

