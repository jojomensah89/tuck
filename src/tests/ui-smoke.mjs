import { chromium } from '@playwright/test';
import { readFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
const root = process.cwd();
const browser = await chromium.launch({channel:'msedge',headless:true});
try {
  const context = await browser.newContext({viewport:{width:520,height:230}, reducedMotion:'reduce'});
  await context.route('http://tuck.test/**', async route => {
    const pathname = new URL(route.request().url()).pathname;
    const file = path.join(root, 'dist', pathname === '/' ? 'index.html' : pathname);
    const types = {'.html':'text/html','.js':'application/javascript','.css':'text/css','.svg':'image/svg+xml'};
    try { await route.fulfill({body:await readFile(file),contentType:types[path.extname(file)] || 'application/octet-stream'}); }
    catch { await route.fulfill({status:404,body:'Not found'}); }
  });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror',error => errors.push(error.message));
  await page.goto('http://tuck.test/');
  await page.waitForTimeout(200);
  assert.equal(await page.locator('.full').evaluate(el => el.inert), true);
  await page.locator('#isl').focus();
  await page.keyboard.press('Enter');
  assert.equal(await page.locator('#isl').getAttribute('data-state'),'open');
  assert.equal(await page.locator('.full').getAttribute('aria-hidden'),'false');
  const box = await page.locator('#isl').boundingBox();
  assert.equal(Math.round(box.width),440); assert.equal(Math.round(box.height),180);
  await page.locator('#track').focus();
  await page.keyboard.press('ArrowRight');
  assert.equal(await page.locator('.page.on').getAttribute('data-page'),'media');
  await page.locator('#nextPage').click();
  await page.locator('#nextPage').click();
  assert.equal(await page.locator('.page.on').getAttribute('data-page'),'activities');
  const overflow = await page.locator('.page.activities').evaluate(el => {
    const box = el.getBoundingClientRect();
    return [...el.querySelectorAll('button')].some(button => button.getBoundingClientRect().bottom > box.bottom + 1);
  });
  assert.equal(overflow,false,'Activity controls must fit');
  await page.locator('#actQuick5').click();
  assert.equal(await page.locator('#actStatus').textContent(),'Active');
  await page.locator('#actPause').click();
  assert.equal(await page.locator('#actPause').textContent(),'Resume');
  await mkdir(path.join(root,'artifacts'),{recursive:true});
  await page.screenshot({path:path.join(root,'artifacts','island-activities.png')});
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('.full').evaluate(el => el.inert),true);
  await page.goto('http://tuck.test/settings.html');
  await page.locator('#kAnthropic').fill('test-placeholder-not-a-key');
  await page.locator('#saveKeys').click();
  assert.equal(await page.locator('#kAnthropic').inputValue(),'');
  assert.match(await page.locator('#settingsStatus').textContent(),/failed/);
  await page.locator('#removeOpenai').click();
  assert.match(await page.locator('#settingsStatus').textContent(),/failed/);
  await page.locator('#cCap').fill('-1');
  await page.locator('#cHours').fill('3');
  await page.locator('#saveChat').click();
  assert.match(await page.locator('#settingsStatus').textContent(),/positive/);
  await page.locator('#pickFloat').click();
  assert.equal(await page.evaluate(() => localStorage.getItem('notch.shape')),'float');
  assert.deepEqual(errors,[]);
  console.log('UI smoke passed: geometry, inert/ARIA, keyboard seeking, timer controls, credential failures, settings validation, persistence.');
} finally { await browser.close(); }
