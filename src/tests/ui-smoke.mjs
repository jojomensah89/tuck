import { chromium } from '@playwright/test';
import { readFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
const root = process.cwd();
const browser = await chromium.launch({channel:'msedge',headless:true});
try {
  const context = await browser.newContext({viewport:{width:520,height:288}, reducedMotion:'reduce'});
  const serveDist = async route => {
    const pathname = new URL(route.request().url()).pathname;
    const file = path.join(root, process.env.SMOKE_DIST || 'dist', pathname === '/' ? 'index.html' : pathname);
    const types = {'.html':'text/html','.js':'application/javascript','.css':'text/css','.svg':'image/svg+xml'};
    try { await route.fulfill({body:await readFile(file),contentType:types[path.extname(file)] || 'application/octet-stream'}); }
    catch { await route.fulfill({status:404,body:'Not found'}); }
  };
  await context.route('http://tuck.test/**', serveDist);
  const page = await context.newPage();
  const instrumentTimers = () => {
    const active = new Set();
    const start = window.setInterval.bind(window), stop = window.clearInterval.bind(window);
    window.setInterval = (callback, delay, ...args) => { const id = start(callback, delay, ...args); active.add(id); return id; };
    window.clearInterval = id => { active.delete(id); stop(id); };
    window.__idleIntervals = active;
  };
  await context.addInitScript(instrumentTimers);

  // Opt-in, reproducible review captures; all state is restored before assertions resume.
  const capturePolish = async (page, name, settings = false) => {
    if (!process.env.POLISH_CAPTURE) return;
    const directory = path.join(root, 'docs/screens/polish', process.env.POLISH_CAPTURE);
    await mkdir(directory, {recursive:true});
    const viewport = page.viewportSize();
    for (const width of [440,500]) {
      if (settings) await page.setViewportSize({width,height:720});
      const sizing = settings ? null : await page.addStyleTag({content:`#isl { --open-w: ${width}px !important; } #isl[data-state="open"] { width: ${width}px !important; }`});
      await page.waitForTimeout(40);
      await (settings ? page : page.locator('#isl')).screenshot({path:path.join(directory,`${name}-${width}.png`),...(settings ? {fullPage:true} : {})});
      await sizing?.evaluate(node=>node.remove());
    }
    await page.setViewportSize(viewport);
  };

  const errors = [];
  page.on('pageerror',error => errors.push(error.message));
  await page.goto('http://tuck.test/');
  await page.waitForTimeout(200);
  assert.equal(await page.locator('.full').evaluate(el => el.inert), true);
  await capturePolish(page,'collapsed-media');
  await page.locator('#isl').focus();
  await page.keyboard.press('Enter');
  assert.equal(await page.locator('#isl').getAttribute('data-state'),'open');
  assert.equal(await page.locator('.full').getAttribute('aria-hidden'),'false');
  const box = await page.locator('#isl').boundingBox();
  assert.equal(Math.round(box.width),500); assert.equal(Math.round(box.height),264);
  await page.locator('#track').focus();
  await page.keyboard.press('ArrowRight');
  assert.equal(await page.locator('.tab-panel.on').getAttribute('data-tab'),'media');
  // Rail: five tabs with roving tabindex plus a settings button, no arrows or dots
  assert.equal(await page.locator('.rail-tabs [role=tab]').count(),5);
  assert.equal(await page.locator('#prevPage, #nextPage, #dots').count(),0);
  assert.deepEqual(await page.locator('.rail-tabs [role=tab]').evaluateAll(tabs => tabs.map(t => [t.getAttribute('aria-selected'), t.tabIndex])),[['true',0],['false',-1],['false',-1],['false',-1],['false',-1]]);
  const railGeometry = await page.locator('.rail-tab:visible').evaluateAll(nodes => nodes.map(n => { const r = n.getBoundingClientRect(); return [r.width, r.height]; }));
  assert.equal(railGeometry.every(([w,h]) => w === 32 && h === 32),true,'Rail hit targets are 32px');
  assert.deepEqual(await page.locator('.rail-tabs [role=tab]').evaluateAll(tabs => tabs.map(t => t.getAttribute('aria-label'))),['Media','Today','Dictation','Tools','Usage'],'Rail order');
  assert.equal(await page.locator('.section-index, .section-index-button').count(),0,'The old section marks are gone');
  // The rail holds seven items: tabs, the reserved update slot and the gear pinned at the bottom
  const railFit = () => page.evaluate(() => {
    const rect = el => el.getBoundingClientRect();
    const isl = rect(document.getElementById('isl')), tabs = rect(document.querySelector('.rail-tabs'));
    const gear = rect(document.getElementById('railSettings')), slot = rect(document.getElementById('railUpdate'));
    return { gearPinned: Math.round(gear.bottom - isl.top), tabsTop: Math.round(tabs.top - isl.top), tabsBottom: Math.round(tabs.bottom - isl.top), slotHeight: slot.height, slotTop: Math.round(slot.top - isl.top), gearTop: Math.round(gear.top - isl.top), clearOfContent: rect(document.querySelector('.rail')).right <= rect(document.querySelector('.pages-container')).left };
  });
  assert.deepEqual(await railFit(),{gearPinned:252,tabsTop:12,tabsBottom:172,slotHeight:0,slotTop:0,gearTop:220,clearOfContent:true},'An empty update slot leaves no gap');
  await page.evaluate(() => { document.getElementById('railUpdate').hidden = false; });
  const railWithUpdate = await railFit();
  assert.equal(railWithUpdate.slotHeight,32);
  assert.equal(railWithUpdate.slotTop,188,'A filled slot sits right above the gear');
  assert.equal(railWithUpdate.gearPinned,252);
  assert.ok(railWithUpdate.slotTop >= railWithUpdate.tabsBottom,'Seven items fit without overlapping');
  await page.evaluate(() => { document.getElementById('railUpdate').hidden = true; });
  // The real content box, with and without the row
  const contentBox = () => page.locator('.tab-panel.on').evaluate(el => { const r = el.getBoundingClientRect(); return [Math.round(r.width), Math.round(r.height)]; });
  assert.deepEqual(await contentBox(),[420,232],'A tab with one part has the whole content box');
  assert.equal(await page.locator('.section-row').isHidden(),true,'Media has no switch');
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
  assert.deepEqual(await contentBox(),[420,192],'The row takes 40px from a tab with two parts');
  await capturePolish(page,'today');
  // Today holds clock and battery, then the timer
  assert.deepEqual(await page.locator('#panel-today > .page').evaluateAll(n => n.map(x => x.dataset.page)),['today','activities']);
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
  assert.equal(await page.locator('#welcome').count(),0,'Welcome never appears in regular UI smoke');
  await page.goto('http://tuck.test/settings.html#welcome');
  await page.locator('#welcome').waitFor();
  await capturePolish(page,'welcome',true);
  assert.equal(await page.locator('#welcome #autostart').count(),1,'Welcome uses the real autostart control');
  assert.equal(await page.locator('#welcome #modelRows').count(),1,'Welcome uses the real model picker');
  await page.getByRole('button',{name:'Skip',exact:true}).click();
  assert.equal(await page.locator('#view-general #autostart').count(),1,'Closing restores the real controls');
  await page.goto('http://tuck.test/settings.html');
  // Sidebar: one view at a time, arrow keys move between sections.
  assert.equal(await page.locator('#view-general').isVisible(),true);
  assert.equal(await page.locator('#view-keys').count(),0);
  await page.locator('#nav a[data-view="general"]').focus();
  await page.keyboard.press('ArrowDown');
  await page.locator('#view-island').waitFor({state:'visible'});
  assert.equal(await page.locator('#nav a[data-view="island"]').getAttribute('aria-current'),'page');
  // Search filters rows across every view, and clears with Escape.
  await page.keyboard.press('Control+f');
  assert.equal(await page.evaluate(() => document.activeElement.id),'settingsSearch');
  await page.keyboard.type('expand delay');
  assert.equal(await page.locator('#row-openDelay').isVisible(),true);
  // The update row is not on screen without an update, so search must not find or count it
  const searchSettings = async query => { await page.locator('#settingsSearch').fill(query); return (await page.locator('#searchSummary').textContent()).trim(); };
  assert.equal(await searchSettings('update'),'1 setting matches "update".');
  assert.equal(await page.locator('.view .set-row:visible').count(),1,'The update count equals the rows on screen');
  assert.equal(await page.locator('#row-update-check').isVisible(),true);
  assert.equal(await page.locator('#row-update-available').isVisible(),false,'Update row must stay hidden when no update is available');
  assert.equal(await searchSettings('changelog'),'No settings match "changelog".');
  assert.equal(await page.locator('#searchEmpty').isVisible(),true,'A search for the hidden row shows the empty state, not a blank page');
  await page.keyboard.press('Escape');
  await page.locator('#view-island').waitFor({state:'visible'});
  // A default profile (180ms expand delay) must save a switch change straight away
  await page.evaluate(() => localStorage.removeItem('tuck.live.v2'));
  await page.reload(); await page.locator('#nav a[data-view="island"]').click();
  await page.locator('#view-island').waitFor({state:'visible'});
  assert.equal(await page.locator('#hoverExpand').count(),0);
  assert.equal(await page.locator('#row-openDelay').isVisible(),true);
  // Collapsed content offers all three modes and defaults to the lyric
  await page.evaluate(() => document.querySelector('#row-compactContent').closest('.view').dataset.view).then(view => page.locator(`#nav a[data-view="${view}"]`).click());
  assert.deepEqual(await page.locator('#row-compactContent .segmented label').allTextContents(),['Artwork','Title','Lyric']);
  assert.equal(await page.locator('input[name="compactContent"]:checked').getAttribute('value'),'lyrics');
  await page.locator('#row-compactContent label',{hasText:'Artwork'}).click();
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('tuck.live.v2')).compactContent),'artwork');
  await page.locator('#row-compactContent label',{hasText:'Lyric'}).click();
  await page.locator('#nav a[data-view="usage"]').click();
  await page.locator('#nav a[data-view="general"]').click();
  await page.locator('#pickFloat').click();
  assert.equal(await page.evaluate(() => localStorage.getItem('notch.shape')),'float');
  await page.locator('#nav a[data-view="island"]').click();
  await page.locator('#openDelay').fill('150');
  await page.locator('#openDelay').press('Enter');
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('tuck.live.v2')).openDelayMs),150,'Island settings save without a Save button');
  assert.equal(await page.locator('#row-openDelay').evaluate(el => el.classList.contains('saved')),true);
  await page.evaluate(() => { Storage.prototype.setItem = () => { throw new Error('Storage disabled'); }; });
  await page.locator('#nav a[data-view="usage"]').click();
  await page.setViewportSize({width:1000,height:850});
  await page.screenshot({path:path.join(root,'artifacts','settings-usage.png')});
  await page.setViewportSize({width:520,height:288});
  assert.deepEqual(errors,[]);

  // A mocked native bridge exercises the actual controllers; this is not a hardware smoke.
  const nativeBridge = () => {
    window.isTauri=true;
    let serial=0;const callbacks=new Map();const listeners=new Map();
    const state={settings:{notifications:false},shelf:[],notificationStatus:'Disabled',errors:[]};
    window.__nativeState=state;window.__calls=[];
    // Stand-ins for Explorer's thumbnails: a page, a photo, an app icon and a video frame
    window.__fixtureThumb=id=>{
      const kinds={pdf:'page',doc:'word',photo:'photo',exe:'hex',app:'app',video:'frame','video-fixture':'frame','doc-fixture':'page'};
      if(id==='gone')return {kind:'missing'};
      const kind=id.startsWith('added-')?'photo':kinds[id];if(!kind)return {kind:'none'};
      const c=document.createElement('canvas'),x=c.getContext('2d');
      const round=(l,t,w,h,r,fill)=>{x.fillStyle=fill;x.beginPath();x.roundRect(l,t,w,h,r);x.fill();};
      if(kind==='page'||kind==='word'){c.width=144;c.height=192;x.fillStyle='#fff';x.fillRect(0,0,144,192);x.fillStyle=kind==='word'?'#2b579a':'#444';x.fillRect(16,18,kind==='word'?112:70,kind==='word'?14:8);x.fillStyle='#b8b8bc';for(let y=44;y<176;y+=12)x.fillRect(16,y,y%36===8?80:112,4);}
      else if(kind==='photo'){c.width=192;c.height=128;x.fillStyle='#f2a65a';x.fillRect(0,0,192,70);x.fillStyle='#ffd9a0';x.beginPath();x.arc(132,62,18,0,Math.PI*2);x.fill();x.fillStyle='#1f4e6b';x.fillRect(0,70,192,58);x.fillStyle='#163a50';x.fillRect(20,84,60,6);x.fillRect(100,100,70,5);}
      else if(kind==='frame'){c.width=192;c.height=108;x.fillStyle='#18181b';x.fillRect(0,0,192,108);round(24,20,144,68,10,'#2c2c2e');x.fillStyle='#fff';x.beginPath();x.moveTo(88,40);x.lineTo(110,54);x.lineTo(88,68);x.fill();}
      else{c.width=192;c.height=192;round(24,24,144,144,32,kind==='hex'?'#202024':'#1e88e5');x.strokeStyle='#fff';x.lineWidth=10;x.beginPath();if(kind==='hex'){for(let i=0;i<6;i++){const a=Math.PI/3*i+Math.PI/6;x.lineTo(96+40*Math.cos(a),96+40*Math.sin(a));}x.closePath();}else{x.moveTo(96,60);x.lineTo(96,124);x.moveTo(72,102);x.lineTo(96,126);x.lineTo(120,102);}x.stroke();}
      return {kind:'image',url:c.toDataURL()};
    };
    window.__emitNative=(event,payload)=>{for(const id of listeners.get(event)||[])callbacks.get(id)?.({event,id,payload});};
    window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},transformCallback(callback){const id=++serial;callbacks.set(id,callback);return id;},async invoke(command,args={}){
      window.__calls.push({command,args});
      if(command==='plugin:event|listen'){const ids=listeners.get(args.event)||[];ids.push(args.handler);listeners.set(args.event,ids);return ++serial;}
      if(command==='extension_snapshot')return structuredClone(state);
      if(command==='shelf_pick'){state.shelf=[{id:'fixture-file',name:'sample.txt',path:'C:\\\\Fixture\\\\sample.txt',bytes:20}];return;}
      if(command==='shelf_remove'){state.shelf=args.id==null?[]:state.shelf.filter(item=>item.id!==args.id);return null;}
      if(command==='shelf_add'){const added=args.paths.map(p=>({id:`added-${++serial}`,name:p.split(/[\\/]/).pop(),path:p,bytes:48213}));state.shelf.push(...added);return added;}
      if(command==='shelf_thumbnail'||command==='clipboard_thumbnail')return window.__fixtureThumb(args.id);
      if(command==='extension_settings')throw 'Activity settings could not be saved';
      if(command==='media_now')return {active:false,trackId:'',title:'',artist:'',playing:false,positionMs:0,durationMs:0,artwork:null};
      if(command==='refresh_usage') return new Promise((resolve,reject) => { window.__refreshResolve = resolve; window.__refreshReject = reject; });
      if(command==='usage_now') {
        if(window.__holdUsage) return new Promise(resolve => { window.__resolveUsage = resolve; });
        return structuredClone(window.__usageSnapshot || []);
      }
      if(command==='clipboard_get_state')return {enabled:false,persistOnExit:false,items:[]};
      if(command==='clipboard_copy' && window.__missingClipboardFile)throw 'File no longer exists';
      if(command==='clipboard_configure')throw 'Fixture atomic save failure';
      if(command==='plugin:updater|check')return null;
      if(command==='installed_usage_providers')return window.__providers || [];
      if(command==='set_usage_provider')return null;
      if(command==='get_autostart'||command==='installed_usage_enabled'||command==='fullscreen_now')return false;
      if(command==='volume_step')return {kind:'volume',level:40+args.delta,muted:false};
      if(command==='volume_toggle_mute')return {kind:'volume',level:40,muted:true};
      if(command==='get_hide_system_flyout')return {enabled:true,state:'waiting',message:null};
      if(command==='set_hide_system_flyout'&&!args.enabled)return {enabled:false,state:'off',message:null};
      if(command==='set_hide_system_flyout')return {enabled:false,state:'unavailable',message:"Turned off: Windows showed its volume flyout several times, but Tuck couldn't find it on this version of Windows"};
      return null;
    }};
  };
  // Every installed-provider combination, using fixtures without invoking clients.
  for (const names of [[], ['claude'], ['codex'], ['claude','codex'], ['claude','codex','antigravity'], ['antigravity']]) {
    const settings = await context.newPage(); await settings.setViewportSize({width:1000,height:700}); await settings.addInitScript(nativeBridge);
    await settings.addInitScript(names => { window.__providers = ['claude','codex','antigravity'].map(provider => ({provider,installed:names.includes(provider),enabled:false})); }, names);
    await settings.goto('http://tuck.test/settings.html#usage');
    await settings.waitForTimeout(100);
    assert.equal(await settings.locator('#providerSwitches input').count(), names.length);
    assert.equal(await settings.locator('#providersEmpty').isVisible(), names.length === 0);
    for (const name of names) {
      const toggle = settings.locator(`#usage-${name}`); assert.equal(await toggle.isChecked(), false);
      if (name === 'antigravity') {
        await toggle.check();
        await settings.getByRole('dialog').waitFor();
        assert.match(await settings.getByRole('dialog').textContent(), /Tuck will read Antigravity's sign-in file/);
        await settings.screenshot({path:path.join(root,'docs/screens/antigravity-confirmation.png')});
        await settings.getByRole('button',{name:'Cancel',exact:true}).click();
        assert.equal(await toggle.isChecked(), false);
        assert.equal(await settings.evaluate(() => window.__calls.filter(c=>c.command==='set_usage_provider'&&c.args.provider==='antigravity').length),0);
        await toggle.check(); await settings.getByRole('button',{name:'Turn on',exact:true}).click();
        await settings.waitForTimeout(50);
        await settings.screenshot({path:path.join(root,'docs/screens/antigravity-settings.png')});
        await toggle.uncheck();
      } else { await toggle.check(); await toggle.uncheck(); }
    }
    const calls = await settings.evaluate(() => window.__calls.filter(c=>c.command==='set_usage_provider'));
    assert.equal(calls.length, names.length * 2);
    if (names.length === 2) { await mkdir(path.join(root,'docs/screens'),{recursive:true}); await capturePolish(settings,'settings-usage',true); await settings.screenshot({path:path.join(root,'docs/screens/usage-settings.png')}); }
    if (names.length === 3) {
      if(process.env.LOGO_CAPTURE !== 'before') assert.equal(await settings.locator('img[src="/providers/claude.svg"]').count(),1);
      for (const theme of ['dark','light']) {
        await settings.emulateMedia({colorScheme:theme});
        await settings.screenshot({path:path.join(root,`docs/screens/logos-${process.env.LOGO_CAPTURE || 'after'}-settings-${theme}.png`)});
      }
    }
    await settings.close();
  }

  for (const reduced of (process.env.LOGO_CAPTURE === 'before' ? [] : [false,true])) {
    const feedback = await context.newPage();
    await feedback.addInitScript(nativeBridge);
    await feedback.addInitScript(()=>{window.__providers=[{provider:'claude',installed:true,enabled:true}];});
    await feedback.emulateMedia({reducedMotion:reduced?'reduce':'no-preference'});
    for (const surface of ['settings','island']) {
      await feedback.goto(surface==='settings'?'http://tuck.test/settings.html#usage':'http://tuck.test/');
      if(surface==='island') { await feedback.locator('#isl').focus(); await feedback.keyboard.press('Enter'); await feedback.locator('#tab-usage').click(); await feedback.evaluate(()=>{window.__usageSnapshot=[{id:'claude',provider:'claude',label:'Claude 5h',usedPct:30,source:'official',status:'ok'}];window.__emitNative('usage',window.__usageSnapshot);}); await feedback.getByRole('tab',{name:'AI',exact:true}).click(); }
      const button = feedback.locator(surface==='settings'?'#refreshInstalledUsage':'#usageRefresh');
      for (const fail of [false,true]) {
        await button.click();
        assert.equal(await button.getAttribute('aria-busy'),'true');
        const look = await button.locator('svg').evaluate(n=>({animation:getComputedStyle(n).animationName,opacity:getComputedStyle(n).opacity}));
        assert.equal(look.animation,reduced?'none':'refresh-turn');
        if(reduced) { await feedback.waitForTimeout(150); assert.equal(await button.locator('svg').evaluate(n=>getComputedStyle(n).opacity),'0.45'); }
        const count = await feedback.evaluate(()=>window.__calls.filter(c=>c.command==='refresh_usage').length);
        await button.evaluate(n=>n.click());
        assert.equal(await feedback.evaluate(()=>window.__calls.filter(c=>c.command==='refresh_usage').length),count);
        await feedback.evaluate(fail=>fail?window.__refreshReject('Fixture failure'):window.__refreshResolve(window.__usageSnapshot || []),fail);
        assert.equal(await button.getAttribute('aria-busy'),'true','Fast replies retain minimum feedback');
        await feedback.waitForFunction(id=>document.getElementById(id).getAttribute('aria-busy')==='false',await button.getAttribute('id'));
        assert.equal(await button.isDisabled(),false);
        assert.equal(await button.locator('svg').evaluate(n=>getComputedStyle(n).animationName),'none');
      }
    }
    await feedback.close();
  }
  await context.addInitScript(nativeBridge);
  // Update screenshots and assertions use a mocked bridge, never a release installer.
  const updateFixtures = await context.newPage();
  await updateFixtures.goto('http://tuck.test/');
  await updateFixtures.waitForFunction(() => window.__calls?.some(c => c.args?.event === 'update-state-changed'));
  assert.equal(await updateFixtures.locator('#railUpdate').isHidden(), true);
  const fixture = {info:{version:'0.2.2',currentVersion:'0.2.1',body:'Fixture release notes'},phase:'available',progress:null,error:null,blocker:null};
  const emitUpdate = (page, state) => page.evaluate(state => window.__emitNative('update-state-changed', state), state);
  await emitUpdate(updateFixtures, fixture);
  await updateFixtures.waitForTimeout(100);
  await mkdir('docs/screens', {recursive:true});
  await capturePolish(updateFixtures,'update-peek');
  await updateFixtures.screenshot({path:'docs/screens/update-peek.png'});
  await updateFixtures.evaluate(() => window.__emitNative('open-island', null));
  const action = updateFixtures.locator('#railUpdate button');
  assert.equal(await action.getAttribute('aria-label'), 'Update to v0.2.2');
  await action.click();
  assert.ok(await updateFixtures.evaluate(() => window.__calls.some(c => c.command === 'update_action')));
  for (const [name, phase, progress, error, label] of [
    ['available','available',null,null,'Update to v0.2.2'],
    ['downloading','downloading',42,null,'Downloading update, 42%'],
    ['unknown','downloading',null,null,'Downloading update'],
    ['restart','restart',100,null,'Restart to update'],
    ['failed','available',null,'Fixture network failure','Update to v0.2.2'],
  ]) {
    await emitUpdate(updateFixtures, {...fixture,phase,progress,error});
    assert.equal(await action.getAttribute('aria-label'), label);
    assert.equal(await action.isDisabled(), phase === 'downloading');
    if (error) assert.equal(await action.getAttribute('title'), error);
    await updateFixtures.screenshot({path:`docs/screens/update-${name}.png`});
  }
  const updateSettings = await context.newPage();
  await updateSettings.setViewportSize({width:900,height:720});
  await updateSettings.goto('http://tuck.test/settings.html');
  await updateSettings.waitForFunction(() => window.__calls?.some(c => c.args?.event === 'update-state-changed'));
  await emitUpdate(updateSettings, fixture);
  assert.equal(await updateSettings.locator('.update-sidebar').getAttribute('aria-label'), 'Update to v0.2.2');
  await updateSettings.screenshot({path:'docs/screens/update-settings.png'});
  for (const phase of ['downloading', 'restart', 'available']) {
    const shared = {...fixture,phase,progress:42,error:phase === 'available' ? 'Fixture error' : null};
    await Promise.all([emitUpdate(updateFixtures, shared), emitUpdate(updateSettings, shared)]);
    assert.equal(await updateSettings.locator('.update-sidebar').getAttribute('aria-label'), await action.getAttribute('aria-label'));
    assert.equal(await updateSettings.locator('#btnInstallUpdate').isDisabled(), phase === 'downloading');
  }
  await Promise.all([emitUpdate(updateFixtures, {...fixture,info:null}), emitUpdate(updateSettings, {...fixture,info:null})]);
  assert.equal(await action.isHidden(), true);
  assert.equal(await updateSettings.locator('.update-sidebar').isHidden(), true);

  await updateFixtures.close(); await updateSettings.close();

  const native=await context.newPage();const nativeErrors=[];native.on('pageerror',error=>nativeErrors.push(error.message));
  await native.goto('http://tuck.test/');
  await native.waitForFunction(() => window.__calls?.some(call => call.command === 'plugin:event|listen' && call.args.event === 'open-island') && window.__calls.some(call => call.command === 'usage_now'));
  assert.equal(await native.evaluate(() => window.__idleIntervals.size), 0, 'No intervals while collapsed without running activities');
  await capturePolish(native,'idle');
  assert.equal(await native.locator('.page.todos').count(), 0, 'Todos feature must be completely removed');
  // Settings hears about an update from the background check, shows notes as text, and counts the row
  const updates=await context.newPage();
  await updates.goto('http://tuck.test/settings.html#about');
  assert.equal(await updates.locator('#row-update-available').isVisible(),false);
  await updates.evaluate(()=>window.__emitNative('pending-update-changed',{version:'9.9.9',currentVersion:'0.1.0',body:'### Added\n- <img src=x>One\n- Two\n\n### Fixed\n- Three'}));
  await updates.locator('#row-update-available').waitFor({state:'visible'});
  assert.equal(await updates.locator('#updateVersionBadge').textContent(),'v9.9.9');
  assert.match(await updates.locator('#updateStatusText').textContent(),/Update available: v9\.9\.9/);
  assert.deepEqual(await updates.locator('#updateNotes h3').allTextContents(),['Added','Fixed']);
  assert.deepEqual(await updates.locator('#updateNotes li').allTextContents(),['<img src=x>One','Two','Three']);
  assert.equal(await updates.locator('#updateNotes img').count(),0,'Release notes never become HTML');
  await updates.locator('#settingsSearch').fill('changelog');
  assert.equal((await updates.locator('#searchSummary').textContent()).trim(),'1 setting matches "changelog".');
  assert.equal(await updates.locator('#row-update-available').isVisible(),true);
  await updates.locator('#settingsSearch').fill('');
  // A failed install explains itself and does not stop services unless it got that far
  await updates.locator('#btnInstallUpdate').click();

  assert.equal(await updates.locator('#btnInstallUpdate').isEnabled(),true);
  assert.equal(await updates.evaluate(()=>window.__calls.some(call=>call.command==='update_action')),true,'About uses the shared action command');
  assert.equal(await updates.evaluate(()=>window.__calls.some(call=>call.command==='prepare_update_install')),false);
  await updates.evaluate(()=>window.__emitNative('pending-update-changed',null));
  await updates.locator('#row-update-available').waitFor({state:'hidden'});
  await updates.close();
  // A missed event is recovered by the pull on open; a racing newer event wins.
  const usageFixture = pct => [{id:'claude-5h',provider:'claude',label:'Claude session',usedPct:pct,spentUsd:null,resetsAt:null,source:'official',stale:false,status:'ok'}];
  await native.evaluate(meters => { window.__usageSnapshot = meters; window.__emitNative('open-island',null); }, usageFixture(31));
  try {
    await native.waitForFunction(() => document.querySelector('#uList .u-face')?.getAttribute('aria-label')?.includes('69% left'));
  } catch (error) {
    console.error('Usage open diagnostics', await native.evaluate(() => ({state:document.getElementById('isl')?.dataset.state,calls:window.__calls,usage:document.querySelector('#panel-usage')?.textContent})), nativeErrors);
    throw error;
  }
  await native.locator('#tab-media').click();
  await native.mouse.move(250,90);
  await native.locator('#track').focus();
  const beforeAnnouncement = await native.evaluate(() => ({focus:document.activeElement.id,hits:window.__calls.filter(c=>c.command==='set_hit').length}));
  await native.evaluate(() => {
    window.__openStates = [];
    new MutationObserver(records => records.forEach(r => { if(r.attributeName==='data-state') window.__openStates.push(document.getElementById('isl').dataset.state); })).observe(document.getElementById('isl'),{attributes:true});
    window.dispatchEvent(new CustomEvent('tuck-preview',{detail:{title:'Test',text:'Quiet notification'}}));
  });
  await native.waitForTimeout(3600);
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open');
  assert.equal(await native.evaluate(() => document.activeElement.id),beforeAnnouncement.focus);
  assert.deepEqual(await native.evaluate(() => window.__openStates),[],'Notification never transitions open geometry');
  assert.equal(await native.evaluate(() => window.__calls.filter(c=>c.command==='set_hit').length),beforeAnnouncement.hits);
  await native.locator('#tab-usage').click();
  await native.locator('#usageRefresh').click();
  assert.equal(await native.locator('#usageRefresh').isDisabled(),true);
  await native.evaluate(() => document.getElementById('usageRefresh').click());
  assert.equal(await native.evaluate(() => window.__calls.filter(c=>c.command==='refresh_usage').length),1);
  await native.evaluate(() => window.__refreshReject('Fixture failure'));
  await native.waitForFunction(() => !document.getElementById('usageRefresh').disabled);
  assert.match(await native.locator('#usageRefresh').getAttribute('title'),/failed/);
  assert.equal(await native.locator('#usageRefreshStatus').count(),0,'Refresh has no visible status text');
  assert.ok((await native.locator('#uList .u-face').getAttribute('aria-label')).includes('69% left'),'Failure retains readings');
  const refreshBox = await native.locator('#usageRefresh').boundingBox();
  assert.ok(refreshBox.width>=32 && refreshBox.height>=32);
  await native.locator('#usageRefresh').click();
  await native.evaluate(meters => window.__refreshResolve(meters),usageFixture(62));
  await native.waitForFunction(() => !document.getElementById('usageRefresh').disabled);
  assert.ok((await native.locator('#uList .u-face').getAttribute('aria-label')).includes('38% left'));
  await native.screenshot({path:path.join(root,'artifacts','island-usage-refresh.png')});
  await native.mouse.move(519,229);
  await native.keyboard.press('Escape');
  await native.evaluate(() => { window.__holdUsage = true; window.__emitNative('open-island',null); });
  await native.waitForFunction(() => typeof window.__resolveUsage === 'function');
  await native.evaluate(meters => window.__emitNative('usage',meters), usageFixture(72));
  await native.evaluate(meters => { window.__resolveUsage(meters); window.__holdUsage = false; }, usageFixture(12));
  await native.waitForTimeout(20);
  assert.ok((await native.locator('#uList .u-face').getAttribute('aria-label')).includes('28% left'),'A stale open response cannot overwrite a newer usage event');
  await native.keyboard.press('Escape');
  await native.evaluate(() => {
    window.__emitNative('open-island',null);
  });
  await native.keyboard.press('Escape');
  // Volume/brightness HUD from a mocked native level event
  const hudState = () => native.locator('#isl').evaluate(el => [el.dataset.state, el.dataset.hud ?? null]);
  await native.evaluate(()=>window.__emitNative('system-level',{kind:'volume',level:37,muted:false}));
  assert.deepEqual(await hudState(),['peek','volume'],'A volume change morphs the island into the HUD');
  assert.equal((await native.locator('#cr .compact-slot.current').textContent()).trim(),'37%');
  assert.equal(await native.locator('.compact').getAttribute('aria-label'),'Volume 37%');
  assert.equal(await native.locator('#cc .compact-slot.current .hud-bar').evaluate(el => el.style.getPropertyValue('--level')),'0.37');
  await native.locator('#statusMessage').filter({hasText:'Volume 37%'}).waitFor({timeout:2000});
  await native.waitForFunction(()=>document.getElementById('isl').dataset.state==='idle',null,{timeout:3000});
  assert.equal(await native.locator('#isl').getAttribute('data-hud'),null,'The HUD dismisses itself');
  await native.evaluate(()=>window.__emitNative('system-level',{kind:'brightness',level:70,muted:false}));
  assert.deepEqual(await hudState(),['peek','brightness']);
  assert.equal(await native.locator('.compact').getAttribute('aria-label'),'Brightness 70%');
  await native.waitForFunction(()=>document.getElementById('isl').dataset.state==='idle',null,{timeout:3000});
  // Wheel over the idle pill steps the system volume (Shift for 10) and shows the HUD
  const volumeCalls = () => native.evaluate(()=>window.__calls.filter(call=>call.command.startsWith('volume_')).map(call=>[call.command,call.args?.delta ?? null]));
  const pill = await native.locator('#isl').boundingBox();
  await native.mouse.move(pill.x + pill.width / 2, pill.y + pill.height / 2);
  await native.mouse.wheel(0,-100);
  await native.locator('#cr .compact-slot.current').filter({hasText:'42%'}).waitFor({timeout:2000});
  assert.deepEqual(await hudState(),['peek','volume']);
  await native.keyboard.down('Shift'); await native.mouse.wheel(0,100); await native.keyboard.up('Shift');
  await native.locator('#cr .compact-slot.current').filter({hasText:'30%'}).waitFor({timeout:2000});
  await native.mouse.click(pill.x + pill.width / 2, pill.y + pill.height / 2, {button:'middle'});
  await native.locator('.compact[aria-label="Volume muted, 40%"]').waitFor({timeout:2000});
  assert.deepEqual(await volumeCalls(),[['volume_step',2],['volume_step',-10],['volume_toggle_mute',null]]);
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'peek','Scrolling never opens the island');
  await native.waitForFunction(()=>document.getElementById('isl').dataset.state==='idle',null,{timeout:3000});
  await native.mouse.move(0, 220);
  // The flyout failure follows the HUD that triggered it (native emits the level first)
  const flyoutFailure = {enabled:false,state:'unavailable',message:'Turned off: flyout not found'};
  await native.evaluate(failure=>{window.__emitNative('system-level',{kind:'volume',level:50,muted:false});window.__emitNative('system-flyout',failure);},flyoutFailure);
  assert.deepEqual(await hudState(),['peek','volume'],'The HUD comes first');
  assert.equal(await native.locator('#statusMessage').textContent(),flyoutFailure.message,'Screen readers hear the failure immediately');
  await native.locator('#cc .compact-slot.current').filter({hasText:flyoutFailure.message}).waitFor({timeout:3000});
  assert.deepEqual(await hudState(),['peek',null],'The queued failure takes over once the HUD settles');
  await native.waitForFunction(()=>document.getElementById('isl').dataset.state==='idle',null,{timeout:5000});
  const setLive = update => native.evaluate(([key,update])=>{localStorage.setItem(key,JSON.stringify({...JSON.parse(localStorage.getItem(key)||'{}'),...update}));window.dispatchEvent(new StorageEvent('storage',{key}));},['tuck.live.v2',update]);
  const toIdle = () => native.waitForFunction(()=>document.getElementById('isl').dataset.state==='idle',null,{timeout:6000});
  // The HUD setting silences native level events; scrolling over the island still shows the level
  const flyoutPause = () => native.evaluate(()=>{const args=window.__calls.findLast(call=>call.command==='set_flyout_island_pause')?.args;return args?[args.hudOff,args.pinnedOpen]:null;});
  assert.deepEqual(await flyoutPause(),[false,false],'Native is told where the flyout stands at startup');
  await setLive({levelHud:false});
  assert.deepEqual(await flyoutPause(),[true,false],'The Windows flyout comes back while the HUD switch is off');
  await native.evaluate(()=>window.__emitNative('system-level',{kind:'volume',level:61,muted:false}));
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'idle','No HUD with the setting off');
  await native.mouse.move(pill.x + pill.width / 2, pill.y + pill.height / 2);
  await native.mouse.wheel(0,-100);
  await native.locator('.compact[aria-label="Volume 42%"]').waitFor({timeout:2000});
  await native.mouse.move(0, 220);
  await toIdle();
  await setLive({levelHud:true});
  assert.deepEqual(await flyoutPause(),[false,false]);
  // A brightness ramp right after a charger change stays quiet
  const power = {statusKnown:true,percentKnown:true,hasBattery:true,percent:80,minutesLeft:null};
  await native.evaluate(power=>{window.__emitNative('power',{...power,acConnected:false,charging:false});window.__emitNative('power',{...power,acConnected:true,charging:true});window.__emitNative('system-level',{kind:'brightness',level:33,muted:false});},power);
  assert.equal(await native.locator('#isl').getAttribute('data-hud'),null,'No brightness HUD during a charger ramp');
  await toIdle();
  // ...unless the Windows flyout is hidden, when the island is the only feedback there is
  await native.evaluate(power=>{window.__emitNative('system-flyout',{enabled:true,state:'hidden',message:null});window.__emitNative('power',{...power,acConnected:false,charging:false});window.__emitNative('power',{...power,acConnected:true,charging:true});window.__emitNative('system-level',{kind:'brightness',level:34,muted:false});},power);
  assert.deepEqual(await hudState(),['peek','brightness'],'A brightness change shows during the charger quiet period while the flyout is hidden');
  await native.evaluate(()=>window.__emitNative('system-flyout',{enabled:false,state:'off',message:null}));
  await toIdle();
  // Caps Lock then a volume key over a pinned open island: both peeks pass and it comes back open
  await setLive({pinned:true});
  await native.locator('#isl').click();
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open');
  assert.deepEqual(await flyoutPause(),[false,true],'The Windows flyout comes back while the island is pinned open');
  // Level changes over the open island: nothing collapses, and screen readers still hear them
  await native.evaluate(()=>window.__emitNative('system-level',{kind:'volume',level:66,muted:false}));
  await native.locator('#levelStrip[data-on="volume"] .hud-value').filter({hasText:'66%'}).waitFor({timeout:2000});
  await native.locator('#statusMessage').filter({hasText:'Volume 66%'}).waitFor({timeout:2000});
  assert.deepEqual(await hudState(),['open',null],'A level change does not collapse the open island');
  await native.evaluate(()=>window.__emitNative('desktop-peek',{title:'Caps Lock',text:'On'}));
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open');
  assert.match(await native.locator('#openAnnouncement').textContent(),/Caps Lock/);
  await native.evaluate(()=>window.__emitNative('system-level',{kind:'volume',level:55,muted:false}));
  assert.deepEqual(await hudState(),['open',null],'Notifications and HUD keep the pinned island open');
  // Losing focus does not close a pinned island.
  await native.evaluate(()=>window.dispatchEvent(new Event('blur')));
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open','Blur does not close a pinned island');
  await setLive({pinned:false});
  await native.evaluate(()=>window.dispatchEvent(new CustomEvent('tuck-drag-active',{detail:true})));
  await native.evaluate(()=>window.dispatchEvent(new Event('blur')));
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open','Blur does not close the island during a drag');
  await native.evaluate(()=>window.dispatchEvent(new CustomEvent('tuck-drag-active',{detail:false})));
  assert.deepEqual(await flyoutPause(),[false,false]);
  await native.evaluate(()=>window.__emitNative('hover',false));
  await toIdle();
  await native.mouse.move(0, 220);
  // The hover-opened island: the pill-sized strip stays volume, everywhere else is tabs
  const panelTab = () => native.locator('.tab-panel.on').getAttribute('data-tab');
  await native.evaluate(()=>window.__emitNative('hover',true));
  await native.waitForTimeout(250);
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open','Hover opens the island after its delay');
  const openBox = await native.locator('#isl').boundingBox();
  const stripX = openBox.x + openBox.width / 2, stripY = openBox.y + 16;
  const tabBeforeStrip = await panelTab();
  const stripCalls = (await volumeCalls()).length;
  await native.mouse.move(stripX, stripY);
  await native.mouse.wheel(0,-100);
  await native.locator('#levelStrip[data-on="volume"] .hud-value').filter({hasText:'42%'}).waitFor({timeout:2000});
  assert.deepEqual((await volumeCalls()).slice(stripCalls),[['volume_step',2]],'Wheel over the strip changes the volume');
  assert.equal(await panelTab(),tabBeforeStrip,'Wheel over the strip leaves the active tab alone');
  assert.deepEqual(await hudState(),['open',null],'The open island keeps its size and content');
  await native.locator('#statusMessage').filter({hasText:'Volume 42%'}).waitFor({timeout:2000});
  // The strip's corners count, and scrollers underneath do not take the wheel
  await native.mouse.move(stripX - 105, openBox.y + 31);
  await native.mouse.wheel(0,100);
  await native.locator('#levelStrip .hud-value').filter({hasText:'38%'}).waitFor({timeout:2000});
  await native.mouse.click(stripX, stripY, {button:'middle'});
  await native.locator('#levelStrip .hud-value.muted').filter({hasText:'40%'}).waitFor({timeout:2000});
  assert.deepEqual((await volumeCalls()).slice(stripCalls + 1),[['volume_step',-2],['volume_toggle_mute',null]],'Middle click over the strip toggles mute');
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open');
  // The indicator only paints: a control under the strip stays clickable while it is up
  assert.equal(await native.evaluate(([x,y])=>!document.elementFromPoint(x,y).closest('#levelStrip'),[stripX,stripY]),true,'The indicator takes no pointer events');
  await native.getByRole('tab',{name:'Tools',exact:true}).click();
  await native.locator('#extensionsPage').evaluate(el => { el.closest('.tab-panel').scrollTop = el.closest('.page').offsetTop; });
  await native.mouse.move(stripX, stripY);
  await native.mouse.wheel(0,-100);
  await native.locator('#levelStrip[data-on]').waitFor({timeout:2000});
  assert.equal(await native.getByRole('button',{name:'Downloads',exact:true}).count(),0);
  await native.getByRole('tab',{name:'Media',exact:true}).click();
  await native.locator('#levelStrip:not([data-on])').waitFor({state:'attached',timeout:3000});
  // Scrolling anywhere else switches tabs and leaves the volume alone
  const callsBeforeElsewhere = (await volumeCalls()).length;
  await native.waitForTimeout(400);
  await native.locator('#tab-usage').hover();
  await native.mouse.wheel(0,100);
  await native.waitForFunction(()=>document.querySelector('.tab-panel.on').dataset.tab==='today',null,{timeout:2000});
  await native.waitForTimeout(400);
  await native.mouse.move(stripX, openBox.y + 120);
  await native.mouse.wheel(100,0);
  await native.waitForFunction(()=>document.querySelector('.tab-panel.on').dataset.tab==='dictation',null,{timeout:2000});
  assert.equal((await volumeCalls()).length,callsBeforeElsewhere,'Scrolling outside the strip never changes the volume');
  await native.getByRole('tab',{name:'Media',exact:true}).click();
  await native.evaluate(()=>window.__emitNative('hover',false));
  await toIdle();
  await native.mouse.move(0, 220);
  const volumeCallsBeforeOpen = (await volumeCalls()).length;
  await native.evaluate(()=>window.__emitNative('open-island',null));
  await native.getByRole('tab',{name:'Tools',exact:true}).click();
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'tools');
  await native.keyboard.press('Escape');
  await native.evaluate(()=>window.__emitNative('hover',true));
  await native.waitForTimeout(250);
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open','Hover expands after the configured delay');
  assert.equal(await native.evaluate(()=>window.__calls.some(call=>call.command==='deactivate_island')),true);
  await native.evaluate(()=>window.__emitNative('open-island',null));
  await native.getByRole('tab',{name:'Files',exact:true}).click();
  // Files, with fixtures through the mocked bridge: Explorer's thumbnails and real drops need native runs
  const shelfDrop = native.getByRole('button',{name:/^Drop files here/});
  const dropBox = () => native.locator('.shelf-drop').evaluate(n=>{const r=n.getBoundingClientRect(),s=getComputedStyle(n);return {w:Math.round(r.width),h:Math.round(r.height),dashed:s.borderTopStyle,colour:s.borderTopColor,shown:s.display!=='none'};});
  const setShelf = names => native.evaluate(names=>{window.__nativeState.shelf=names.map(([id,name,bytes])=>({id,name,path:`C:\\Fixture\\${name}`,bytes}));window.__emitNative('extension-state',structuredClone(window.__nativeState));},names);
  const tileIds = () => native.locator('.shelf-tile:not(.leaving)').evaluateAll(n=>n.map(x=>x.dataset.id));
  const arrows = () => native.evaluate(()=>[...document.querySelectorAll('.shelf-scroll')].map(n=>!n.hidden));
  // Arrows follow the scroll a frame later
  const arrowsSettle = expected => native.waitForFunction(expected=>[...document.querySelectorAll('.shelf-scroll')].map(n=>!n.hidden).join()===expected.join(),expected,{timeout:2000}).catch(()=>{});
  const emptyDrop = await dropBox();
  assert.deepEqual([emptyDrop.w,emptyDrop.h,emptyDrop.dashed],[420,192,'dashed'],'An empty shelf fills the content box');
  assert.equal(await native.locator('.shelf-drop .drop-text').textContent(),'Drop files here');
  assert.equal(await native.locator('.shelf-drop .drop-hint').textContent(),'Drag out anytime · click to open');
  assert.equal(await native.locator('.shelf-drop .drop-text').evaluate(n=>getComputedStyle(n).fontSize),'14px','The title is at the primary size');
  assert.equal(await native.getByRole('button',{name:'Clear shelf',exact:true}).isVisible(),false,'An empty shelf has nothing to clear');
  await native.mouse.move(0,400);
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/shelf-empty-500.png')});
  await native.evaluate(()=>window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:true})));
  assert.notEqual((await dropBox()).colour,emptyDrop.colour,'A drag over the island highlights the box');
  assert.equal(await native.locator('.shelf-drop .drop-text').textContent(),'Release to add','A drag says what happens next');
  await native.evaluate(()=>window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:false})));
  await shelfDrop.click();
  await native.locator('.shelf-tile').waitFor();
  assert.equal(await native.locator('.shelf-tile .tile-name').textContent(),'sample.txt');
  assert.equal(await native.locator('.shelf-tile .tile-size').textContent(),'20 B');
  assert.equal((await dropBox()).shown,false,'With files the drop target steps aside for the tiles');
  assert.equal(await native.evaluate(()=>[...document.querySelectorAll('.full *')].filter(n=>getComputedStyle(n).borderTopStyle==='dashed').length),1,'The drop target is the one dashed element');
  const shelfFixtures = [
    ['pdf','Application form.pdf',158822],
    ['zip','Tuck-test-build-2026-11-08.zip',4.04*1024**2],
    ['exe','HexSetup.exe',17.4*1024**2],
    ['photo','Harbour at dusk.jpg',2.6*1024**2],
    ['doc','Quarterly report with a rather long file name that never ends.docx',16384],
    ['video','Island walkthrough.mp4',258.6*1024**2],
    ['gone','Old notes.txt',812],
    ['app','Installer.exe',794*1024],
  ];
  for (const width of [500,440]) {
    await setLive({openWidth:width});
    for (const count of [1,3,8]) {
      await setShelf(shelfFixtures.slice(0,count));
      await native.waitForFunction(count=>document.querySelectorAll('.shelf-tile').length===count,count);
      const painted = shelfFixtures.slice(0,count).filter(([id])=>id!=='zip').length;
      await native.waitForFunction(painted=>document.querySelectorAll('.shelf-tile img.loaded, .shelf-tile.missing').length===painted,painted,{timeout:3000});
      const geometry = await native.evaluate(()=>{
        const panel=document.querySelector('.tab-panel.on').getBoundingClientRect(),list=document.querySelector('.shelf-tiles').getBoundingClientRect();
        const tiles=[...document.querySelectorAll('.shelf-tile')].map(t=>{const p=t.querySelector('.tile-preview').getBoundingClientRect();return [Math.round(t.getBoundingClientRect().width),Math.round(p.width),Math.round(p.height)];});
        return {listWidth:Math.round(list.width),inside:list.left>=panel.left-0.5&&list.right<=panel.right+0.5&&list.top>=panel.top&&list.bottom<=panel.bottom,tiles,overflow:document.querySelector('.shelf-tiles').scrollWidth>document.querySelector('.shelf-tiles').clientWidth};
      });
      assert.equal(geometry.listWidth,width-80,`The row spans the content box at ${width}`);
      assert.equal(geometry.inside,true,'The row fits the content box');
      assert.equal(new Set(geometry.tiles.map(t=>t.join('x'))).size,1,'Tiles are equal');
      assert.deepEqual(geometry.tiles[0],[96,96,96],'Tiles are 96px squares');
      assert.equal(geometry.overflow,count===8,'Only eight files overflow');
      // New tiles scroll into view; start each check from the beginning of the row
      await native.locator('.shelf-tiles').evaluate(el=>{el.scrollTo({left:0,behavior:'instant'});});
      await arrowsSettle([false,count===8]);
      assert.deepEqual(await arrows(),[false,count===8],`Arrows show only where there is more (${count} at ${width})`);
      await native.mouse.move(0,400);
      await native.waitForFunction(()=>!document.querySelector('.tile-flash'),null,{timeout:3000});
      await native.locator('#isl').screenshot({path:path.join(root,`docs/screens/shelf-${count}-${width}.png`)});
    }
    // Long names end in an ellipsis; sizes are tabular figures
    const longName = native.locator('.shelf-tile[data-id="doc"] .tile-name');
    assert.deepEqual(await longName.evaluate(n=>[getComputedStyle(n).textOverflow,n.scrollWidth>n.clientWidth]),['ellipsis',true]);
    assert.match(await native.locator('.tile-size').first().evaluate(n=>getComputedStyle(n).fontVariantNumeric),/tabular-nums/);
    assert.deepEqual(await native.locator('.tile-size').allTextContents(),['155 KB','4.0 MB','17.4 MB','2.6 MB','16 KB','258.6 MB','File missing','794 KB']);
    assert.equal(await native.locator('.shelf-tile[data-id="gone"]').evaluate(n=>n.classList.contains('missing')),true,'A missing file shows a neutral missing state');
    assert.equal(await native.locator('.shelf-tile[data-id="zip"] .tile-preview img').count(),0,'No thumbnail falls back to the file glyph');
    // Arrows page through the row and disappear at the ends; the wheel scrolls it too
    const list = native.locator('.shelf-tiles');
    await native.getByRole('button',{name:'Scroll right',exact:true}).click();
    await native.waitForFunction(()=>document.querySelector('.shelf-tiles').scrollLeft>0);
    await arrowsSettle([true,true]);
    assert.deepEqual(await arrows(),[true,true],'In the middle both arrows show');
    await list.evaluate(el=>{el.scrollLeft=el.scrollWidth;});
    await arrowsSettle([true,false]);
    assert.deepEqual(await arrows(),[true,false],'At the end only the left arrow shows');
    await native.mouse.move(0,400);
    await native.locator('#isl').screenshot({path:path.join(root,`docs/screens/shelf-8-end-${width}.png`)});
    await native.getByRole('button',{name:'Scroll left',exact:true}).click();
    await list.evaluate(el=>{el.scrollLeft=0;});
    await native.waitForFunction(()=>document.querySelector('.shelf-scroll.prev').hidden);
    const tabBefore = await native.locator('.tab-panel.on').getAttribute('data-tab');
    await native.waitForTimeout(400);
    await list.hover({position:{x:150,y:40}}); await native.mouse.wheel(0,120); await native.waitForTimeout(100);
    assert.ok(await list.evaluate(el=>el.scrollLeft)>0,'The wheel scrolls the row sideways');
    await native.waitForTimeout(400);
    const sideways = await list.evaluate(el=>el.scrollLeft);
    await native.mouse.wheel(80,0); await native.waitForTimeout(100);
    assert.ok(await list.evaluate(el=>el.scrollLeft)>sideways,'A sideways trackpad swipe scrolls the row');
    assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),tabBefore,'Scrolling the row leaves the tab alone');
    await list.evaluate(el=>{el.scrollLeft=0;});
  }
  // Hover shows the remove X (a 32px target with a name); it removes one file
  await setLive({openWidth:500});
  await native.mouse.move(0,400);
  await setShelf(shelfFixtures.slice(0,3));
  await native.waitForFunction(()=>document.querySelectorAll('.shelf-tile').length===3);
  const firstTile = native.locator('.shelf-tile[data-id="pdf"]');
  const removePdf = native.getByRole('button',{name:'Remove Application form.pdf',exact:true});
  assert.equal(await removePdf.evaluate(n=>getComputedStyle(n).opacity),'0','The X waits for hover or focus');
  await firstTile.hover();
  await native.waitForFunction(()=>getComputedStyle(document.querySelector('.shelf-tile[data-id="pdf"] .tile-remove')).opacity==='1');
  const removeBox = await removePdf.boundingBox();
  assert.deepEqual([removeBox.width,removeBox.height],[32,32],'The X has a 32px target');
  assert.equal(await native.getByRole('button',{name:'Show Application form.pdf in folder',exact:true}).isVisible(),true,'Show in folder stays reachable');
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/shelf-hover-500.png')});
  // Clicking a tile opens the file by its shelf id, never by a path from the page
  await firstTile.locator('.tile-open').click();
  assert.deepEqual(await native.evaluate(()=>window.__calls.findLast(c=>c.command==='shelf_open').args),{id:'pdf'});
  await native.getByRole('button',{name:'Show Application form.pdf in folder',exact:true}).click();
  assert.deepEqual(await native.evaluate(()=>window.__calls.findLast(c=>c.command==='shelf_reveal').args),{id:'pdf'});
  await firstTile.hover();
  await removePdf.click();
  assert.deepEqual(await native.evaluate(()=>window.__calls.findLast(c=>c.command==='shelf_remove').args),{id:'pdf'});
  await native.waitForFunction(()=>document.querySelectorAll('.shelf-tile').length===2);
  assert.deepEqual(await tileIds(),['zip','exe'],'The other files stay in order');
  // Dragging a file over the island covers the tiles with the drop target for the whole drag
  const fileDrag = (event, paths) => native.evaluate(([event,paths])=>window.__emitNative(event,{paths,position:{x:200,y:120}}),[event,paths]);
  await fileDrag('tauri://drag-enter',['C:\\Fixture\\Dropped sketch.png']);
  await native.waitForFunction(()=>document.getElementById('extensionsPage').classList.contains('drag-over'));
  const covering = await dropBox();
  assert.deepEqual([covering.shown,covering.w,covering.h],[true,420,192],'The full drop target covers the tiles');
  assert.equal(await native.locator('.shelf').isVisible(),false,'Tiles are hidden while a file is over the island');
  assert.equal(await native.locator('.shelf-drop .drop-text').textContent(),'Release to add');
  assert.equal(covering.dashed,'none','Dragging removes the edge entirely');
  assert.equal(await native.locator('.shelf-drop svg.ico').evaluate(n=>getComputedStyle(n).color),'rgb(255, 255, 255)','The drag icon stays neutral');
  await fileDrag('tauri://drag-over',[]);
  assert.equal(await native.locator('.shelf').isVisible(),false,'The tiles stay covered for the whole drag');
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/shelf-drag-over-500.png')});
  // Leaving without a drop brings the same tiles back
  await native.evaluate(()=>window.__emitNative('tauri://drag-leave',null));
  await native.waitForFunction(()=>!document.getElementById('extensionsPage').classList.contains('drag-over'));
  assert.equal(await native.locator('.shelf').isVisible(),true);
  assert.deepEqual(await tileIds(),['zip','exe'],'Leaving without a drop changes nothing');
  assert.equal(await native.evaluate(()=>window.__calls.some(c=>c.command==='shelf_add')),false);
  // A drop adds a tile, which arrives highlighted
  await fileDrag('tauri://drag-enter',['C:\\Fixture\\Dropped sketch.png']);
  await fileDrag('tauri://drag-drop',['C:\\Fixture\\Dropped sketch.png']);
  await native.waitForFunction(()=>document.querySelectorAll('.shelf-tile').length===3&&!document.getElementById('extensionsPage').classList.contains('drag-over'));
  assert.deepEqual(await native.evaluate(()=>window.__calls.findLast(c=>c.command==='shelf_add').args),{paths:['C:\\Fixture\\Dropped sketch.png']});
  assert.equal(await native.locator('.shelf-tile').last().locator('.tile-name').textContent(),'Dropped sketch.png');
  assert.equal(await native.locator('.shelf-tile').last().locator('.tile-flash').count(),1,'The new tile is highlighted');
  await native.mouse.move(0,400);
  await native.locator('.shelf-tile').last().locator('img.loaded').waitFor();
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/shelf-dropped-500.png')});
  // Clear in the row removes everything and brings back the empty drop target
  const clearShelf = native.getByRole('button',{name:'Clear shelf',exact:true});
  assert.equal(await clearShelf.textContent(),'Clear');
  const clearBox = await clearShelf.boundingBox(), snipNow = await native.getByRole('button',{name:'Screen snip',exact:true}).boundingBox();
  assert.ok(clearBox.height===32 && clearBox.x+clearBox.width<=snipNow.x && snipNow.x-(clearBox.x+clearBox.width)<=8,'Clear sits next to Screen snip');
  await clearShelf.click();
  assert.equal(await native.evaluate(()=>window.__calls.findLast(c=>c.command==='shelf_remove').args.id),null);
  await native.waitForFunction(()=>document.querySelectorAll('.shelf-tile').length===0);
  assert.equal((await dropBox()).shown,true,'An empty shelf shows the drop target again');
  await shelfDrop.click();
  await native.locator('.shelf-tile').waitFor();
  await native.locator('.page.clipboard').evaluate(el=>{el.closest('.tab-panel').scrollTop=el.offsetTop;});
  await native.getByRole('button',{name:'Clear unpinned history',exact:true}).waitFor({timeout:3000});
  const snipBox = await native.getByRole('button',{name:'Screen snip',exact:true}).boundingBox();
  assert.equal(snipBox.width,32);
  assert.equal(snipBox.height,32);
  assert.equal(await native.getByRole('button',{name:'Screen snip',exact:true}).count(),1,'One Screen snip');
  assert.equal(await native.evaluate(()=>document.querySelector('.section-actions').lastElementChild.getAttribute('aria-label')),'Screen snip','Screen snip is at the right end of the row');
  assert.equal(Math.round(snipBox.x+snipBox.width),Math.round((await native.locator('.section-row').boundingBox()).x+(await native.locator('.section-row').boundingBox()).width));
  assert.equal(await native.getByRole('button',{name:'Clear unpinned history',exact:true}).isVisible(),true,'Clipboard actions sit beside it');
  assert.equal(await native.getByRole('button',{name:'Clear shelf',exact:true}).isVisible(),false,'Only the part on screen shows its actions');
  assert.equal(await native.locator('#extensionsPage').evaluate(n=>n.inert),true,'The part off screen cannot take focus');
  await native.getByRole('button',{name:'Screen snip',exact:true}).click();
  assert.equal(await native.evaluate(()=>window.__calls.some(c=>c.command==='quick_action'&&c.args.name==='capture')),true);
  assert.equal(await native.locator('.page.quick').count(),0);
  assert.equal(await native.getByRole('button',{name:'Calls',exact:true}).count(),0);
  assert.equal(await native.getByRole('button',{name:'Radios',exact:true}).count(),0);
  await native.evaluate(()=>window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:true})));
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open');
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'tools','A file drag reveals the file shelf');
  assert.equal(await native.locator('#extensionsPage').evaluate(el => { const p = el.closest('.tab-panel').getBoundingClientRect(), r = el.getBoundingClientRect(); return Math.abs(r.top - p.top) < 2; }),true,'The shelf section is scrolled into view');
  await native.evaluate(()=>window.__emitNative('hover',false));await native.waitForTimeout(1750);
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open','Native drag must hold the island open');
  await native.evaluate(()=>window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:false})));
  await native.mouse.move(0,400);
  await native.keyboard.press('Escape');
  await native.evaluate(()=>window.__emitNative('notification-preview',{title:'Fixture notification',body:'Preview text'}));
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'peek');
  assert.match(await native.locator('#cc').textContent(),/Fixture notification/);
  await native.screenshot({path:path.join(root,'artifacts','island-v1.png')});
  // File-drag navigation uses fixtures; no Explorer/OLE claim.
  const dragEvent = active => native.evaluate(active => window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:active})), active);
  for (const origin of ['clipboard','today','collapsed']) {
    await native.evaluate(()=>window.__emitNative('open-island',null));
    if (origin === 'clipboard') { await native.locator('#tab-tools').click(); await native.getByRole('tab',{name:'Clipboard',exact:true}).click(); }
    else await native.locator('#tab-today').click();
    if (origin === 'collapsed') await native.keyboard.press('Escape');
    if(origin==='clipboard') await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/motion-switch-clipboard-fixture.png')});
    await dragEvent(true);
    if(origin==='clipboard') await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/motion-switch-files-fixture.png')});
    assert.equal(await native.locator('#tab-tools').getAttribute('aria-selected'),'true',origin+' drag selects Tools');
    assert.equal(await native.getByRole('tab',{name:'Files',exact:true}).getAttribute('aria-selected'),'true',origin+' drag selects Files');
    assert.equal(await native.locator('#extensionsPage').evaluate(el=>!el.inert),true);
    await dragEvent(false);
    assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),origin==='clipboard'?'tools':'today','Leave restores tab');
    if(origin==='clipboard') assert.equal(await native.getByRole('tab',{name:'Clipboard',exact:true}).getAttribute('aria-selected'),'true','Leave restores part');
    if(origin==='collapsed') assert.equal(await native.locator('#isl').getAttribute('data-state'),'idle','Leave restores collapsed state');
  }
  await dragEvent(true);
  await native.evaluate(()=>window.dispatchEvent(new Event('tuck-drop-complete')));
  await dragEvent(false);
  assert.equal(await native.locator('#tab-tools').getAttribute('aria-selected'),'true','Drop stays on Tools');
  assert.equal(await native.getByRole('tab',{name:'Files',exact:true}).getAttribute('aria-selected'),'true','Drop stays on Files');
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/motion-drop-fixture.png')});
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
  await native.getByRole('tab',{name:'Today',exact:true}).click();
  await native.getByRole('tab',{name:'Media',exact:true}).click();
  await native.screenshot({path:path.join(root,'artifacts','island-phase-two-media.png')});

  // Closed media content stays present while playing and uses one stable width per track.
  await native.evaluate(()=>document.getElementById('actReset').click());
  await native.evaluate(()=>window.__emitNative('media',{active:true,trackId:'fixture-track',title:'Artwork color fixture',artist:'Synthetic test data',playing:true,positionMs:3000,durationMs:120000,artwork:null}));
  await setLive({compactContent:'lyrics'});
  await native.keyboard.press('Escape');
  await native.waitForTimeout(450);
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'idle');
  assert.equal((await native.locator('#isl').boundingBox()).width,300,'Lyric mode reserves a stable wider closed pill');
  assert.equal((await native.locator('#cc .compact-title-track').textContent()).trim(),'Current fixture line');
  assert.ok(await native.evaluate(()=>{const m=r=>r.left+r.width/2;return Math.abs(m(document.querySelector('#cc .compact-slot.current .compact-title-track').getBoundingClientRect())-m(document.querySelector('.island').getBoundingClientRect()))<=2;}),'Collapsed text is centred on the pill');
  await setLive({compactContent:'title'}); await native.waitForTimeout(50);
  assert.equal((await native.locator('#cc .compact-title-track').textContent()).trim(),'Artwork color fixture');
  assert.equal((await native.locator('#isl').boundingBox()).width,300,'Title mode keeps the same stable width');
  // The native hit area follows the wider pill, so all of it clicks and takes the volume wheel
  const lastHit = () => native.evaluate(()=>window.__calls.findLast(call=>call.command==='set_hit').args.w);
  assert.equal(await lastHit(),300,'Native hit bounds cover the wider pill');
  const textPill = await native.locator('#isl').boundingBox();
  await native.mouse.move(textPill.x + textPill.width - 4, textPill.y + 16);
  await native.mouse.wheel(0,-100);
  await native.locator('.compact[aria-label^="Volume"]').waitFor({timeout:2000});
  await native.mouse.move(0, 220);
  await toIdle();
  assert.equal((await native.locator('#isl').boundingBox()).width,300,'The pill comes back wide after the HUD');
  await native.mouse.click(textPill.x + 4, textPill.y + 16);
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open','The far edge of the wider pill opens the island');
  // A cursor resting there keeps the volume wheel once the pill has opened under it
  const volumeBeforeEdge = (await volumeCalls()).length;
  await native.mouse.move(textPill.x + 4, textPill.y + 16);
  await native.mouse.wheel(0,-100);
  await native.locator('#levelStrip[data-on="volume"]').waitFor({timeout:2000});
  assert.equal((await volumeCalls()).length,volumeBeforeEdge + 1,'The open strip uses the width the pill would close to');
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open');
  await native.keyboard.press('Escape');
  await native.mouse.move(0, 220);
  // Lyric lines change the text only: same width, no new hit bounds
  await setLive({compactContent:'lyrics'}); await native.waitForTimeout(50);
  const hitCalls = () => native.evaluate(()=>window.__calls.filter(call=>call.command==='set_hit').length);
  const hitsBefore = await hitCalls();
  await native.evaluate(()=>window.__emitNative('media',{active:true,trackId:'fixture-track',title:'Artwork color fixture',artist:'Synthetic test data',playing:true,positionMs:5000,durationMs:120000,artwork:null}));
  await native.locator('#cc .compact-title-track',{hasText:'Next fixture line'}).waitFor({timeout:2000});
  assert.equal((await native.locator('#isl').boundingBox()).width,300,'A new lyric line never resizes the pill');
  assert.equal(await hitCalls(),hitsBefore,'A new lyric line never touches the native bounds');
  // A line too long for the pill marquees instead of widening it
  await setLive({compactContent:'title'}); await native.waitForTimeout(50);
  await native.evaluate(()=>window.__emitNative('media',{active:true,trackId:'long-track',title:'A title far too long to fit in the collapsed pill however wide it gets',artist:'Synthetic test data',playing:true,positionMs:0,durationMs:120000,artwork:null}));
  await native.waitForTimeout(100);
  assert.equal((await native.locator('#isl').boundingBox()).width,300,'Long text keeps the stable width');
  assert.equal(await native.locator('#cc .compact-slot.current .compact-title').evaluate(n=>getComputedStyle(n).textOverflow==='ellipsis'&&n.scrollWidth>n.clientWidth&&!n.classList.contains('scroll')&&!n.classList.contains('scroll-once')),true,'Reduced motion ends a long line in an ellipsis without scrolling');
  // A pause between two tracks holds the pill: no resize and no new hit bounds
  const hitsBeforePause = await hitCalls();
  await native.evaluate(()=>window.__emitNative('media',{active:true,trackId:'long-track',title:'A title far too long to fit in the collapsed pill however wide it gets',artist:'Synthetic test data',playing:false,positionMs:0,durationMs:120000,artwork:null}));
  await native.waitForTimeout(200);
  assert.equal((await native.locator('#isl').boundingBox()).width,300,'A pause holds the pill for a moment');
  await native.evaluate(()=>window.__emitNative('media',{active:true,trackId:'fixture-track',title:'Artwork color fixture',artist:'Synthetic test data',playing:true,positionMs:5000,durationMs:120000,artwork:null}));
  assert.equal((await native.locator('#isl').boundingBox()).width,300,'The next track finds the pill unchanged');
  assert.equal(await hitCalls(),hitsBeforePause,'A pause between tracks never touches the native bounds');
  // Dictation outranks media and reserves readable space for its live preview
  await native.evaluate(()=>window.dispatchEvent(new CustomEvent('tuck-activity',{detail:{id:'dictation',type:'dictation',phase:'recording',title:'Dictation',text:'',priority:100,active:true}})));
  assert.equal(await native.locator('.compact').getAttribute('aria-label'),'Dictation recording');
  await native.waitForTimeout(50);
  assert.equal((await native.locator('#isl').boundingBox()).width,300,'Dictation reserves a stable preview width');
  assert.equal(await native.locator('#cc .compact-slot.current .dictation-waveform').count(),1);
  await native.evaluate(()=>window.dispatchEvent(new CustomEvent('tuck-activity',{detail:{id:'dictation',type:'dictation',title:'Dictation',text:'',priority:100,active:false}})));
  await setLive({compactContent:'title'}); await native.waitForTimeout(50);
  await setLive({compactContent:'artwork'}); await native.waitForTimeout(450);
  assert.equal((await native.locator('#isl').boundingBox()).width,212,'Artwork mode returns to the base pill width');
  assert.equal((await native.locator('#cc').textContent()).trim(),'');
  await setLive({compactContent:'title'});
  await native.evaluate(()=>window.__emitNative('media',{active:true,trackId:'fixture-track',title:'Artwork color fixture',artist:'Synthetic test data',playing:false,positionMs:3000,durationMs:120000,artwork:null}));
  await native.waitForTimeout(300);
  assert.equal((await native.locator('#isl').boundingBox()).width,300,'The pill holds just after a pause');
  await native.waitForTimeout(1000);
  assert.equal((await native.locator('#isl').boundingBox()).width,212,'Paused media returns to the idle glyph layout');
  assert.match(await native.locator('.compact').getAttribute('aria-label'),/^Time /);
  assert.equal(await native.locator('#cr .battery-glyph').count(),1);
  await native.evaluate(()=>window.__emitNative('open-island',null));
  // An exact match stays quiet; only an approximate match offers timing, and the correction sticks to the track.
  const lyricEvent=extra=>native.evaluate(extra=>window.__emitNative('lyrics',{trackId:'fixture-track',requestId:window.__calls.findLast(call=>call.command==='lyrics_get').args.requestId,syncedLyrics:'[00:01]Previous fixture line\n[00:03]Current fixture line\n[00:05]Next fixture line',...extra}),extra);
  const lyricTrackEvent=trackId=>native.evaluate(trackId=>window.__emitNative('lyrics',{trackId,requestId:window.__calls.findLast(call=>call.command==='lyrics_get').args.requestId,status:'synced',approximate:true,syncedLyrics:'[00:01]Fixture line'}),trackId);
  for (const width of [440, 500]) {
    await native.evaluate(width => {
      const key = 'tuck.live.v2';
      localStorage.setItem(key, JSON.stringify({...JSON.parse(localStorage.getItem(key) || '{}'), openWidth: width}));
      window.dispatchEvent(new StorageEvent('storage', {key}));
    }, width);
    // No lyrics: the player takes the full width, the toggle is quiet and says why
    await lyricEvent({status:'unavailable',syncedLyrics:''});
    const bare = await native.evaluate(() => {
      const rect = selector => document.querySelector(selector).getBoundingClientRect();
      const toggle = document.querySelector('.lyrics-toggle-btn');
      return {
        paneHidden: getComputedStyle(document.querySelector('.lyrics-pane')).display === 'none',
        fullWidth: Math.abs(rect('.page.media .bar').right - rect('.page.media').right) <= 1,
        disabled: toggle.getAttribute('aria-disabled'), why: toggle.title, quiet: getComputedStyle(toggle).color === getComputedStyle(document.querySelector('.page.media .art')).color,
        titleRowClear: rect('#mediaBattBadge').top >= rect('#title').bottom - 1,
      };
    });
    assert.deepEqual(bare,{paneHidden:true,fullWidth:true,disabled:'true',why:'No lyrics for this track',quiet:true,titleRowClear:true},`Without lyrics the player fills ${width}px`);
    await native.locator('.lyrics-toggle-btn').click({force:true});
    assert.equal(await native.locator('.lyrics-pane').isVisible(),false,'The quiet toggle does not open an empty pane');
    await capturePolish(native,'media');
    await lyricEvent({status:'synced'});
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
    await capturePolish(native,'lyrics');
    await native.screenshot({path:path.join(root,'artifacts',`island-unslop-${width}.png`)});
  }
  await lyricEvent({status:'synced'});
  assert.equal(await native.locator('.lyrics-offset-controls').isVisible(),false,'An exact match shows no timing control');
  assert.equal(await native.locator('.lyrics-header').isVisible(),false,'An exact match leaves no empty header above the lyrics');
  await lyricEvent({status:'synced',approximate:true});
  assert.equal(await native.locator('.lyrics-status').textContent(),'Approximate');
  assert.equal(await native.locator('.lyrics-offset-value').textContent(),'0.0 s');
  assert.match(await native.locator('.lyrics-note').textContent(),/another edit/i,'An approximate match says why');
  assert.equal(await native.locator('.lyrics-offset-value').getAttribute('aria-live'),'polite');
  for(let step=0;step<3;step++) await native.getByRole('button',{name:'Show lyrics 0.1 s later, hold to repeat'}).click();
  assert.equal(await native.locator('.lyrics-offset-value').textContent(),'+0.3 s');
  await native.locator('.lyric-line').last().click();
  assert.equal(await native.evaluate(()=>window.__calls.findLast(call=>call.command==='media_seek').args.ms),5300,'Seeking follows the corrected timing');
  assert.deepEqual(await native.evaluate(()=>JSON.parse(localStorage.getItem('tuck.live.v2')).lyricsOffsets),[{key:'fixture-track',offsetMs:300}]);
  await lyricEvent({status:'synced'});
  assert.equal(await native.locator('.lyrics-offset-value').textContent(),'+0.3 s','A saved correction stays reachable on an exact match');
  assert.equal(await native.locator('.lyrics-note').isVisible(),false,'A saved correction alone is no timing warning');
  await lyricEvent({status:'plain',approximate:true,syncedLyrics:'',plainLyrics:'Fixture words'});
  assert.equal(await native.locator('.lyrics-offset-controls').isVisible(),false,'Plain lyrics have no timing to correct');
  assert.equal(await native.locator('.lyrics-status').textContent(),'Approximate · not synchronized');
  await native.evaluate(()=>{const key='tuck.live.v2';localStorage.setItem(key,JSON.stringify({...JSON.parse(localStorage.getItem(key)),lyricsOffsets:[]}));});
  // A different credit is no reason to doubt the timing.
  await lyricEvent({status:'synced',creditDiffers:true});
  assert.equal(await native.locator('.lyrics-header').isVisible(),false,'A credit difference shows no label or stepper');
  assert.equal(await native.locator('.lyrics-note').isVisible(),false,'A credit difference shows no timing warning');
  assert.match(await native.locator('.lyrics-pane').getAttribute('title'),/credit/);
  // Auto-scroll centres the current line, whatever sits above the list.
  await lyricEvent({status:'synced',approximate:true,syncedLyrics:Array.from({length:40},(_,i)=>`[${String(Math.floor(i*5/60)).padStart(2,'0')}:${String(i*5%60).padStart(2,'0')}]Line ${i}`).join('\n')});
  await native.evaluate(()=>window.__emitNative('media',{active:true,trackId:'fixture-track',title:'Artwork color fixture',artist:'Synthetic test data',playing:false,positionMs:62000,durationMs:300000}));
  const centerError=await native.evaluate(()=>{const list=document.querySelector('.lyric-lines').getBoundingClientRect(),line=document.querySelector('.lyric-line.current').getBoundingClientRect();return Math.abs(line.top+line.height/2-(list.top+list.height/2));});
  assert.ok(centerError<=2,`Current lyric sits ${centerError}px off centre`);
  await native.evaluate(()=>window.__emitNative('media',{active:true,trackId:'fixture-track',title:'Artwork color fixture',artist:'Synthetic test data',playing:false,positionMs:3000,durationMs:300000}));
  // Holding repeats; other buttons and a track change during the hold must not move anything.
  const fixtureMedia=trackId=>native.evaluate(trackId=>window.__emitNative('media',{active:true,trackId,title:'Artwork color fixture',artist:'Synthetic test data',playing:false,positionMs:3000,durationMs:120000}),trackId);
  const later=native.getByRole('button',{name:'Show lyrics 0.1 s later, hold to repeat'});
  const offsetValue=()=>native.locator('.lyrics-offset-value').textContent();
  await lyricEvent({status:'synced',approximate:true});
  await later.hover();
  await native.mouse.down(); await native.waitForTimeout(600); await native.mouse.up();
  const held=await offsetValue();
  assert.notEqual(held,'0.0 s','Holding a stepper repeats');
  await native.waitForTimeout(200);
  assert.equal(await offsetValue(),held,'Releasing stops the repeat, and its click adds nothing');
  await native.mouse.down({button:'right'}); await native.waitForTimeout(600); await native.mouse.up({button:'right'});
  assert.equal(await offsetValue(),held,'Only the main button repeats');
  await later.focus(); await native.keyboard.press('Enter');
  assert.notEqual(await offsetValue(),held,'A right-button hold does not swallow the next key press');
  await native.locator('.lyrics-toggle-btn').hover();
  await fixtureMedia('hold-track'); await lyricTrackEvent('hold-track');
  await later.hover(); await native.mouse.down();
  await fixtureMedia('hold-track-2'); await lyricTrackEvent('hold-track-2');
  await native.waitForTimeout(600); await native.mouse.up();
  assert.equal(await offsetValue(),'0.0 s','A hold that spans a track change saves nothing on the new track');
  await later.focus();
  await fixtureMedia('fixture-track');
  assert.equal(await native.evaluate(()=>document.activeElement?.className),'lyrics-toggle-btn','Focus stays near when the timing controls hide');
  await native.evaluate(()=>{const key='tuck.live.v2';localStorage.setItem(key,JSON.stringify({...JSON.parse(localStorage.getItem(key)),lyricsOffsets:[]}));});
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
  // Emit and read in one evaluate, so the page's one-second poll cannot land in between.
  const statsRead=()=>({text:document.querySelector('.page.stats').textContent,faces:[...document.querySelectorAll('.page.stats .stat-face')].map(face=>face.getAttribute('aria-label'))});
  const measured=await native.evaluate(read=>{window.__emitNative('telemetry',{sampledAt:Date.now(),cpuPct:43,networkDownBps:12400000,networkUpBps:1200000,ramUsedBytes:8589934592,ramTotalBytes:17179869184,gpus:[{id:'gpu0',name:'Fixture GPU',usedPct:null}]});return new Function(`return (${read})()`)();},statsRead.toString());
  assert.match(measured.text,/43%/);
  assert.match(measured.text,/12.4 MB\/s/); assert.match(measured.text,/1.2 MB\/s/);

  assert.equal(measured.faces.length,3,'One ring each for CPU, memory and the GPU');
  assert.equal(measured.faces[1],'Memory 50%, 8.0 of 16.0 GB');
  assert.equal(measured.faces[2],'Fixture GPU unavailable');
  const afterOlder=await native.evaluate(read=>{window.__emitNative('telemetry',{sampledAt:Date.now()-5000,cpuPct:99,ramUsedBytes:null,ramTotalBytes:null,gpus:[]});return new Function(`return (${read})()`)();},statsRead.toString());
  assert.match(afterOlder.text,/43%/,'An older snapshot cannot replace newer metrics');
  // The mock's poll keeps failing: one failure keeps the value, a run of them clears it.
  await native.waitForTimeout(1200);
  assert.match(await native.locator('.page.stats').textContent(),/43%/,'One failed poll does not blank a valid metric');
  await native.waitForFunction(()=>/unavailable/.test(document.querySelector('.stat-face').getAttribute('aria-label')),null,{timeout:6000});
  assert.equal(await native.locator('.stat-value').first().textContent(),'\u2013','Repeated polling failures clear prior measurements');
  for (const width of [440,500]) {
    await native.evaluate(()=>window.__emitNative('telemetry',{sampledAt:Date.now(),cpuPct:43,networkDownBps:12400000,networkUpBps:1200000,ramUsedBytes:8589934592,ramTotalBytes:17179869184,gpus:[{id:'gpu0',name:'Fixture GPU',usedPct:null}]}));
    await setLive({openWidth:width}); await native.waitForTimeout(60);
    await native.locator('#isl').screenshot({path:path.join(root,`docs/screens/weekly-${process.env.LOGO_CAPTURE || 'after'}-system-${width}.png`)});
    if (process.env.LOGO_CAPTURE !== 'before') {
      assert.equal(await native.locator('.network-caption').textContent(),'Network');
      assert.match(await native.locator('.network-rate').first().getAttribute('aria-label'),/^Download speed /);
      assert.equal(await native.locator('.network-rate').last().getAttribute('title'),'Upload speed');
    }
    assert.equal(await native.locator('.network-rates').evaluate(n=>{ const r=n.getBoundingClientRect(),p=n.closest('.page').getBoundingClientRect(); return r.left>=p.left && r.right<=p.right && r.bottom<=p.bottom; }),true,'Network rates fit the System part');
    await mkdir(path.join(root,'docs/screens'),{recursive:true});
    await native.locator('#isl').screenshot({path:path.join(root,`docs/screens/usage-system-${width}.png`)});
  }

  await native.evaluate(()=>window.__emitNative('media',{active:true,trackId:'other',title:'Other fixture',artist:'Test',playing:true,positionMs:0,durationMs:10000}));
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'usage','Track changes must not steal a selected tool');
  // AI usage: one provider card, double rings for 5h + 7d, official marks, no unconnected cards
  await native.evaluate(()=>{
    const now=Math.round(Date.now()/1000);
    window.__emitNative('usage',[
      {id:'claude-5h',provider:'claude',label:'Claude session (5 h)',usedPct:42,spentUsd:null,resetsAt:now+8130,source:'official',stale:false,status:'ok'},
      {id:'claude-week',provider:'claude',label:'Claude weekly',usedPct:18,spentUsd:null,resetsAt:now+86400*2,source:'official',stale:false,status:'ok'},
      {id:'codex-installed-c-primary',provider:'codex',label:'Codex 5 h',usedPct:72,spentUsd:null,resetsAt:now+3630,source:'official',stale:false,status:'ok'},
      {id:'codex-installed-c-secondary',provider:'codex',label:'Codex weekly',usedPct:9,spentUsd:null,resetsAt:now+86400*5,source:'official',stale:true,status:'stale'},
    ]);
  });
  await native.locator('.page.usage').scrollIntoViewIfNeeded();
  assert.equal(await native.locator('#uList .u-provider').count(),2,'Two connected providers become two cards');
  assert.equal(await native.locator('#uList .u-provider .brand-mark, #uList .u-provider .provider-artwork').count(),2,'Every provider shows its real mark');
  assert.equal(await native.locator('#uList .u-ring-outer').count(),2);
  assert.equal(await native.locator('#uList .u-ring-inner').count(),2);
  await native.waitForTimeout(100);
  assert.deepEqual(await native.locator('#uList .ring').evaluateAll(n=>n.map(x=>x.dataset.tone)),['ok','ok','caution','ok']);
  assert.deepEqual(await native.locator('#uList .u-provider').first().locator('.u-value-line').evaluateAll(n=>n.map(x=>x.textContent)),['5h58% left','7d82% left']);
  assert.match(await native.locator('#uList .u-face').first().getAttribute('aria-label'),/Claude\. 5h, 58% left.*7d, 82% left/);
  assert.equal(await native.locator('#uList .u-provider').nth(1).getAttribute('data-stale'),'true','A stale configured provider remains and dims');
  assert.equal(await native.locator('#uList .u-provider').nth(1).locator('.u-state').textContent(),'Stale','A stale provider says so');
  assert.equal(await native.locator('#uList .u-provider').first().locator('.u-state').isHidden(),true,'A fresh provider has no state word');
  assert.match(await native.locator('#uList .u-provider').first().getAttribute('title'),/resets in 2h 15m/,'Reset times live in the tooltip');
  const ringTargets = await native.locator('#uList .u-face').evaluateAll(nodes => nodes.map(n => n.getBoundingClientRect().width));
  assert.equal(ringTargets.every(size => size >= 72), true, 'Provider rings are larger than the old rings');

  // Every card fits, whatever the island width and however many providers are connected
  const quotaProviders = now => [
    {id:'claude-5h',provider:'claude',label:'Claude session (5 h)',usedPct:100,spentUsd:null,resetsAt:now+8130,source:'official',stale:false,status:'ok'},
    {id:'claude-week',provider:'claude',label:'Claude weekly',usedPct:93,spentUsd:null,resetsAt:now+86400*2,source:'official',stale:false,status:'ok'},
    {id:'codex-primary',provider:'codex',label:'Codex 5 h',usedPct:72,spentUsd:null,resetsAt:now+3630,source:'official',stale:false,status:'ok'},
    {id:'codex-secondary',provider:'codex',label:'Codex weekly',usedPct:9,spentUsd:null,resetsAt:now+86400*5,source:'official',stale:true,status:'stale'},
  ];
  const usageFit = () => native.evaluate(()=>{
    const list=document.getElementById('uList'),page=document.querySelector('.page.usage').getBoundingClientRect();
    const container=document.querySelector('.pages-container').getBoundingClientRect();
    const inside=r=>r.left>=container.left-1&&r.right<=container.right+1&&r.top>=page.top-1&&r.bottom<=page.bottom+1;
    const parts=[...list.querySelectorAll('.u-face,.u-values,.u-value-line,.u-state')].filter(n=>n.getBoundingClientRect().height>0);
    const clear=[...list.querySelectorAll('.u-provider')].every(card=>card.querySelector('.u-face').getBoundingClientRect().right<=card.querySelector('.u-values').getBoundingClientRect().left&&card.querySelector('.u-values').scrollWidth<=card.querySelector('.u-values').clientWidth+1);
    const segments=[...document.querySelectorAll('.section-switch-button')].map(n=>n.getBoundingClientRect());
    return {inside:parts.every(n=>inside(n.getBoundingClientRect())),clear,switchClear:segments.every(r=>r.bottom<=page.top+0.5),fits:list.scrollHeight<=page.height+1};
  });
  for (const width of [440,500]) {
    await setLive({openWidth:width});
    await native.waitForTimeout(50);
    assert.deepEqual(await usageFit(),{inside:true,clear:true,switchClear:true,fits:true},`Two provider cards fit at ${width}px`);
    await native.evaluate(meters=>window.__emitNative('usage',meters),quotaProviders(Math.round(Date.now()/1000)));
    await native.waitForTimeout(50);
    assert.equal(await native.locator('#uList .u-provider').count(),2,'Each connected source is one card');
    assert.deepEqual(await usageFit(),{inside:true,clear:true,switchClear:true,fits:true},`Two provider cards fit at ${width}px`);
    // A used-up quota has an empty ring, so its number warns; anything still left, does not
    const valueColours = await native.locator('#uList .u-val').evaluateAll(nodes=>nodes.map(n=>[n.textContent,n.dataset.out==='true',getComputedStyle(n).color]));
    const warnColour = await native.locator('#uList').evaluate(n=>{const probe=document.createElement('span');probe.style.color='var(--warn-ink)';n.append(probe);const colour=getComputedStyle(probe).color;probe.remove();return colour;});
    assert.deepEqual(valueColours.filter(([,out])=>out).map(([text,,colour])=>[text,colour]),[['0% left',warnColour]],'Only the used-up quota reads as out, in the warning colour');
    assert.equal(valueColours.filter(([,out])=>!out).some(([,,colour])=>colour===warnColour),false,'Other values keep their normal colour');
    const bannerFit = await native.locator('#openAnnouncement').evaluate(n=>{const b=n.getBoundingClientRect(),i=document.getElementById('isl').getBoundingClientRect();return !n.textContent || (b.left>=i.left+56 && b.right<=i.right-24 && b.bottom<=i.top+16);});
    assert.equal(bannerFit,true,'Open announcements stay inside the island above the content row');
    await capturePolish(native,'usage');
    await native.locator('#isl').screenshot({path:path.join(root,`docs/screens/usage-providers-${width}.png`)});
    await native.evaluate(meters=>window.__emitNative('usage',meters),quotaProviders(Math.round(Date.now()/1000)).slice(0,4));
  }
  assert.equal(await native.locator('#uList .u-provider').count(),2);

  // Review evidence uses only the mocked native bridge and synthetic quota readings.
  for (const theme of ['dark','light']) {
    await native.emulateMedia({colorScheme:theme});
    for (const width of [440,500]) {
      await setLive({openWidth:width});
      const now = Math.round(Date.now()/1000);
      const extra = ['antigravity','fixture-b','fixture-c'].map(provider=>({id:provider,provider,label:provider==='antigravity'?'Antigravity quota':provider,usedPct:35,resetsAt:now+3600,source:'official',stale:false,status:'ok'}));
      for (const state of ['ai','five','stale']) {
        const meters = [...quotaProviders(now), ...extra.slice(0,state==='five'?3:1), {id:'antigravity-weekly',provider:'antigravity',label:'Antigravity weekly',usedPct:61,resetsAt:now+86400*6,source:'official',stale:false,status:'ok'}];
        meters.find(m=>m.provider==='antigravity').label='Antigravity 5 h';
        if(state==='stale') meters.forEach(m=>{m.stale=true;m.status='stale';});
        await native.evaluate(m=>window.__emitNative('usage',m),meters);
        await native.waitForTimeout(100);
        assert.deepEqual(await usageFit(),{inside:true,clear:true,switchClear:true,fits:true});
        if (process.env.LOGO_CAPTURE !== 'before') {
          if(state==='ai') {
            const cards=await native.locator('#uList .u-provider').evaluateAll(nodes=>nodes.map(n=>{const r=n.getBoundingClientRect();return {left:r.left,top:r.top};}));
            assert.equal(cards[2].left,cards[0].left,'Third provider is under the first');
            assert.ok(cards[2].top>cards[0].top);
            assert.deepEqual(await native.locator('#uList .u-provider').nth(2).locator('.u-window').allTextContents(),['5h','7d']);
            assert.equal(await native.locator('#uList .u-provider').nth(2).locator('.ring').count(),2);
          }
          const session = await context.newCDPSession(native);
          for (const scale of [1,1.5,2]) {
            await session.send('Emulation.setDeviceMetricsOverride',{width:520,height:288,deviceScaleFactor:scale,mobile:false});
            assert.deepEqual(await usageFit(),{inside:true,clear:true,switchClear:true,fits:true},`Fits at ${scale*100}%`);
            assert.equal(await native.locator('.provider-artwork').evaluateAll(nodes=>nodes.every(n=>n.complete&&n.naturalWidth>0&&(n.naturalWidth>=96||n.currentSrc.endsWith('.svg')))),true,'Official artwork loads sharply');
          }
          await session.send('Emulation.clearDeviceMetricsOverride');
          await session.detach();
          await native.setViewportSize({width:520,height:288});
          await native.evaluate(()=>window.scrollTo(0,0));
        }
        await native.evaluate(()=>document.activeElement?.blur());
        await native.mouse.move(519,287);
        await native.waitForTimeout(150);
        await native.locator('#isl').screenshot({path:path.join(root,`docs/screens/logos-${process.env.LOGO_CAPTURE || 'after'}-${state}-${width}-${theme}.png`)});
      }
    }
  }
  // Antigravity fixtures: five cards at both supported widths, then error/off.
  for (const width of [440,500]) {
    await setLive({openWidth:width});
    const now = Math.round(Date.now()/1000);
    const extra = ['antigravity','fixture-b','fixture-c'].map(provider => ({id:provider,provider,label:provider==='antigravity'?'Antigravity model quota':provider,usedPct:35,resetsAt:now+3600,source:'official',stale:false,status:'ok'}));
    await native.evaluate(m=>window.__emitNative('usage',m),[...quotaProviders(now),...extra]);
    await native.waitForTimeout(100);
    assert.equal(await native.locator('#uList .u-provider').count(),5);
    assert.deepEqual(await usageFit(),{inside:true,clear:true,switchClear:true,fits:true});
    assert.equal(await native.locator('.u-list.dense .u-val').evaluateAll(nodes=>nodes.every(n=>parseFloat(getComputedStyle(n).fontSize)>=11)),true,'Dense quota values stay readable');
    await capturePolish(native,'usage-dense');
    await native.locator('#isl').screenshot({path:path.join(root,`docs/screens/antigravity-five-${width}.png`)});
    extra[0]={...extra[0],usedPct:null,stale:true,status:'error',error:'Antigravity quota unavailable'};
    await native.evaluate(m=>window.__emitNative('usage',m),[...quotaProviders(now),extra[0]]);
    assert.equal(await native.locator('#uList .u-state').last().textContent(),'Error');
    if(width===440) await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/antigravity-error.png')});
    await native.evaluate(m=>window.__emitNative('usage',m),quotaProviders(now));
    assert.equal(await native.locator('#uList .u-provider').count(),2);
  }

  // The segmented switch is a real tablist and follows the same snap scroller.
  const sectionButtons = native.locator('.section-switch-button');
  assert.equal(await sectionButtons.count(),2);
  assert.deepEqual(await sectionButtons.allTextContents(),['AI','System']);
  assert.equal(await sectionButtons.first().getAttribute('aria-selected'),'true');
  assert.equal(await native.locator('.section-switch').getAttribute('role'),'tablist');
  assert.equal(await sectionButtons.first().getAttribute('role'),'tab');
  assert.equal(await native.locator(`#${await sectionButtons.first().getAttribute('aria-controls')}`).getAttribute('role'),'tabpanel');
  assert.deepEqual(await sectionButtons.evaluateAll(nodes=>nodes.map(n=>n.tabIndex)),[0,-1],'Roving tabindex');
  const sectionTargets = await sectionButtons.evaluateAll(nodes=>nodes.map(node=>{const r=node.getBoundingClientRect();return [r.width,r.height];}));
  assert.equal(sectionTargets.every(([w,h])=>w>=32&&h===32),true,'Segments are 32px tall');
  assert.equal(sectionTargets[0][0],sectionTargets[1][0],'Segments are equal widths');
  // Refresh belongs to AI and stays quiet
  const refreshLook = await native.locator('#usageRefresh').evaluate(n=>{const s=getComputedStyle(n);const probe=document.createElement('i');probe.style.color='var(--ink-2)';document.body.append(probe);const ink=getComputedStyle(probe).color;probe.remove();return {background:s.backgroundColor,ink:s.color===ink};});
  assert.deepEqual(refreshLook,{background:'rgba(0, 0, 0, 0)',ink:true},'Refresh is an ink-2 icon without a filled tile');
  await sectionButtons.nth(1).focus();
  assert.equal(await sectionButtons.nth(1).evaluate(node=>document.activeElement===node),true,'Section controls are keyboard focusable');
  await sectionButtons.nth(1).click();
  await native.waitForTimeout(400);
  assert.equal(await sectionButtons.nth(1).getAttribute('aria-selected'),'true');
  assert.equal(await native.locator('#usageRefresh').isVisible(),false,'Refresh leaves with AI');
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open','Section clicks do not collapse the island');
  const usageTop = () => native.locator('#panel-usage').evaluate(el => el.scrollTop);
  assert.ok(await usageTop() > 0,'A click snaps the panel to its section');
  assert.equal(await native.evaluate(()=>JSON.parse(localStorage.getItem('tuck.live.v2')).lastTab),'usage','A section click keeps the tab remembered');
  await sectionButtons.first().focus();
  await native.keyboard.press('Enter');
  await native.waitForTimeout(400);
  assert.equal(await usageTop(),0,'The keyboard reaches the same navigation');
  assert.equal(await sectionButtons.first().evaluate(n=>getComputedStyle(n).outlineStyle),'solid','A focused mark shows its focus ring');
  // The wheel over the switch moves between parts, like over the page, and only then between tabs
  await native.waitForTimeout(400);
  await sectionButtons.first().hover();
  await native.mouse.wheel(0,120); await native.waitForTimeout(400);
  assert.equal(await sectionButtons.nth(1).getAttribute('aria-selected'),'true','Wheel over the switch moves to the next part');
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'usage');
  // The section count can change while the tab is open
  await native.evaluate(()=>window.__emitNative('usage',[]));
  assert.equal(await native.locator('.section-switch').isHidden(),true);
  assert.equal(await native.evaluate(()=>document.querySelector('.tab-panel.on').contains(document.activeElement)||document.activeElement===document.querySelector('.tab-panel.on')),true,'Focus does not fall out of the island when the switch leaves');
  await native.evaluate(()=>{const now=Math.round(Date.now()/1000);window.__emitNative('usage',[
    {id:'claude-5h',provider:'claude',label:'Claude session (5 h)',usedPct:42,spentUsd:null,resetsAt:now+8130,source:'official',stale:false,status:'ok'}]);});
  assert.equal(await sectionButtons.nth(1).getAttribute('aria-selected'),'true','A section the reader moved to stays in view when another returns above it');
  assert.ok(await usageTop() > 0);
  await sectionButtons.first().click(); await native.waitForTimeout(400);
  assert.equal(await sectionButtons.first().getAttribute('aria-selected'),'true');

  // Arrows, Home and End rove inside the switch and do not change the tab
  const selectedSegment = name => native.locator('.section-switch-button[aria-selected="true"]',{hasText:name}).waitFor({timeout:3000});
  await sectionButtons.nth(1).focus();
  await native.keyboard.press('ArrowLeft');
  await selectedSegment('AI');
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'usage');
  assert.equal(await sectionButtons.first().evaluate(n=>document.activeElement===n),true,'Focus follows the selected segment');
  await native.keyboard.press('End');
  await selectedSegment('System');
  await native.keyboard.press('ArrowRight');
  await selectedSegment('AI'); // Switch arrows wrap
  // The switch belongs to the tab on screen, and tabs without parts have none
  await native.getByRole('tab',{name:'Media',exact:true}).click();
  assert.equal(await sectionButtons.count(),0);
  await native.getByRole('tab',{name:'Dictation',exact:true}).click();
  assert.equal(await sectionButtons.count(),0);
  await native.getByRole('tab',{name:'Today',exact:true}).click();
  assert.deepEqual(await sectionButtons.allTextContents(),['Clock','Timer']);
  await native.getByRole('tab',{name:'Tools',exact:true}).click();
  assert.deepEqual(await sectionButtons.allTextContents(),['Files','Clipboard']);
  await native.getByRole('tab',{name:'Usage',exact:true}).click();
  assert.deepEqual(await sectionButtons.allTextContents(),['AI','System']);

  // One provider keeps one card; no connected providers remove AI usage and its index mark.
  await native.evaluate(()=>{const now=Math.round(Date.now()/1000);window.__emitNative('usage',[
    {id:'claude-5h',provider:'claude',label:'Claude session (5 h)',usedPct:42,spentUsd:null,resetsAt:now+8130,source:'official',stale:false,status:'ok'},
    {id:'claude-week',provider:'claude',label:'Claude weekly',usedPct:18,spentUsd:null,resetsAt:now+86400*2,source:'official',stale:false,status:'ok'}]);});
  assert.equal(await native.locator('#uList .u-provider').count(),1);
  assert.equal(await native.locator('#uList .u-ring-inner').count(),1,'Both windows draw for the one provider');
  await native.evaluate(()=>{const now=Math.round(Date.now()/1000);window.__emitNative('usage',[
    {id:'claude-week',provider:'claude',label:'Claude weekly',usedPct:18,spentUsd:null,resetsAt:now+86400*2,source:'official',stale:false,status:'ok'}]);});
  assert.equal(await native.locator('#uList .u-ring-outer').count(),1);
  assert.equal(await native.locator('#uList .u-ring-inner').count(),0,'A single-window provider draws one ring');
  assert.equal(await native.locator('#uList .u-value-line').count(),1);
  // A client that has failed before its first reading: no source word in the window slot
  const errorCard = {id:'codex-installed',provider:'codex',label:'ChatGPT / Codex',usedPct:null,spentUsd:null,resetsAt:null,source:'official',stale:true,status:'error',error:'Installed client usage request timed out'};
  const week = now => ({id:'claude-week',provider:'claude',label:'Claude weekly',usedPct:18,spentUsd:null,resetsAt:now+86400*2,source:'official',stale:false,status:'ok'});
  await native.evaluate(([a,b])=>window.__emitNative('usage',[a,b]),[week(Math.round(Date.now()/1000)),errorCard]);
  assert.equal((await native.locator('#uList .u-provider').nth(1).locator('.u-values').textContent()).trim(),'\u2013Error');
  assert.match(await native.locator('#uList .u-provider').nth(1).getAttribute('title'),/official/,'The source stays in the tooltip');
  // Keyboard focus survives a card being rebuilt or removed
  await native.locator('#uList .u-face').nth(1).focus();
  const codexWindows = now => [{id:'codex-primary',provider:'codex',label:'Codex 5 h',usedPct:72,spentUsd:null,resetsAt:now+3630,source:'official',stale:false,status:'ok'},{id:'codex-secondary',provider:'codex',label:'Codex weekly',usedPct:9,spentUsd:null,resetsAt:now+86400*5,source:'official',stale:false,status:'ok'}];
  const rebuildNow = Math.round(Date.now()/1000);
  await native.evaluate(now=>{window.__realDateNow=Date.now;Date.now=()=>now*1000;},rebuildNow);
  await native.evaluate(meters=>window.__emitNative('usage',meters),[week(rebuildNow),...codexWindows(rebuildNow)]);
  assert.match(await native.evaluate(()=>document.activeElement.getAttribute('aria-label')),/^OpenAI/,'A rebuilt card keeps focus');
  await native.evaluate(()=>{window.__titleWrites=0;new MutationObserver(list=>{window.__titleWrites+=list.length;}).observe(document.querySelector('#uList .u-provider'),{attributes:true,attributeFilter:['title']});});
  await native.evaluate(meters=>window.__emitNative('usage',meters),[week(rebuildNow),...codexWindows(rebuildNow)]);
  await native.waitForTimeout(20);
  assert.equal(await native.evaluate(()=>window.__titleWrites),0,'An unchanged reading writes nothing');
  await native.evaluate(()=>{Date.now=window.__realDateNow;});
    await native.evaluate(meter=>window.__emitNative('usage',[meter]),week(Math.round(Date.now()/1000)));
  assert.match(await native.evaluate(()=>document.activeElement.getAttribute('aria-label')),/^Claude/,'A removed card hands focus to the next one');
  await native.evaluate(()=>window.__emitNative('usage',[]));
  assert.equal(await native.evaluate(()=>document.activeElement.id),'panel-usage','The last card hands focus to its tab');
  await native.evaluate(meter=>window.__emitNative('usage',[meter]),week(Math.round(Date.now()/1000)));
  await native.evaluate(()=>window.__emitNative('usage',[]));
  assert.equal(await native.locator('.page.usage').isHidden(),true,'No connected provider removes the AI section');
  assert.equal(await native.locator('.section-switch').isHidden(),true,'One remaining Usage section hides the index');
  await native.evaluate(()=>{const now=Math.round(Date.now()/1000);window.__emitNative('usage',[
    {id:'claude-5h',provider:'claude',label:'Claude session (5 h)',usedPct:42,spentUsd:null,resetsAt:now+8130,source:'official',stale:false,status:'ok'},
    {id:'claude-week',provider:'claude',label:'Claude weekly',usedPct:18,spentUsd:null,resetsAt:now+86400*2,source:'official',stale:false,status:'ok'},
    {id:'codex-primary',provider:'codex',label:'Codex 5 h',usedPct:72,spentUsd:null,resetsAt:now+3630,source:'official',stale:false,status:'ok'},
    {id:'codex-secondary',provider:'codex',label:'Codex weekly',usedPct:9,spentUsd:null,resetsAt:now+86400*5,source:'official',stale:false,status:'ok'}]);});
  await setLive({openWidth:500});
  await native.getByRole('tab',{name:'Tools',exact:true}).click();
  await native.getByRole('tab',{name:'Dictation',exact:true}).click();
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'starting',modelReady:true}));
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'recording',modelReady:true}));
  await native.evaluate(()=>window.__emitNative('dictation_partial',{status:'recording',preview:'Provisional words',modelReady:true}));
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'recording',modelReady:true}));
  assert.doesNotMatch(await native.locator('.page.dictation').textContent(),/Live transcript paused/);
  await native.evaluate(()=>window.__emitNative('dictation_partial',{status:'recording',preview:'Late stale preview',modelReady:true}));
  assert.doesNotMatch(await native.locator('.transcript').textContent(),/Provisional words|Late stale preview/);
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'transcribing',modelReady:true}));
  assert.doesNotMatch(await native.locator('.transcript').textContent(),/Provisional words|Late stale preview/);
  assert.match(await native.locator('.page.dictation').textContent(),/Transcribing\u2026/);
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'cancelling',modelReady:true}));
  assert.match(await native.locator('.page.dictation').textContent(),/Cancelling\u2026/);
  await mkdir(path.join(root,'docs/screens'),{recursive:true});
  await capturePolish(native,'dictation-cancelling');
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/dictation-cancelling.png')});
  assert.equal(await native.getByRole('button',{name:'Record',exact:true}).isDisabled(),true);
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'processing',modelReady:true}));
  assert.match(await native.locator('.page.dictation').textContent(),/Microphone closed/);
  await capturePolish(native,'dictation-processing');
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/dictation-processing.png')});
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'starting',modelReady:true}));
  assert.doesNotMatch(await native.locator('.page.dictation').textContent(),/Live transcript paused|Provisional words/);
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'review',transcript:'<b>Text, not markup</b>',modelReady:true}));
  assert.equal(await native.locator('.transcript').textContent(),'<b>Text, not markup</b>');
  assert.equal(await native.locator('.transcript b').count(),0);
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'review',transcript:'Partial insertion fixture',modelReady:true,canInsert:false,message:'Automatic retry disabled; check the target manually.'}));
  assert.equal(await native.getByRole('button',{name:'Insert',exact:true}).isDisabled(),true,'Partial insertions never offer a whole-transcript retry');
  // Dictation has the full content height: status, transcript, a large record control
  const dictationBoxes = await native.evaluate(()=>{const r=s=>document.querySelector(s).getBoundingClientRect();return {page:Math.round(r('.page.dictation').height),record:Math.round(r('.dictation-record').height),cancel:Math.round(r('.dictation-controls .icon-btn').height),fit:r('.dictation-controls').bottom<=r('.page.dictation').bottom+1&&r('.transcript').height>=96};});
  assert.deepEqual(dictationBoxes,{page:232,record:56,cancel:36,fit:true});
  assert.equal(await native.getByRole('button',{name:'Model settings',exact:true}).isVisible(),false,'A ready model needs no settings link');
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'idle',modelInstalled:false,activeModelLabel:null}));
  assert.match(await native.locator('.dictation-status').textContent(),/No speech model yet/);
  assert.equal(await native.getByRole('button',{name:'Record',exact:true}).isDisabled(),true,'No model, no recording');
  await native.getByRole('button',{name:'Model settings',exact:true}).click();
  assert.deepEqual(await native.evaluate(()=>window.__calls.findLast(c=>c.command==='open_settings_cmd').args),{view:'tools'},'The link goes to the model settings');
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'error',message:'Fixture microphone error',modelInstalled:true}));
  assert.match(await native.locator('.dictation-status').textContent(),/Fixture microphone error/);
  assert.equal(await native.locator('.dictation-status .status-dot').getAttribute('data-status'),'warn');
  // Clipboard's empty state is one calm line in the middle
  await native.getByRole('tab',{name:'Tools',exact:true}).click();
  await native.getByRole('tab',{name:'Clipboard',exact:true}).click();
  await native.waitForTimeout(100);
  await native.evaluate(()=>window.__emitNative('clipboard_history_changed',{enabled:true,items:[]}));
  assert.equal((await native.locator('.page.clipboard .empty-line').textContent()).trim(),'Nothing copied yet');
  assert.ok(await native.evaluate(()=>{const p=document.querySelector('.page.clipboard').getBoundingClientRect(),l=document.querySelector('.page.clipboard .empty-line').getBoundingClientRect();return l.top>=p.top && l.bottom<=p.bottom;}),'Empty state fits below the category row');
  await native.evaluate(()=>window.__emitNative('clipboard_history_changed',{enabled:true,items:[{id:'clip',text:'<img src=x>clipboard fixture',pinned:false,createdAt:Date.now()}]}));
  assert.equal(await native.locator('.clipboard-text img').count(),0);
  assert.deepEqual(await native.locator('.clipboard-row .icon-btn').evaluateAll(n=>n.map(x=>x.getAttribute('aria-label'))),['Pin','Remove text']);
  const toolTargets = await native.locator('.page.clipboard .icon-btn, .section-actions .icon-btn:visible').evaluateAll(nodes => nodes.map(n => { const r = n.getBoundingClientRect(); return Math.min(r.width, r.height); }));
  assert.equal(toolTargets.every(size => size >= 32), true, 'Icon buttons are at least 32px');
  await native.locator('.clipboard-text').click();
  assert.equal(await native.evaluate(()=>window.__calls.some(call=>call.command==='clipboard_copy'&&call.args.id==='clip')),true);
  for (const width of [440,500]) {
    await setLive({openWidth:width});
    await native.evaluate(()=>window.__emitNative('clipboard_history_changed',{enabled:true,items:[
      {id:'clip',text:'Clipboard text fixture',pinned:true,createdAt:2},
      {id:'link',text:'https://example.com/fixture',pinned:false,createdAt:1},
    ]}));
    assert.deepEqual(await native.locator('.clipboard-filters button').allTextContents(),['All clips','Text','Links']);
    assert.equal(await native.locator('.clipboard-row').count(),2);
    const sizes = await native.locator('.clipboard-row').evaluateAll(nodes=>nodes.map(n=>Math.round(n.getBoundingClientRect().width)));
    assert.equal(new Set(sizes).size,1);
    await native.getByRole('button',{name:'Links',exact:true}).click();
    assert.equal(await native.locator('.clipboard-row').count(),1);
    assert.equal(await native.locator('.clipboard-row').getAttribute('data-kind'),'link');
    await native.getByRole('button',{name:'All clips',exact:true}).click();
    assert.equal(await native.getByRole('button',{name:'Unpin',exact:true}).getAttribute('aria-pressed'),'true');
    await native.getByRole('button',{name:'Unpin',exact:true}).click();
    assert.ok(await native.evaluate(()=>window.__calls.some(c=>c.command==='clipboard_pin'&&c.args.id==='clip'&&c.args.pinned===false)));
    await native.evaluate(()=>window.__emitNative('clipboard_history_changed',{enabled:true,items:[{id:'clip',text:'Clipboard fixture',pinned:false,createdAt:1}]}));
    await native.getByRole('button',{name:'Remove text',exact:true}).focus();
    await native.keyboard.press('Enter');
    assert.ok(await native.evaluate(()=>window.__calls.some(c=>c.command==='clipboard_delete'&&c.args.id==='clip')));
    await native.evaluate(()=>window.__emitNative('clipboard_history_changed',{enabled:true,items:[{id:'clip',text:'Clipboard fixture',pinned:false,createdAt:1}]}));
  }
  for (const width of [440,500]) {
    await setLive({openWidth:width});
    await native.evaluate(()=>{
      // A wide screenshot, the case that used to crop confusingly
      const canvas=document.createElement('canvas'); canvas.width=128; canvas.height=72; const ctx=canvas.getContext('2d');
      ctx.fillStyle='#f4f4f5';ctx.fillRect(0,0,128,72);ctx.fillStyle='#2b2b30';ctx.fillRect(0,0,128,10);ctx.fillStyle='#64d2ff';ctx.fillRect(8,18,40,6);ctx.fillStyle='#c7c7cc';for(let y=30;y<66;y+=8)ctx.fillRect(8,y,y%16===14?70:112,3);ctx.fillStyle='#ff9f0a';ctx.fillRect(92,16,28,22);
      window.__clipboardFixture={enabled:true,items:[
        {id:'image-fixture',kind:'image',text:'Image',thumbnail:canvas.toDataURL(),pinned:true,createdAt:5},
        {id:'text-fixture',kind:'text',text:'A small thought worth keeping\nSynthetic clipboard fixture',pinned:false,createdAt:4},
        {id:'link-fixture',kind:'link',text:'https://example.com/notes',pinned:false,createdAt:3},
        {id:'video-fixture',kind:'video',text:'Island walkthrough.mp4',paths:['Z:/Fixture/video.mp4'],pinned:false,createdAt:2},
        {id:'doc-fixture',kind:'file',text:'Application form.pdf',paths:['Z:/Fixture/form.pdf'],pinned:false,createdAt:1.5},
        {id:'file-fixture',kind:'file',text:'Project notes.txt',paths:['Z:/Fixture/notes.txt'],missing:true,pinned:false,createdAt:1},
      ]}; window.__emitNative('clipboard_history_changed',window.__clipboardFixture);
    });
    assert.deepEqual(await native.locator('.clipboard-filters button').allTextContents(),['All clips','Text','Links','Images','Videos','Files']);
    assert.equal(await native.locator('.clipboard-row').count(),6);
    await native.locator('.clipboard-row[data-kind="image"] img').evaluate(img=>img.decode());
    assert.equal(await native.locator('.clipboard-row[data-kind="image"] img').evaluate(img=>img.naturalWidth),128);
    assert.equal(await native.locator('.clipboard-row[data-kind="image"] img').evaluate(img=>getComputedStyle(img).objectFit),'cover','The preview fills the card');
    assert.equal(await native.locator('.clipboard-row').first().evaluate(n=>getComputedStyle(n).borderBottomWidth),'0px','No type strip');
    const preview = await native.locator('.clipboard-row[data-kind="image"]').evaluate(n=>{const a=n.getBoundingClientRect(),b=n.querySelector('img').getBoundingClientRect();return [a.width===b.width,a.height===b.height];});
    assert.deepEqual(preview,[true,true],'The image reaches every card edge');
    // Copied files and videos show Explorer's thumbnail; a missing file asks for none and stays neutral
    await native.locator('.clipboard-row[data-clip-id="video-fixture"] .clipboard-file-icon.has-thumb img').waitFor();
    await native.locator('.clipboard-row[data-clip-id="doc-fixture"] .clipboard-file-icon.has-thumb img').waitFor();
    assert.equal(await native.evaluate(()=>window.__calls.some(c=>c.command==='clipboard_thumbnail'&&c.args.id==='file-fixture')),false);
    assert.equal(await native.locator('.clipboard-missing').evaluate(n=>getComputedStyle(n).color),'rgb(161, 161, 166)','Missing is quiet, not an error');
    assert.equal(await native.locator('.clipboard-row[data-clip-id="doc-fixture"] img').evaluate(n=>getComputedStyle(n).objectFit),'cover');
    const thumbCalls = await native.evaluate(()=>window.__calls.filter(c=>c.command==='clipboard_thumbnail').length);
    await native.evaluate(()=>window.__emitNative('clipboard_history_changed',{...window.__clipboardFixture,items:window.__clipboardFixture.items.map(item=>item.id==='text-fixture'?{...item,text:item.text+' '}:item)}));
    assert.equal(await native.locator('.clipboard-row[data-clip-id="video-fixture"] .clipboard-file-icon.has-thumb img').count(),1,'A re-render paints a known thumbnail straight away');
    assert.equal(await native.evaluate(()=>window.__calls.filter(c=>c.command==='clipboard_thumbnail').length),thumbCalls,'Re-renders reuse the thumbnails');
    assert.equal(await native.locator('.clipboard-row').first().getAttribute('data-kind'),'image');
    await native.locator('.clipboard-list').evaluate(el=>el.scrollLeft=0);
    await native.locator('.clipboard-row[data-kind="image"]').hover();
    await native.locator('#panel-tools').screenshot({path:path.join(root,`docs/screens/clipboard-${width}.png`)});
    await native.locator('.clipboard-list').evaluate(el=>el.scrollLeft=el.scrollWidth);
    await native.mouse.move(0,400);
    await native.locator('#panel-tools').screenshot({path:path.join(root,`docs/screens/clipboard-files-${width}.png`)});
    await native.locator('.clipboard-list').evaluate(el=>el.scrollLeft=0);
    const before=await native.locator('.clipboard-list').evaluate(el=>el.scrollLeft);
    await native.locator('.clipboard-list').hover({position:{x:100,y:60}}); await native.mouse.wheel(0,120); await native.waitForTimeout(100);
    assert.ok(await native.locator('.clipboard-list').evaluate(el=>el.scrollLeft)>before);
    assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'tools');
    await native.getByRole('button',{name:'Images',exact:true}).click();
    assert.equal(await native.locator('.clipboard-row').count(),1);
    await native.getByRole('button',{name:'Copy image',exact:true}).click();
    assert.ok(await native.evaluate(()=>window.__calls.some(c=>c.command==='clipboard_copy'&&c.args.id==='image-fixture')));
    await native.getByRole('button',{name:'Remove image',exact:true}).focus(); await native.keyboard.press('Enter');
    assert.ok(await native.evaluate(()=>window.__calls.some(c=>c.command==='clipboard_delete'&&c.args.id==='image-fixture')));
    await native.evaluate(()=>window.__emitNative('clipboard_history_changed',{enabled:true,items:[]}));
    assert.equal(await native.locator('.page.clipboard .empty-line').textContent(),'No images copied yet');
    await native.getByRole('button',{name:'All clips',exact:true}).click();
    await native.evaluate(()=>window.__emitNative('clipboard_history_changed',window.__clipboardFixture));
    await native.getByRole('button',{name:'Videos',exact:true}).click(); assert.equal(await native.locator('.clipboard-file-name').textContent(),'Island walkthrough.mp4');
    await native.getByRole('button',{name:'Files',exact:true}).click();
    assert.equal(await native.locator('.clipboard-missing').textContent(),'File no longer exists');
    await native.evaluate(()=>window.__missingClipboardFile=true);
    await native.locator('.clipboard-row.missing .clipboard-text').click(); assert.equal(await native.locator('.page.clipboard .live-note').textContent(),'File no longer exists');
    await native.evaluate(()=>window.__missingClipboardFile=false);
    await native.getByRole('button',{name:'All clips',exact:true}).click();
  }
  await native.getByRole('tab',{name:'Media',exact:true}).click();
  await native.evaluate(()=>{window.__clipboardMutations=0;window.__clipObserver=new MutationObserver(records=>window.__clipboardMutations+=records.length);window.__clipObserver.observe(document.querySelector('.clipboard-list'),{subtree:true,childList:true,attributes:true});window.__emitNative('clipboard_history_changed',{enabled:true,items:[{id:'clip',text:'Hidden fixture',pinned:false,createdAt:1}]});});
  await native.waitForTimeout(30); assert.equal(await native.evaluate(()=>window.__clipboardMutations),0);
  await native.evaluate(()=>window.__clipObserver.disconnect());
  await native.getByRole('tab',{name:'Tools',exact:true}).click();
  await native.getByRole('tab',{name:'Clipboard',exact:true}).click();
  await setLive({openWidth:500});
  await native.locator('.clipboard-text').focus();
  await native.keyboard.press('ArrowRight');
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'usage');
  assert.equal(await native.evaluate(()=>document.querySelector('.tab-panel.on').contains(document.activeElement)),true,'Keyboard navigation relocates focus into the visible tab');
  // Focus rings are for the keyboard. After a mouse click there is none, even when script moves
  // focus (a list re-rendering, the island focusing itself); Tab still shows one
  const ring = () => native.evaluate(()=>{const el=document.activeElement,s=getComputedStyle(el);return {at:el.id||el.className,ring:s.outlineStyle!=='none'&&parseFloat(s.outlineWidth)>0};});
  await native.getByRole('tab',{name:'Tools',exact:true}).click();
  await native.getByRole('tab',{name:'Clipboard',exact:true}).click();
  await native.evaluate(()=>window.__emitNative('clipboard_history_changed',window.__clipboardFixture));
  await native.locator('.clipboard-row[data-clip-id="text-fixture"] .clipboard-text').click();
  assert.equal((await ring()).ring,false,'No ring on a clicked card');
  await native.evaluate(()=>window.__emitNative('clipboard_history_changed',{...window.__clipboardFixture,items:window.__clipboardFixture.items.map(item=>({...item,text:item.text+' '}))}));
  assert.equal((await ring()).ring,false,'No ring after the list re-renders under the pointer');
  await native.evaluate(()=>window.__emitNative('open-island',null));
  assert.deepEqual(await ring(),{at:'isl',ring:false},'The island focusing itself draws no ring around it');
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/focus-after-click-500.png')});
  await native.keyboard.press('Tab');
  assert.equal((await ring()).ring,true,'Tab still shows where focus is');
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/focus-after-tab-500.png')});

  // Wheel: content scrolls first, then a fresh gesture at the edge switches tabs
  const panelTop = () => native.locator('.tab-panel.on').evaluate(el => el.scrollTop);
  const activeTab = () => native.locator('.tab-panel.on').getAttribute('data-tab');
  await native.getByRole('tab',{name:'Media',exact:true}).click();
  // Below the pill-sized strip, which takes the wheel for volume
  await native.locator('.page.media').hover({position:{x:200,y:60}});
  await native.mouse.wheel(0,-120); await native.waitForTimeout(400);
  assert.equal(await activeTab(),'media','Wheel up on the first tab stays put');
  await native.mouse.wheel(0,120); await native.waitForTimeout(400);
  assert.equal(await activeTab(),'today','Wheel past the last section switches to the next tab');
  assert.equal(await panelTop(),0,'Moving forward lands on the first section');
  await native.mouse.wheel(0,-120); await native.waitForTimeout(400);
  assert.equal(await activeTab(),'media');
  assert.equal(await panelTop(),0,'Media has one section');
  await native.mouse.wheel(120,0); await native.waitForTimeout(400);
  assert.equal(await activeTab(),'today','Horizontal wheel switches tabs');
  // Vertical wheel visits the tab's parts, then moves on; going back lands on the last part
  const selectedPart = () => native.locator('.section-switch-button[aria-selected="true"]').textContent();
  await native.mouse.wheel(0,120); await native.waitForTimeout(400);
  assert.deepEqual([await activeTab(),await selectedPart()],['today','Timer'],'Wheel moves to the next part first');
  await native.mouse.wheel(0,120); await native.waitForTimeout(400);
  assert.equal(await activeTab(),'dictation','Past the last part the wheel moves to the next tab');
  await native.mouse.wheel(0,-120); await native.waitForTimeout(400);
  assert.deepEqual([await activeTab(),await selectedPart()],['today','Timer'],'Going back lands on the last part');
  await native.mouse.wheel(0,-120); await native.waitForTimeout(400);
  assert.deepEqual([await activeTab(),await selectedPart()],['today','Clock']);
  // Two calls since then: the wheel over the far edge of the wider pill, collapsed and open
  assert.equal((await volumeCalls()).length,volumeCallsBeforeOpen + 2,'Wheel outside the strip never changes the volume');
  // Pointer drag swipe: dragging left moves to the next tab
  const area = await native.locator('.pages-container').boundingBox();
  await native.mouse.move(area.x + area.width - 40, area.y + 60);
  await native.mouse.down();
  await native.mouse.move(area.x + area.width - 120, area.y + 62, {steps:4});
  await native.mouse.move(area.x + area.width - 200, area.y + 64, {steps:4});
  await native.mouse.up();
  assert.equal(await activeTab(),'dictation','A left drag swipes to the next tab');
  // The last tab is remembered across sessions
  await native.getByRole('tab',{name:'Usage',exact:true}).click();
  assert.equal(await native.evaluate(() => JSON.parse(localStorage.getItem('tuck.live.v2')).lastTab),'usage');
  // Every tab an older version could have saved still opens
  for (const old of ['media','today','tools']) {
    await native.evaluate(old=>{const key='tuck.live.v2';localStorage.setItem(key,JSON.stringify({...JSON.parse(localStorage.getItem(key)),lastTab:old}));},old);
    await native.reload(); await native.waitForTimeout(150);
    assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),old,'An older saved tab is restored');
  }
  await native.evaluate(()=>{const key='tuck.live.v2';localStorage.setItem(key,JSON.stringify({...JSON.parse(localStorage.getItem(key)),lastTab:'usage'}));});
  await native.reload(); await native.waitForTimeout(150);
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'usage','The last tab is restored');
  // A fresh session has no chosen section, so the first reading to arrive is what shows
  await native.evaluate(()=>window.__emitNative('usage',[{id:'claude-5h',provider:'claude',label:'Claude session (5 h)',usedPct:42,spentUsd:null,resetsAt:Math.round(Date.now()/1000)+8130,source:'official',stale:false,status:'ok'}]));
  await native.waitForTimeout(50);
  assert.equal(await native.locator('.section-switch-button').first().getAttribute('aria-selected'),'true');
  assert.equal(await native.locator('#panel-usage').evaluate(el=>el.scrollTop),0);
  await native.evaluate(()=>window.__emitNative('open-island',null));
  await native.locator('#railSettings').click();
  assert.equal(await native.evaluate(()=>window.__calls.some(call=>call.command==='open_settings_cmd')),true,'The rail settings button opens the settings window');
  await native.goto('http://tuck.test/settings.html#tools');
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'idle',modelInstalled:false,activeModelLabel:null,models:[{id:'ggml-base.bin',label:'Base',bytes:147951465,speed:'Very fast',accuracy:'Good',note:null,state:'found',foundIn:'podcli',active:false}]}));
  assert.match(await native.locator('#dictationSetupStatus').textContent(),/Use an existing model or download/);
  await native.setViewportSize({width:1000,height:850});
  await native.screenshot({path:path.join(root,'docs/screens/model-reuse-settings.png')});
  assert.equal(await native.locator('#dictationReady').isChecked(), true);
  await native.screenshot({path:path.join(root,'docs/screens/dictation-ready-settings.png')});
  await native.setViewportSize({width:520,height:288});
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'review',modelInstalled:true,modelDownloadedBytes:0,activeModelLabel:'Tiny',models:[{id:'ggml-tiny.bin',label:'Tiny',bytes:77691713,speed:'Fastest',accuracy:'Lower',note:null,state:'installed',foundIn:null,active:true},{id:'ggml-base.bin',label:'Base (full size)',bytes:147951465,speed:'Very fast',accuracy:'Good',note:null,state:'found',foundIn:'podcli',active:false}]}));
  assert.equal(await native.locator('.model-row[data-id="ggml-tiny.bin"] button',{hasText:'Remove'}).isDisabled(),true,'Review must be discarded before model removal');
  assert.match(await native.locator('#dictationSetupStatus').textContent(),/In use: Tiny/);
  assert.match(await native.locator('.model-row[data-id="ggml-base.bin"]').textContent(),/Found in podcli/);
  assert.equal(await native.locator('.model-row[data-id="ggml-base.bin"] button',{hasText:'Remove'}).count(),0,'Another app\'s model has no Remove button');
  await native.evaluate(()=>{
    const row={id:'ggml-tiny.bin',label:'Tiny',bytes:77691713,speed:'Fastest',accuracy:'Lower',note:null,state:'installed',foundIn:null,active:true,canRemove:true};
    window.__emitNative('dictation_state',{status:'idle',modelInstalled:true,activeModelLabel:'Tiny',models:[row]});
    window.__emitNative('dictation_state',{status:'idle',modelInstalled:true,activeModelLabel:'Tiny',models:[{...row,canRemove:false}]});
  });
  assert.equal(await native.locator('.model-row[data-id="ggml-tiny.bin"] button',{hasText:'Remove'}).count(),0,'Ownership loss removes the delete action even when phase and active model are unchanged');
  assert.equal(await native.locator('.model-row[data-id="ggml-tiny.bin"] button',{hasText:'Stop using'}).isEnabled(),true);
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'idle',modelInstalled:true,modelDownloadedBytes:0,activeModelLabel:'Tiny',modelNotice:'Base is no longer on this computer. Using Tiny instead.',shortcutProblems:{toggle:'Ctrl+Shift+Space is in use by another app right now. Try again once it is free, or record a different shortcut.',hold:null}}));
  assert.match(await native.locator('#dictationModelNotice').textContent(),/no longer on this computer/);
  assert.match(await native.locator('#dictationShortcutError').textContent(),/in use by another app/);
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'idle',modelInstalled:true,modelDownloadedBytes:0,activeModelLabel:'Tiny',models:[]}));
  assert.equal(await native.locator('#dictationModelNotice').isHidden(),true,'A cleared notice disappears');
  assert.equal(await native.locator('#dictationShortcutError').isHidden(),true,'A fixed shortcut clears its reason');
  await native.locator('#clipboardEnabled').click();
  await native.getByText('Could not save clipboard settings. Current preferences restored below.',{exact:true}).waitFor();
  assert.equal(await native.locator('#clipboardEnabled').isChecked(),false);
  assert.equal(await native.locator('#clipboardPersist').isChecked(),false);
  assert.deepEqual(await native.evaluate(()=>window.__calls.filter(call=>['clipboard_configure','clipboard_set_persistence','clipboard_set_enabled'].includes(call.command)).map(call=>call.command)),['clipboard_configure'],'Both clipboard flags use a single native mutation');
  // Activity switches save immediately and revert when the native save fails.
  await native.locator('#nav a[data-view="activities"]').click();
  await native.setViewportSize({width:1000,height:850});
  await native.screenshot({path:path.join(root,'artifacts','settings-activities.png')});
  await native.setViewportSize({width:520,height:288});
  await native.locator('#ext-notifications').click();
  await native.getByText('Activity settings could not be saved',{exact:true}).waitFor();
  assert.equal(await native.locator('#ext-notifications').isChecked(),false);
  assert.equal(await native.evaluate(()=>window.__calls.findLast(call=>call.command==='extension_settings').args.settings.notifications),true);
  // Hiding the Windows flyout defaults on and reports when it turned itself off
  await native.locator('#nav a[data-view="island"]').click();
  await native.locator('#row-hideSystemFlyout').waitFor({state:'visible'});
  assert.equal(await native.locator('#levelHud').isChecked(),true,'The HUD is on by default');
  assert.equal(await native.locator('#hideSystemFlyout').isChecked(),true,'On by default');
  await native.locator('#hideSystemFlyout').click();
  assert.equal(await native.locator('#hideSystemFlyout').isChecked(),false,'Can be disabled');
  await native.locator('#hideSystemFlyout').click();
  await native.locator('#flyoutStatus').filter({hasText:'Turned off'}).waitFor({timeout:2000});
  assert.equal(await native.locator('#hideSystemFlyout').isChecked(),false,'An unavailable flyout turns the setting back off');
  assert.equal(await native.locator('#flyoutStatus').getAttribute('data-kind'),'error');
  assert.deepEqual(nativeErrors,[]);

  // Recording never publishes words, including legacy late partial events.
  await native.setViewportSize({width:520,height:288});
  await native.goto('http://tuck.test/');
  await native.locator('#isl').waitFor();
  await native.locator('#isl').focus(); await native.keyboard.press('Enter');
  await native.getByRole('tab',{name:'Tools',exact:true}).click();
  await native.getByRole('tab',{name:'Dictation',exact:true}).click();
  const dictationEvent = (revision,status,transcript=null) => native.evaluate(({revision,status,transcript}) => window.__emitNative('dictation_state',{revision,sessionId:1,status,transcript,audioLevel:0.5,modelInstalled:true,activeModelLabel:'Base',canInsert:true}),{revision,status,transcript});
  await dictationEvent(10,'recording');
  await native.waitForTimeout(450);
  assert.equal(await native.locator('.transcript').textContent(),'Listening\u2026');
  await native.evaluate(() => window.__emitNative('dictation_partial',{revision:11,status:'recording',preview:'Legacy words'}));
  assert.doesNotMatch(await native.locator('.transcript').textContent(),/Legacy words/);
  await capturePolish(native,'dictation-recording');
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/dictation-recording.png')});
  // Live words from the stream replace "Listening" and stay provisional until the final text.
  await native.evaluate(() => window.__emitNative('dictation_state',{revision:11,sessionId:1,status:'recording',transcript:null,liveText:'Fixture: this is a local dictation fixture, release the shortcut',audioLevel:0.5,modelInstalled:true,activeModelLabel:'Base',canInsert:true}));
  await native.waitForTimeout(300);
  assert.match(await native.locator('.transcript').textContent(),/release the shortcut/);
  assert.equal(await native.locator('.transcript').evaluate(el => el.classList.contains('live')),true);
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/dictation-live.png')});
  await native.keyboard.press('Escape');
  await native.waitForTimeout(700);
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/dictation-live-collapsed.png')});
  assert.match(await native.locator('#cc .compact-slot.current').textContent(),/release the shortcut$/);
  await native.locator('#isl').focus(); await native.keyboard.press('Enter');
  await native.getByRole('tab',{name:'Tools',exact:true}).click();
  await native.getByRole('tab',{name:'Dictation',exact:true}).click();
  await dictationEvent(12,'review','Fixture: words ready to insert');
  assert.equal(await native.locator('.transcript').evaluate(el => el.classList.contains('live')),false);
  assert.equal(await native.getByRole('button',{name:'Insert',exact:true}).isEnabled(),true);
  await capturePolish(native,'dictation-review');
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/dictation-review.png')});
  await native.getByRole('button',{name:'Insert',exact:true}).click();
  assert.equal(await native.evaluate(()=>window.__calls.some(c=>c.command==='dictation_review_insert')),true);
  await dictationEvent(13,'idle');
  await native.evaluate(()=>window.__emitNative('dictation_state',{revision:14,sessionId:1,status:'idle',message:'Inserted',modelInstalled:true,activeModelLabel:'Base'}));
  await capturePolish(native,'dictation-done');
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/dictation-done.png')});
  await native.locator('#isl').focus();
  await native.keyboard.press('Escape');
  await native.mouse.move(0,220);
  await native.waitForTimeout(600);
  await native.evaluate(()=>window.__emitNative('dictation_transcript',{reason:'Copied'}));
  await native.waitForTimeout(450);
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'peek');
  assert.equal(await native.locator('#statusMessage').textContent(),'Copied, paste with Ctrl+V');
  assert.match(await native.locator('#cc, #openAnnouncement').allTextContents().then(a=>a.join(' ')),/Copied, paste with Ctrl\+V/);
  await capturePolish(native,'dictation-copied');
  await native.locator('#isl').screenshot({path:path.join(root,'docs/screens/dictation-copied.png')});
  assert.deepEqual(nativeErrors,[]);

  // With motion on: what actually keeps running while the island is collapsed
  const motionContext = await browser.newContext({viewport:{width:520,height:288}, reducedMotion:'no-preference'});
  await motionContext.route('http://tuck.test/**', serveDist);
  await motionContext.addInitScript(nativeBridge);
  await motionContext.addInitScript(instrumentTimers);
  const motion = await motionContext.newPage();
  const motionErrors = []; motion.on('pageerror',error=>motionErrors.push(error.message));
  await motion.goto('http://tuck.test/');
  const running = () => motion.evaluate(()=>document.getAnimations().filter(a=>a.playState==='running').map(a=>a.animationName||a.transitionProperty).sort());
  const emitTrack = (title,playing) => motion.evaluate(([title,playing])=>window.__emitNative('media',{active:true,trackId:title,title,artist:'Fixture',playing,positionMs:0,durationMs:120000,artwork:null}),[title,playing]);
  const longTitle = 'A synthetic title just long enough to need a single scroll here';
  await motion.waitForTimeout(600);
  assert.deepEqual(await running(),[],'Collapsed with nothing playing runs no animations');
  await emitTrack(longTitle,false);
  await motion.waitForTimeout(600);
  assert.deepEqual(await running(),[],'A paused track with an overflowing open-page title runs no animations while collapsed');
  await motion.locator('#isl').focus(); await motion.keyboard.press('Enter');
  await motion.waitForTimeout(600);
  assert.ok((await running()).includes('marquee-anim'),'The open page title still marquees');
  await motion.keyboard.press('Escape');
  await motion.waitForTimeout(600);
  assert.deepEqual(await running(),[],'Closing stops the open page title');
  await emitTrack(longTitle,true);
  await motion.waitForTimeout(700);
  assert.ok((await running()).includes('compact-marquee-once'),'A long collapsed line scrolls');
  assert.equal((await running()).filter(name=>name==='eq-bounce').length,3,'The equalizer runs during playback');
  await motion.waitForFunction(()=>!document.getAnimations().some(a=>a.animationName==='compact-marquee-once'),null,{timeout:15000});
  assert.deepEqual([...new Set(await running())],['eq-bounce'],'Only the equalizer runs after the one scroll');
  assert.equal(await motion.locator('#cc .compact-slot.current .compact-title').evaluate(n=>getComputedStyle(n).textOverflow==='ellipsis'&&n.scrollWidth>n.clientWidth&&!n.classList.contains('scroll-once')),true,'The line stops on an ellipsis');
  await emitTrack(longTitle,true);
  await motion.waitForTimeout(600);
  assert.deepEqual([...new Set(await running())],['eq-bounce'],'The same text does not re-arm the scroll');
  await emitTrack(`${longTitle} again`,true);
  await motion.waitForTimeout(600);
  assert.ok((await running()).includes('compact-marquee-once'),'New text re-arms the scroll');
  await emitTrack(`${longTitle} again`,false);
  await motion.waitForTimeout(1600);
  assert.deepEqual(await running(),[],'The equalizer stops on pause and nothing else runs');
  assert.equal(await motion.evaluate(() => window.__idleIntervals.size), 0, 'Pausing collapsed media parks all intervals');
  // Native bounds and the smooth part switch, in both shapes
  await motion.locator('#isl').focus(); await motion.keyboard.press('Enter');
  await motion.waitForTimeout(600);
  const lastHitBox = () => motion.evaluate(()=>{const {w,h}=window.__calls.findLast(call=>call.command==='set_hit').args;return [w,h];});
  assert.deepEqual(await lastHitBox(),[500,264],'The open hit area matches the notch island');
  await motion.getByRole('tab',{name:'Today',exact:true}).click();
  await motion.getByRole('tab',{name:'Timer',exact:true}).click();
  await motion.waitForFunction(()=>{const p=document.getElementById('panel-today');return Math.abs(p.scrollTop-p.clientHeight)<1;},null,{timeout:3000});
  await motion.locator('.section-switch-button[aria-selected="true"]',{hasText:'Timer'}).waitFor({timeout:3000});
  assert.equal(await motion.locator('#actPause').evaluate(n=>!n.closest('[inert]')),true,'The part in view is interactive after a smooth move');
  // Hidden tabs do no polling
  await motion.getByRole('tab',{name:'Dictation',exact:true}).click();
  await motion.evaluate(()=>{window.__calls.length=0;});
  await motion.waitForTimeout(1300);
  assert.deepEqual(await motion.evaluate(()=>window.__calls.filter(call=>['telemetry_now','media_sessions'].includes(call.command)).length),0,'Dictation polls nothing');
  await motion.evaluate(m=>window.__emitNative('usage',m),usageFixture(20));
  await motion.getByRole('tab',{name:'Usage',exact:true}).click();
  await motion.getByRole('tab',{name:'System',exact:true}).click();
  await motion.waitForFunction(()=>window.__calls.some(call=>call.command==='telemetry_now'),null,{timeout:3000});
  await motion.getByRole('tab',{name:'AI',exact:true}).click();
  await motion.waitForTimeout(150); await motion.evaluate(()=>{window.__calls.length=0;});
  await motion.waitForTimeout(1200);
  assert.equal(await motion.evaluate(()=>window.__calls.filter(c=>c.command==='telemetry_now').length),0,'System stops sampling on the AI part');

  await motion.evaluate(()=>localStorage.setItem('notch.shape','float'));
  await motion.reload(); await motion.waitForTimeout(150);
  await motion.locator('#isl').focus(); await motion.keyboard.press('Enter');
  await motion.waitForTimeout(600);
  assert.deepEqual(await lastHitBox(),[500,274],'A floating island adds its gap to the hit area');
  const floatBox = await motion.locator('#isl').boundingBox();
  assert.ok(floatBox.y===10&&floatBox.y+floatBox.height<=288&&floatBox.height===264,'A floating island fits the native canvas');
  assert.deepEqual(motionErrors,[]);
  // The compact clock stays accurate with one minute-aligned timeout, without an interval.
  const idleClock = await context.newPage();
  const minuteEdge = new Date('2026-11-04T12:00:59.900Z');
  await idleClock.clock.install({time:new Date(minuteEdge.getTime() - 1000)});
  await idleClock.clock.pauseAt(minuteEdge);
  await idleClock.goto('http://tuck.test/');
  const beforeMinute = await idleClock.locator('.compact').innerText();
  await idleClock.clock.runFor(200);
  assert.notEqual(await idleClock.locator('.compact').innerText(), beforeMinute, 'Collapsed clock advances at the next minute');
  assert.equal(await idleClock.evaluate(() => window.__idleIntervals.size), 0, 'The minute clock does not start an interval');
  await idleClock.close();

  const readySettings = await context.newPage();
  await readySettings.setViewportSize({width:1000,height:850});
  await readySettings.addInitScript(() => {
    let settings = {toggleShortcut:'Ctrl+Shift+Space',holdToTalkShortcut:'RightAlt',reviewBeforeInsert:false,keepMicrophoneReady:true};
    const snapshot = () => ({status:'idle',modelInstalled:false,models:[],settings});
    window.__readySaves = [];
    window.__TAURI_INTERNALS__ = {transformCallback:()=>1, async invoke(command,args) {
      if(command === 'dictation_get_state') return snapshot();
      if(command === 'dictation_set_settings') { settings = args.settings; window.__readySaves.push(settings.keepMicrophoneReady); return snapshot(); }
      return null;
    }};
  });
  await readySettings.goto('http://tuck.test/settings.html#tools');
  await readySettings.locator('#dictationReady:enabled').waitFor();
  assert.equal(await readySettings.locator('#dictationReady').isChecked(),true);
  await readySettings.locator('#dictationReady').click();
  await readySettings.waitForFunction(()=>window.__readySaves.length===1);
  assert.equal(await readySettings.locator('#dictationReady').isChecked(),false);
  await readySettings.locator('#dictationReady:enabled').click();
  await readySettings.waitForFunction(()=>window.__readySaves.length===2);
  assert.deepEqual(await readySettings.evaluate(()=>window.__readySaves),[false,true]);
  await readySettings.screenshot({path:path.join(root,'docs/screens/dictation-ready-settings.png')});
  await readySettings.close();

  console.log('UI smoke passed: geometry, tab rail (keys, wheel edges, swipe, memory), accessibility, keyboard seeking, timers, collapsed media text and its running animations, usage rings, segmented switch, dictation tab, empty states, icon buttons, persistence errors, volume/brightness HUD, wheel and middle-click volume, the pill-sized strip on the open island, queued flyout failure, return to open after HUD, HUD setting, charger-ramp brightness, flyout default-on, shelf, drag hold and previews.');
} finally { await browser.close(); }

