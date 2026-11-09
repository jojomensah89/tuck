import { chromium } from '@playwright/test';
import { readFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
const browser = await chromium.launch({channel:'msedge',headless:true});
try {
 const context = await browser.newContext({viewport:{width:980,height:800},reducedMotion:'reduce'});
 await context.route('http://tuck.test/**',async route=>{
  const name = new URL(route.request().url()).pathname;
  const file=path.join(process.cwd(),'dist',name);
  const types={'.html':'text/html','.js':'application/javascript','.css':'text/css'};
  try { await route.fulfill({body:await readFile(file),contentType:types[path.extname(file)]||'application/octet-stream'}); } catch { await route.fulfill({status:404,body:''}); }
 });
 await context.addInitScript(()=>{
  window.__calls=[];
  window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'settings'},currentWebview:{label:'settings'}},transformCallback(){return 1;},async invoke(cmd){
   window.__calls.push(cmd);
   if(cmd==='is_store_build')return true;
   if(cmd==='autostart_status')return 'Turned off in Task Manager > Startup apps';
   if(cmd==='get_autostart')return false;
   if(cmd==='installed_usage_providers'||cmd==='usage_now')return [];
   if(cmd==='dictation_get_state')return {phase:'idle',settings:{},models:[]};
   if(cmd==='extension_snapshot')return {settings:{},shelf:[],notifications:[]};
   return null;
  }};
 });
 const page=await context.newPage();
 await page.goto('http://tuck.test/settings.html#about');
 await page.getByText('Updates come from the Microsoft Store',{exact:true}).waitFor();
 assert.equal(await page.locator('#btnCheckUpdates').isVisible(),false);
 assert.equal(await page.evaluate(()=>window.__calls.includes('update_check')),false);
 await mkdir('docs/screens/msix',{recursive:true});
 await page.screenshot({path:'docs/screens/msix/about.png'});
 await page.goto('http://tuck.test/settings.html#general');
 await page.getByText('Turned off in Task Manager > Startup apps',{exact:true}).waitFor({state:'attached'});
 // Select the view containing the actual startup switch.
 await page.evaluate(()=>document.querySelector('[data-view="general"]')?.click());
 await page.screenshot({path:'docs/screens/msix/startup.png'});
 console.log('MSIX Settings assertions passed (mock native package identity and startup state)');
} finally { await browser.close(); }

