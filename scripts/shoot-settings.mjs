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

const page = await context.newPage();
await page.goto('http://tuck.test/settings.html#about');
await page.waitForTimeout(400);

await page.evaluate(() => {
  const row = document.getElementById('row-update-available');
  if (row) {
    row.hidden = false;
    const badge = document.getElementById('updateVersionBadge');
    if (badge) badge.textContent = 'v0.2.0';
    const notes = document.getElementById('updateNotes');
    if (notes) {
      notes.textContent =
        'Features:\n- Automated release pipeline with NSIS installer and updater artifacts\n- Full 8-item tray menu with left-click toggle and autostart synchronization\n- In-app updater with quiet background polling and manual check/install';
    }
  }
  const status = document.getElementById('updateStatusText');
  if (status) status.textContent = 'Update to v0.2.0 is available.';
});

await page.waitForTimeout(300);
const aboutView = page.locator('#view-about');
await aboutView.screenshot({ path: path.join(out, 'settings-updates.png') });
console.log('Saved screenshot to docs/screens/settings-updates.png');
await browser.close();
