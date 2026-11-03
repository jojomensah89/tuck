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
  assert.equal(await page.locator('.tab-panel.on').getAttribute('data-tab'),'media');
  // Rail: four tabs with roving tabindex plus a settings button, no arrows or dots
  assert.equal(await page.locator('[role=tablist] [role=tab]').count(),4);
  assert.equal(await page.locator('#prevPage, #nextPage, #dots').count(),0);
  assert.deepEqual(await page.locator('[role=tab]').evaluateAll(tabs => tabs.map(t => [t.getAttribute('aria-selected'), t.tabIndex])),[['true',0],['false',-1],['false',-1],['false',-1]]);
  const railGeometry = await page.locator('.rail-tab').evaluateAll(nodes => nodes.map(n => { const r = n.getBoundingClientRect(); return [r.width, r.height]; }));
  assert.equal(railGeometry.every(([w,h]) => w >= 36 && h >= 36),true,'Rail hit targets are at least 36px');
  await page.locator('#tab-media').focus();
  await page.keyboard.press('End');
  assert.equal(await page.locator('#tab-usage').getAttribute('aria-selected'),'true');
  assert.equal(await page.evaluate(() => document.activeElement.id),'tab-usage','Roving focus follows the selected tab');
  await page.keyboard.press('ArrowRight');
  assert.equal(await page.locator('.tab-panel.on').getAttribute('data-tab'),'media','Rail arrows wrap');
  await page.keyboard.press('Home');
  await page.keyboard.press('ArrowDown');
  assert.equal(await page.locator('.tab-panel.on').getAttribute('data-tab'),'today');
  assert.equal(await page.locator('#panel-today').getAttribute('aria-labelledby'),'tab-today');
  // Today holds clock and battery, then the timer, then the calendar
  assert.deepEqual(await page.locator('#panel-today > .page').evaluateAll(n => n.map(x => x.dataset.page)),['today','activities','calendar']);
  await page.locator('.page.activities').scrollIntoViewIfNeeded();
  const overflow = await page.locator('.page.activities').evaluate(el => {
    const box = el.getBoundingClientRect();
    return [...el.querySelectorAll('button')].some(button => button.getBoundingClientRect().bottom > box.bottom + 1);
  });
  assert.equal(overflow,false,'Activity controls must fit');
  // Timer: numeric presets, icon controls, a status dot and a progress ring
  const timerTargets = await page.locator('.page.activities button').evaluateAll(nodes => nodes.map(n => { const r = n.getBoundingClientRect(); return Math.min(r.width, r.height); }));
  assert.equal(timerTargets.every(size => size >= 32), true, 'Timer controls are at least 32px');
  assert.equal(await page.locator('.page.activities .act-face .ring').count(), 1);
  await page.locator('#actQuick5').click();
  assert.equal(await page.locator('#actStatus').getAttribute('data-status'),'running');
  assert.equal(await page.locator('#actQuick5').getAttribute('aria-pressed'),'true');
  await page.locator('#actPause').click();
  assert.equal(await page.locator('#actPause').getAttribute('aria-label'),'Resume');
  assert.equal(await page.locator('#actStatus').getAttribute('aria-label'),'Paused');
  await page.locator('#actReset').click();
  assert.equal(await page.locator('#actStatus').getAttribute('data-status'),'ready');
  assert.equal(await page.locator('#actReset').isDisabled(),true,'Nothing to reset when idle');
  await page.locator('#actPause').click();
  assert.equal(await page.locator('#actStatus').getAttribute('data-status'),'running','Play starts the last preset when idle');
  await page.locator('#actPause').click();
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
      if(command==='shelf_pick'){state.shelf=[{id:'fixture-file',name:'sample.txt',path:'C:\\\\Fixture\\\\sample.txt',bytes:20}];return;}
      if(command==='radio_set'){state.radios[0].state=args.enabled?'On':'Off';return;}
      if(command==='extension_settings')throw 'Activity settings could not be saved';
      if(command==='media_now')return {active:false,trackId:'',title:'',artist:'',playing:false,positionMs:0,durationMs:0,artwork:null};
      if(command==='usage_now')return [];
      if(command==='clipboard_get_state')return {enabled:false,persistOnExit:false,items:[]};
      if(command==='clipboard_configure')throw 'Fixture atomic save failure';
      if(command==='get_autostart'||command==='installed_usage_enabled'||command==='fullscreen_now')return false;
      return null;
    }};
  });
  const native=await context.newPage();const nativeErrors=[];native.on('pageerror',error=>nativeErrors.push(error.message));
  await native.goto('http://tuck.test/');await native.waitForTimeout(100);
  assert.equal(await native.locator('.page.todos').count(), 0, 'Todos feature must be completely removed');
  await native.evaluate(()=>window.__emitNative('open-island',null));
  await native.getByRole('tab',{name:'Tools',exact:true}).click();
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'tools');
  await native.getByRole('button',{name:'Calls',exact:true}).click();
  await native.getByRole('button',{name:'Start call timer',exact:true}).click();
  await native.getByRole('button',{name:'End call timer',exact:true}).waitFor({timeout:3000});
  assert.equal(await native.getByRole('button',{name:'End call timer',exact:true}).count(),1);
  assert.equal(await native.locator('#extensionsPage .ext-note').textContent(),'Manual timer, it does not place calls or use the microphone');
  assert.equal(await native.getByRole('button',{name:'Calls',exact:true}).locator('.ext-badge.dot').isVisible(),true,'A running call timer badges its view');
  await native.keyboard.press('Escape');
  await native.evaluate(()=>window.__emitNative('hover',true));
  await native.waitForTimeout(250);
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open','Hover expands after the configured delay');
  assert.equal(await native.evaluate(()=>window.__calls.some(call=>call.command==='deactivate_island')),true);
  await native.evaluate(()=>window.__emitNative('open-island',null));
  await native.getByRole('button',{name:'Files',exact:true}).click();
  await native.getByRole('button',{name:'Add files',exact:true}).click();
  assert.match(await native.locator('#extensionsPage .ext-body').textContent(),/sample.txt/);
  await native.getByRole('button',{name:'Radios',exact:true}).click();
  assert.equal(await native.getByRole('switch',{name:'Test Wi-Fi',exact:true}).getAttribute('aria-checked'),'true');
  await native.getByRole('switch',{name:'Test Wi-Fi',exact:true}).focus();
  await native.evaluate(()=>{window.__nativeState.radios[0].state='Off';window.__emitNative('extension-state',structuredClone(window.__nativeState));});
  assert.equal(await native.getByRole('switch',{name:'Test Wi-Fi',exact:true}).getAttribute('aria-checked'),'false');
  assert.equal(await native.evaluate(()=>document.activeElement.dataset.actionKey),'radio-wifi-Test Wi-Fi');
  await native.evaluate(()=>window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:true})));
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open');
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'tools','A file drag reveals the file shelf');
  assert.equal(await native.locator('#extensionsPage').evaluate(el => { const p = el.closest('.tab-panel').getBoundingClientRect(), r = el.getBoundingClientRect(); return Math.abs(r.top - p.top) < 2; }),true,'The shelf section is scrolled into view');
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
  await native.getByRole('tab',{name:'Media',exact:true}).click();
  await native.evaluate(()=>{
    const canvas=document.createElement('canvas');canvas.width=32;canvas.height=32;
    const ctx=canvas.getContext('2d');ctx.fillStyle='#91203a';ctx.fillRect(0,0,32,32);
    window.__emitNative('media',{active:true,trackId:'fixture-track',title:'Artwork color fixture',artist:'Synthetic test data',playing:false,positionMs:3000,durationMs:120000,artwork:canvas.toDataURL()});
  });
  await native.waitForTimeout(100);
  await native.evaluate(()=>window.__emitNative('lyrics',{trackId:'fixture-track',requestId:window.__calls.findLast(call=>call.command==='lyrics_get').args.requestId,status:'synced',syncedLyrics:'[00:01]Previous fixture line\n[00:03]Current fixture line\n[00:05]Next fixture line'}));
  assert.equal(await native.locator('.lyric-line.current').textContent(),'Current fixture line');
  assert.match(await native.locator('#isl').getAttribute('style'),/--art-wash: rgba/);
  await native.locator('.lyric-line').last().click();
  assert.equal(await native.evaluate(()=>window.__calls.some(call=>call.command==='media_seek'&&call.args.ms===5000)),true);
  await native.screenshot({path:path.join(root,'artifacts','island-phase-two-media.png')});
  for (const width of [440, 500]) {
    await native.evaluate(width => {
      const key = 'tuck.live.v2';
      localStorage.setItem(key, JSON.stringify({...JSON.parse(localStorage.getItem(key) || '{}'), openWidth: width}));
      window.dispatchEvent(new StorageEvent('storage', {key}));
    }, width);
    const geometry = await native.evaluate(() => {
      const rect = selector => document.querySelector(selector).getBoundingClientRect();
      const title = rect('#title'), lyrics = rect('.lyrics-pane');
      const toggle = rect('button[aria-label="Toggle lyrics"]');
      const controls = rect('.page.media .ctl'), page = rect('.page.media');
      return {
        width: rect('#isl').width,
        titleClear: title.right + 12 <= lyrics.left,
        togglePresent: toggle.width >= 32 && toggle.height >= 32,
        toggleClear: toggle.right <= rect('#prev').left,
        controlsFit: controls.bottom <= page.bottom + 1,
      };
    });
    assert.equal(geometry.width, width);
    assert.equal(geometry.titleClear && geometry.togglePresent && geometry.toggleClear && geometry.controlsFit, true, `Media regions must fit at ${width}px`);
    await native.screenshot({path:path.join(root,'artifacts',`island-unslop-${width}.png`)});
  }
  await native.evaluate(()=>{
    const oldId=window.__calls.findLast(call=>call.command==='lyrics_get').args.requestId;
    const track={active:true,artist:'Fixture',playing:false,positionMs:3000,durationMs:120000};
    window.__emitNative('media',{...track,trackId:'B',title:'B'});
    window.__emitNative('media',{...track,trackId:'fixture-track',title:'A again'});
    window.__emitNative('lyrics',{trackId:'fixture-track',requestId:oldId,status:'synced',syncedLyrics:'[00:01]Stale lyrics must not appear'});
  });
  assert.equal(await native.locator('.lyric-line').count(),0,'Old events cannot restore stale lyrics after returning to the same track');
  await native.getByRole('tab',{name:'Usage',exact:true}).click();
  await native.locator('.page.stats').scrollIntoViewIfNeeded();
  await native.evaluate(()=>window.__emitNative('telemetry',{sampledAt:Date.now(),cpuPct:43,ramUsedBytes:8589934592,ramTotalBytes:17179869184,gpus:[{id:'gpu0',name:'Fixture GPU',usedPct:null}]}));
  assert.match(await native.locator('.page.stats').textContent(),/43%/);
  assert.equal(await native.locator('.page.stats .stat-face').count(),3,'One ring each for CPU, memory and the GPU');
  assert.equal(await native.locator('.page.stats .stat-face').nth(1).getAttribute('aria-label'),'Memory 50%, 8.0 of 16.0 GB');
  assert.equal(await native.locator('.page.stats .stat-face').nth(2).getAttribute('aria-label'),'Fixture GPU unavailable');
  await native.evaluate(()=>window.__emitNative('telemetry',{sampledAt:Date.now()-5000,cpuPct:99,ramUsedBytes:null,ramTotalBytes:null,gpus:[]}));
  assert.match(await native.locator('.page.stats').textContent(),/43%/,'An older snapshot cannot replace newer metrics');
  await native.waitForTimeout(1200);
  assert.match(await native.locator('.stat-face').first().getAttribute('aria-label'),/unavailable/,'Polling failures clear prior measurements');
  assert.equal(await native.locator('.stat-value').first().textContent(),'\u2013');
  await native.evaluate(()=>window.__emitNative('media',{active:true,trackId:'other',title:'Other fixture',artist:'Test',playing:true,positionMs:0,durationMs:10000}));
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'usage','Track changes must not steal a selected tool');
  // AI usage: one ring per meter, the provider mark inside, a threshold tone, details in the name
  await native.evaluate(()=>{
    localStorage.setItem('notch.chatgpt',JSON.stringify({label:'ChatGPT (estimate)',cap:10,hours:3}));
    const now=Math.round(Date.now()/1000);
    window.__emitNative('usage',[
      {id:'claude-5h',provider:'claude',label:'Claude session (5 h)',usedPct:93,spentUsd:null,resetsAt:now+8130,source:'official',stale:false},
      {id:'codex-installed-c-primary',provider:'codex',label:'Codex 5 h',usedPct:72,spentUsd:null,resetsAt:now+3630,source:'official',stale:false},
      {id:'openai-api',provider:'openai-api',label:'OpenAI API',usedPct:null,spentUsd:null,resetsAt:null,source:'api',stale:false,status:'unavailable',error:'No admin key'}]);
  });
  await native.locator('.page.usage').scrollIntoViewIfNeeded();
  assert.equal(await native.locator('#uList .u-cell').count(),4,'Three native meters and the ChatGPT estimate');
  assert.equal(await native.locator('#uList .u-cell svg.mark').count(),4,'Every meter shows its provider mark');
  await native.waitForTimeout(100);
  assert.deepEqual(await native.locator('#uList .ring').evaluateAll(n=>n.map(x=>x.dataset.tone)),['warn','caution','none','ok']);
  assert.equal(await native.locator('#uList .u-face').first().getAttribute('aria-label'),'Claude session (5 h), 93% used, resets in 2h 15m, official');
  assert.deepEqual(await native.locator('#uList .u-sub').evaluateAll(n=>n.map(x=>x.textContent)),['5h \u00b7 2h 15m','5h \u00b7 1h','API \u00b7 off','Est']);
  assert.equal(await native.locator('#uList .u-cell').nth(2).getAttribute('data-stale'),'true','An unavailable meter dims');
  const logButton = native.getByRole('button',{name:/^Log a ChatGPT message/});
  assert.equal(await logButton.count(),1);
  await logButton.click();
  assert.equal(await native.locator('#uList .u-cell').nth(3).locator('.u-val').textContent(),'1/10');
  assert.equal(await logButton.evaluate(n=>document.activeElement===n),true,'Logging keeps focus on the ring');
  const ringTargets = await native.locator('#uList .u-face').evaluateAll(nodes => nodes.map(n => n.getBoundingClientRect().width));
  assert.equal(ringTargets.every(size => size >= 36), true, 'Usage rings are at least 36px');
  await native.getByRole('tab',{name:'Tools',exact:true}).click();
  await native.locator('.page.dictation').scrollIntoViewIfNeeded();
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'review',transcript:'<b>Text, not markup</b>',modelReady:true}));
  assert.equal(await native.locator('.transcript').textContent(),'<b>Text, not markup</b>');
  assert.equal(await native.locator('.transcript b').count(),0);
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'review',transcript:'Partial insertion fixture',modelReady:true,canInsert:false,message:'Automatic retry disabled; check the target manually.'}));
  assert.equal(await native.getByRole('button',{name:'Insert',exact:true}).isDisabled(),true,'Partial insertions never offer a whole-transcript retry');
  await native.locator('.page.clipboard').scrollIntoViewIfNeeded();
  await native.evaluate(()=>window.__emitNative('clipboard_history_changed',{enabled:true,items:[{id:'clip',text:'<img src=x>clipboard fixture',pinned:false,createdAt:Date.now()}]}));
  assert.equal(await native.locator('.clipboard-text img').count(),0);
  assert.deepEqual(await native.locator('.clipboard-row .icon-btn').evaluateAll(n=>n.map(x=>x.getAttribute('aria-label'))),['Pin','Remove']);
  const toolTargets = await native.locator('.page.clipboard .icon-btn, .page.dictation .icon-btn').evaluateAll(nodes => nodes.map(n => { const r = n.getBoundingClientRect(); return Math.min(r.width, r.height); }));
  assert.equal(toolTargets.every(size => size >= 32), true, 'Icon buttons are at least 32px');
  await native.locator('.clipboard-text').click();
  assert.equal(await native.evaluate(()=>window.__calls.some(call=>call.command==='clipboard_copy'&&call.args.id==='clip')),true);
  await native.locator('.clipboard-text').focus();
  await native.keyboard.press('ArrowRight');
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'usage');
  assert.equal(await native.evaluate(()=>document.querySelector('.tab-panel.on').contains(document.activeElement)),true,'Keyboard navigation relocates focus into the visible tab');

  // Wheel: content scrolls first, then a fresh gesture at the edge switches tabs
  const panelTop = () => native.locator('.tab-panel.on').evaluate(el => el.scrollTop);
  const activeTab = () => native.locator('.tab-panel.on').getAttribute('data-tab');
  await native.getByRole('tab',{name:'Media',exact:true}).click();
  await native.locator('.page.media').hover({position:{x:200,y:20}});
  await native.mouse.wheel(0,-120); await native.waitForTimeout(400);
  assert.equal(await activeTab(),'media','Wheel up on the first tab stays put');
  await native.mouse.wheel(0,120); await native.waitForTimeout(400);
  assert.equal(await activeTab(),'media','Wheel scrolls to the next section before switching');
  assert.ok(await panelTop() > 0);
  await native.mouse.wheel(0,120); await native.waitForTimeout(400);
  assert.equal(await activeTab(),'today','Wheel past the last section switches to the next tab');
  assert.equal(await panelTop(),0,'Moving forward lands on the first section');
  await native.mouse.wheel(0,-120); await native.waitForTimeout(400);
  assert.equal(await activeTab(),'media');
  assert.ok(await panelTop() > 0,'Moving back lands on the last section');
  await native.mouse.wheel(120,0); await native.waitForTimeout(400);
  assert.equal(await activeTab(),'today','Horizontal wheel switches tabs');
  // Pointer drag swipe: dragging left moves to the next tab
  const area = await native.locator('.pages-container').boundingBox();
  await native.mouse.move(area.x + area.width - 40, area.y + 60);
  await native.mouse.down();
  await native.mouse.move(area.x + area.width - 120, area.y + 62, {steps:4});
  await native.mouse.move(area.x + area.width - 200, area.y + 64, {steps:4});
  await native.mouse.up();
  assert.equal(await activeTab(),'tools','A left drag swipes to the next tab');
  // The last tab is remembered across sessions
  await native.getByRole('tab',{name:'Usage',exact:true}).click();
  assert.equal(await native.evaluate(() => JSON.parse(localStorage.getItem('tuck.live.v2')).lastTab),'usage');
  await native.reload(); await native.waitForTimeout(150);
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'usage','The last tab is restored');
  await native.evaluate(()=>window.__emitNative('open-island',null));
  await native.locator('#railSettings').click();
  assert.equal(await native.evaluate(()=>window.__calls.some(call=>call.command==='open_settings_cmd')),true,'The rail settings button opens the settings window');
  await native.goto('http://tuck.test/settings.html');
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'review',modelInstalled:true,modelDownloadedBytes:0}));
  assert.equal(await native.locator('#removeModel').isDisabled(),true,'Review must be discarded before model removal');
  await native.locator('#clipboardEnabled').check();
  await native.locator('#clipboardPersist').check();
  await native.locator('#saveClipboard').click();
  await native.getByText('Could not save clipboard settings. Current preferences restored below.',{exact:true}).waitFor();
  assert.equal(await native.locator('#clipboardEnabled').isChecked(),false);
  assert.equal(await native.locator('#clipboardPersist').isChecked(),false);
  assert.deepEqual(await native.evaluate(()=>window.__calls.filter(call=>['clipboard_configure','clipboard_set_persistence','clipboard_set_enabled'].includes(call.command)).map(call=>call.command)),['clipboard_configure'],'Both clipboard flags use a single native mutation');
  assert.deepEqual(nativeErrors,[]);
  console.log('UI smoke passed: geometry, tab rail (keys, wheel edges, swipe, memory), accessibility, keyboard seeking, timers, usage rings, icon buttons, persistence errors, native-bridge call priority, shelf, focused radio updates, drag hold and previews.');
} finally { await browser.close(); }

