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
  assert.equal(Math.round(box.width),500); assert.equal(Math.round(box.height),210);
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
  await page.evaluate(() => { Storage.prototype.setItem = () => { throw new Error('Storage disabled'); }; });
  await page.locator('#saveBudgets').click();
  assert.match(await page.locator('#settingsStatus').textContent(),/Could not save budgets/);
  assert.equal(await page.locator('#s1').evaluate(el => el.classList.contains('show')),false);
  assert.deepEqual(errors,[]);

  // A mocked native bridge exercises the actual controllers; this is not a hardware smoke.
  await context.addInitScript(() => {
    window.isTauri=true;
    let serial=0;const callbacks=new Map();const listeners=new Map();
    const state={settings:{downloads:false,calendar:false,notifications:false,radios:true},downloads:[],call:null,calendar:[],shelf:[],radios:[{kind:'wifi',name:'Test Wi-Fi',state:'On'}],notificationStatus:'Disabled',errors:[]};
    window.__nativeState=state;window.__calls=[];
    window.__emitNative=(event,payload)=>{for(const id of listeners.get(event)||[])callbacks.get(id)?.({event,id,payload});};
    window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},transformCallback(callback){const id=++serial;callbacks.set(id,callback);return id;},async invoke(command,args={}){
      window.__calls.push({command,args});
      if(command==='plugin:event|listen'){const ids=listeners.get(args.event)||[];ids.push(args.handler);listeners.set(args.event,ids);return ++serial;}
      if(command==='extension_snapshot')return structuredClone(state);
      if(command==='call_set_active'){state.call=args.active?{title:'Manual call timer',startedAt:Date.now()}:null;return;}
      if(command==='shelf_pick'){state.shelf=[{id:'fixture-file',name:'sample.txt',path:'C:\\Fixture\\sample.txt',bytes:20}];return;}
      if(command==='radio_set'){state.radios[0].state=args.enabled?'On':'Off';return;}
      if(command==='extension_settings')throw 'Activity settings could not be saved';
      if(command==='media_now')return {active:false,trackId:'',title:'',artist:'',playing:false,positionMs:0,durationMs:0,artwork:null};
      if(command==='usage_now')return [];
      if(command==='get_autostart'||command==='installed_usage_enabled'||command==='fullscreen_now')return false;
      return null;
    }};
  });
  const native=await context.newPage();const nativeErrors=[];native.on('pageerror',error=>nativeErrors.push(error.message));
  await native.goto('http://tuck.test/');await native.waitForTimeout(100);
  await native.evaluate(()=>window.__emitNative('open-island',null));
  for(let index=0;index<5;index++)await native.locator('#nextPage').click();
  assert.equal(await native.locator('.page.on').getAttribute('data-page'),'extensions');
  await native.getByRole('button',{name:'Start call timer',exact:true}).click();
  await native.getByRole('button',{name:'End timer',exact:true}).waitFor({timeout:3000});
  assert.equal(await native.getByRole('button',{name:'End timer',exact:true}).count(),1);
  assert.equal(await native.locator('.ext-note').textContent(),'Manual call timer. This does not place calls or control your microphone.');
  await native.keyboard.press('Escape');
  await native.evaluate(()=>window.__emitNative('hover',true));
  await native.waitForTimeout(250);
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open','Hover expands after the configured delay');
  assert.equal(await native.evaluate(()=>window.__calls.some(call=>call.command==='deactivate_island')),true);
  await native.evaluate(()=>window.__emitNative('open-island',null));
  await native.getByRole('button',{name:'Files',exact:true}).click();
  await native.getByRole('button',{name:'Add files',exact:true}).click();
  assert.match(await native.locator('.ext-body').textContent(),/sample.txt/);
  await native.getByRole('button',{name:'Radios',exact:true}).click();
  await native.getByRole('button',{name:'Turn off',exact:true}).focus();
  await native.evaluate(()=>{window.__nativeState.radios[0].state='Off';window.__emitNative('extension-state',structuredClone(window.__nativeState));});
  assert.equal(await native.getByRole('button',{name:'Turn on',exact:true}).count(),1);
  assert.equal(await native.evaluate(()=>document.activeElement.dataset.actionKey),'radio-wifi-Test Wi-Fi');
  await native.evaluate(()=>window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:true})));
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open');
  await native.evaluate(()=>window.__emitNative('hover',false));await native.waitForTimeout(1750);
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open','Native drag must hold the island open');
  await native.evaluate(()=>window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:false})));
  await native.keyboard.press('Escape');
  await native.evaluate(()=>window.__emitNative('notification-preview',{title:'Fixture notification',body:'Preview text'}));
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'peek');
  assert.match(await native.locator('#cc').textContent(),/Fixture notification/);
  await native.screenshot({path:path.join(root,'artifacts','island-v1.png')});
  // Phase-two presentation uses synthetic bridge fixtures, never a live-player claim.
  await native.evaluate(()=>window.__emitNative('open-island',null));
  await native.getByRole('button',{name:'Go to media page',exact:true}).click();
  await native.evaluate(()=>{
    const canvas=document.createElement('canvas');canvas.width=32;canvas.height=32;
    const ctx=canvas.getContext('2d');ctx.fillStyle='#91203a';ctx.fillRect(0,0,32,32);
    window.__emitNative('media',{active:true,trackId:'fixture-track',title:'Artwork color fixture',artist:'Synthetic test data',playing:false,positionMs:3000,durationMs:120000,artwork:canvas.toDataURL()});
  });
  await native.waitForTimeout(100);
  await native.evaluate(()=>window.__emitNative('lyrics',{trackId:'fixture-track',status:'synced',syncedLyrics:'[00:01]Previous fixture line\n[00:03]Current fixture line\n[00:05]Next fixture line'}));
  assert.equal(await native.locator('.lyric-line.current').textContent(),'Current fixture line');
  assert.match(await native.locator('#isl').getAttribute('style'),/--art-wash: rgba/);
  await native.locator('.lyric-line').last().click();
  assert.equal(await native.evaluate(()=>window.__calls.some(call=>call.command==='media_seek'&&call.args.ms===5000)),true);
  await native.screenshot({path:path.join(root,'artifacts','island-phase-two-media.png')});
  await native.getByRole('button',{name:'Go to stats page',exact:true}).click();
  await native.evaluate(()=>window.__emitNative('telemetry',{sampledAt:Date.now(),cpuPct:43,ramUsedBytes:8589934592,ramTotalBytes:17179869184,gpus:[{id:'gpu0',name:'Fixture GPU',usedPct:null}]}));
  assert.match(await native.locator('.page.stats').textContent(),/43%/);
  assert.match(await native.locator('.page.stats').textContent(),/8.0 GB \/ 16.0 GB/);
  assert.match(await native.locator('.page.stats').textContent(),/Fixture GPUUnavailable/);
  await native.evaluate(()=>window.__emitNative('media',{active:true,trackId:'other',title:'Other fixture',artist:'Test',playing:true,positionMs:0,durationMs:10000}));
  assert.equal(await native.locator('.page.on').getAttribute('data-page'),'stats','Track changes must not steal a selected tool');
  await native.getByRole('button',{name:'Go to dictation page',exact:true}).click();
  await native.evaluate(()=>window.__emitNative('dictation',{status:'review',transcript:'<b>Text, not markup</b>',modelReady:true}));
  assert.equal(await native.locator('.transcript').textContent(),'<b>Text, not markup</b>');
  assert.equal(await native.locator('.transcript b').count(),0);
  await native.getByRole('button',{name:'Go to clipboard page',exact:true}).click();
  await native.evaluate(()=>window.__emitNative('clipboard-history',{enabled:true,items:[{id:'clip',text:'<img src=x>clipboard fixture',pinned:false,createdAt:Date.now()}]}));
  assert.equal(await native.locator('.clipboard-text img').count(),0);
  await native.locator('.clipboard-text').click();
  assert.equal(await native.evaluate(()=>window.__calls.some(call=>call.command==='clipboard_copy'&&call.args.id==='clip')),true);
  assert.deepEqual(nativeErrors,[]);
  console.log('UI smoke passed: geometry, accessibility, keyboard seeking, timers, persistence errors, native-bridge call priority, shelf, focused radio updates, drag hold and previews.');
} finally { await browser.close(); }
