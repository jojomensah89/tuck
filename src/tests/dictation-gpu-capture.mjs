import {chromium} from '@playwright/test';
import {readFile,mkdir} from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
const phase=process.argv[2]||'after';
const browser=await chromium.launch({channel:'msedge',headless:true});
try {const page=await browser.newPage({viewport:{width:1000,height:740},reducedMotion:'reduce'});
await page.route('http://tuck.test/**',async route=>{const p=new URL(route.request().url()).pathname;try{await route.fulfill({body:await readFile(path.join('dist',p)),contentType:p.endsWith('.html')?'text/html':p.endsWith('.js')?'application/javascript':p.endsWith('.css')?'text/css':'image/svg+xml'});}catch{await route.fulfill({status:404});}});
await page.addInitScript(() => {
 window.__TAURI_INTERNALS__ = {transformCallback: () => 1, async invoke(command) {
  if(command === 'dictation_get_state') return {backend:'GPU: NVIDIA GeForce RTX 3060 Laptop GPU', status:'idle', modelInstalled:true, activeModelLabel:'Parakeet Unified English', settings:{toggleShortcut:'Ctrl+Shift+Space',holdToTalkShortcut:'Left+RightShift',reviewBeforeInsert:false}, models:[{id:'parakeet-unified-en-0.6b-Q8_0.gguf',label:'Parakeet Unified English',bytes:731357568,state:'found',active:true,speed:'Not measured',accuracy:'English only'}]};
  return null;
 }};
});
await page.goto('http://tuck.test/settings.html#tools');
await page.locator('#dictationBackend').filter({hasText:'GPU: NVIDIA'}).waitFor();
await mkdir('docs/screens',{recursive:true});
await page.screenshot({path:'docs/screens/dictation-gpu-settings.png',fullPage:true});
assert.match(await page.locator('#dictationBackend').textContent(), /GPU: NVIDIA/);
} finally {await browser.close();}
