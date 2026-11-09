// Captures the real island (site/island/index.html?embed) into site/img/ for the landing page.
// Run after `bun run build:site` with the site served: node scripts/capture-site-shots.mjs [baseUrl]
import { chromium } from '@playwright/test';
import { mkdirSync } from 'node:fs';

const base = process.argv[2] || 'http://localhost:4173';
const out = new URL('../site/img/', import.meta.url);
mkdirSync(out, { recursive: true });
const browser = await chromium.launch({ channel: process.env.PW_CHANNEL || 'msedge' });
const page = await browser.newPage({ viewport: { width: 600, height: 300 }, deviceScaleFactor: 2 });
const island = page.locator('#isl');
const shot = async (name, settle = 700) => { await page.waitForTimeout(settle); await island.screenshot({ path: new URL(`${name}.png`, out).pathname.replace(/^\/([A-Z]:)/, '$1'), omitBackground: true }); console.log('saved', name); };
const segment = async (label) => {
  await page.waitForTimeout(400);
  const found = await page.evaluate(text => {
    const el = [...document.querySelectorAll('#isl button')].find(b => b.textContent.trim() === text);
    el?.click(); return Boolean(el);
  }, label);
  if (!found) throw new Error(`No section switch named ${label}`);
};

await page.goto(`${base}/island/index.html?embed&state=idle&quiet`);
await page.waitForTimeout(1200);
await shot('state-quiet');
await page.goto(`${base}/island/index.html?embed&state=idle`);
await page.waitForTimeout(1200);
await shot('state-idle');

await page.goto(`${base}/island/index.html?embed`);
await page.waitForTimeout(2500);
await shot('media');
await page.getByRole('tab', { name: 'Today' }).click(); await shot('today-clock');
await segment('Timer'); await page.getByRole('button', { name: 'Start a 25 minute focus session' }).click(); await page.waitForTimeout(2200); await shot('today-timer');
await page.getByRole('tab', { name: 'Dictation' }).click(); await page.getByRole('button', { name: 'Record' }).click(); await page.waitForTimeout(1200); await shot('dictation-recording');
await page.waitForTimeout(3200); await shot('dictation-review');
await page.getByRole('tab', { name: 'Tools' }).click(); await shot('tools-files');
await segment('Clipboard'); await shot('tools-clipboard');
await page.getByRole('tab', { name: 'Usage' }).click(); await shot('usage-ai');
await segment('System'); await page.waitForTimeout(1500); await shot('usage-system');
await browser.close();
