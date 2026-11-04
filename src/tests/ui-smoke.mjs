import { chromium } from '@playwright/test';
import { readFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
const root = process.cwd();
const browser = await chromium.launch({channel:'msedge',headless:true});
try {
  const context = await browser.newContext({viewport:{width:520,height:230}, reducedMotion:'reduce'});
  const serveDist = async route => {
    const pathname = new URL(route.request().url()).pathname;
    const file = path.join(root, 'dist', pathname === '/' ? 'index.html' : pathname);
    const types = {'.html':'text/html','.js':'application/javascript','.css':'text/css','.svg':'image/svg+xml'};
    try { await route.fulfill({body:await readFile(file),contentType:types[path.extname(file)] || 'application/octet-stream'}); }
    catch { await route.fulfill({status:404,body:'Not found'}); }
  };
  await context.route('http://tuck.test/**', serveDist);
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
  // Sidebar: one view at a time, arrow keys move between sections.
  assert.equal(await page.locator('#view-general').isVisible(),true);
  assert.equal(await page.locator('#view-keys').isVisible(),false);
  await page.locator('#nav a[data-view="general"]').focus();
  await page.keyboard.press('ArrowDown');
  await page.locator('#view-island').waitFor({state:'visible'});
  assert.equal(await page.locator('#nav a[data-view="island"]').getAttribute('aria-current'),'page');
  // Search filters rows across every view, and clears with Escape.
  await page.keyboard.press('Control+f');
  assert.equal(await page.evaluate(() => document.activeElement.id),'settingsSearch');
  await page.keyboard.type('anthropic key');
  assert.equal(await page.locator('#row-kAnthropic').isVisible(),true);
  assert.equal(await page.locator('#row-kOpenai').isVisible(),false);
  assert.equal(await page.locator('#row-openDelay').isVisible(),false);
  await page.keyboard.press('Escape');
  await page.locator('#view-island').waitFor({state:'visible'});
  // A default profile (180ms expand delay) must save a switch change straight away
  await page.evaluate(() => localStorage.removeItem('tuck.live.v2'));
  await page.reload(); await page.locator('#nav a[data-view="island"]').click();
  await page.locator('#view-island').waitFor({state:'visible'});
  const hoverBefore = await page.locator('#hoverExpand').isChecked();
  await page.locator('#hoverExpand').click();
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('tuck.live.v2') || '{}').hoverExpand), !hoverBefore, 'Default delays do not block saving');
  await page.locator('#hoverExpand').click(); // restore for the island checks below
  assert.equal(await page.locator('#row-openDelay').isVisible(),true);
  // Collapsed content offers all three modes and defaults to the lyric
  await page.evaluate(() => document.querySelector('#row-compactContent').closest('.view').dataset.view).then(view => page.locator(`#nav a[data-view="${view}"]`).click());
  assert.deepEqual(await page.locator('#row-compactContent .segmented label').allTextContents(),['Artwork','Title','Lyric']);
  assert.equal(await page.locator('input[name="compactContent"]:checked').getAttribute('value'),'lyrics');
  await page.locator('#row-compactContent label',{hasText:'Artwork'}).click();
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('tuck.live.v2')).compactContent),'artwork');
  await page.locator('#row-compactContent label',{hasText:'Lyric'}).click();
  await page.locator('#nav a[data-view="keys"]').click();
  await page.locator('#kAnthropic').fill('test-placeholder-not-a-key');
  await page.locator('#saveKeys').click();
  assert.equal(await page.locator('#kAnthropic').inputValue(),'');
  assert.match(await page.locator('#settingsStatus').textContent(),/failed/);
  // Removing a key asks first, inline.
  await page.locator('#removeOpenai').click();
  assert.doesNotMatch(await page.locator('#settingsStatus').textContent(),/Key removed/);
  await page.locator('#row-kOpenai').getByRole('button',{name:'Remove key',exact:true}).click();
  assert.match(await page.locator('#settingsStatus').textContent(),/failed/);
  await page.locator('#nav a[data-view="usage"]').click();
  await page.locator('#cCap').fill('-1');
  await page.locator('#cHours').fill('3');
  await page.locator('#cHours').press('Enter');
  assert.match(await page.locator('#chatStatus').textContent(),/positive/);
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
  await page.locator('#bAnthropic').fill('50');
  await page.locator('#bAnthropic').press('Enter');
  assert.match(await page.locator('#budgetStatus').textContent(),/Could not save budgets/);
  assert.equal(await page.locator('#row-bAnthropic').evaluate(el => el.classList.contains('saved')),false);
  await page.screenshot({path:path.join(root,'artifacts','settings-usage.png')});
  assert.deepEqual(errors,[]);

  // A mocked native bridge exercises the actual controllers; this is not a hardware smoke.
  const nativeBridge = () => {
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
      if(command==='volume_step')return {kind:'volume',level:40+args.delta,muted:false};
      if(command==='volume_toggle_mute')return {kind:'volume',level:40,muted:true};
      if(command==='get_hide_system_flyout')return {enabled:false,state:'off',message:null};
      if(command==='set_hide_system_flyout')return {enabled:false,state:'unavailable',message:"Turned off: Windows showed its volume flyout several times, but Tuck couldn't find it on this version of Windows"};
      return null;
    }};
  };
  await context.addInitScript(nativeBridge);
  const native=await context.newPage();const nativeErrors=[];native.on('pageerror',error=>nativeErrors.push(error.message));
  await native.goto('http://tuck.test/');await native.waitForTimeout(100);
  assert.equal(await native.locator('.page.todos').count(), 0, 'Todos feature must be completely removed');
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
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'peek');
  assert.deepEqual(await flyoutPause(),[false,true],'A peek over the pinned island does not hide the flyout again');
  await native.evaluate(()=>window.__emitNative('system-level',{kind:'volume',level:55,muted:false}));
  assert.deepEqual(await hudState(),['peek','volume'],'The HUD interrupts the announcement');
  await native.locator('#cc .compact-slot.current').filter({hasText:'Caps Lock: On'}).waitFor({timeout:3000});
  await native.waitForFunction(()=>document.getElementById('isl').dataset.state==='open',null,{timeout:6000});
  await setLive({pinned:false});
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
  const radios = await native.getByRole('button',{name:'Radios',exact:true}).boundingBox();
  const radiosX = radios.x + radios.width / 2;
  const radiosY = Math.min(radios.y + radios.height / 2, openBox.y + 30);
  assert.ok(radios.y < openBox.y + 32 && Math.abs(radiosX - stripX) < 106,'A view button sits under the strip');
  await native.mouse.click(radiosX, radiosY);
  await native.getByRole('switch',{name:'Test Wi-Fi',exact:true}).waitFor({timeout:2000});
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
  await native.waitForFunction(()=>document.querySelector('.tab-panel.on').dataset.tab==='tools',null,{timeout:2000});
  assert.equal((await volumeCalls()).length,callsBeforeElsewhere,'Scrolling outside the strip never changes the volume');
  await native.getByRole('tab',{name:'Media',exact:true}).click();
  await native.evaluate(()=>window.__emitNative('hover',false));
  await toIdle();
  await native.mouse.move(0, 220);
  const volumeCallsBeforeOpen = (await volumeCalls()).length;
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

  // Closed media content stays present while playing and uses one stable width per track.
  await native.evaluate(()=>document.getElementById('actReset').click());
  await native.evaluate(()=>window.dispatchEvent(new CustomEvent('tuck-activity',{detail:{id:'native-call',type:'call',title:'Manual call timer',text:'',priority:95,active:false}})));
  await native.evaluate(()=>window.__emitNative('media',{active:true,trackId:'fixture-track',title:'Artwork color fixture',artist:'Synthetic test data',playing:true,positionMs:3000,durationMs:120000,artwork:null}));
  await setLive({compactContent:'lyrics'});
  await native.keyboard.press('Escape');
  await native.waitForTimeout(450);
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'idle');
  assert.equal((await native.locator('#isl').boundingBox()).width,300,'Lyric mode reserves a stable wider closed pill');
  assert.equal((await native.locator('#cc .compact-title-track').textContent()).trim(),'Current fixture line');
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
  // Dictation outranks media and gives the pill back its base width
  await native.evaluate(()=>window.dispatchEvent(new CustomEvent('tuck-activity',{detail:{id:'dictation',type:'dictation',phase:'recording',title:'Dictation',text:'',priority:100,active:true}})));
  assert.equal(await native.locator('.compact').getAttribute('aria-label'),'Dictation recording');
  await native.waitForTimeout(50);
  assert.equal((await native.locator('#isl').boundingBox()).width,212,'A higher-priority activity takes the slot');
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
  // AI usage: one provider card, double rings for 5h + 7d, official marks, no unconnected cards
  await native.evaluate(()=>{
    localStorage.removeItem('notch.chatgpt');
    const now=Math.round(Date.now()/1000);
    window.__emitNative('usage',[
      {id:'claude-5h',provider:'claude',label:'Claude session (5 h)',usedPct:42,spentUsd:null,resetsAt:now+8130,source:'official',stale:false,status:'ok'},
      {id:'claude-week',provider:'claude',label:'Claude weekly',usedPct:18,spentUsd:null,resetsAt:now+86400*2,source:'official',stale:false,status:'ok'},
      {id:'codex-installed-c-primary',provider:'codex',label:'Codex 5 h',usedPct:72,spentUsd:null,resetsAt:now+3630,source:'official',stale:false,status:'ok'},
      {id:'codex-installed-c-secondary',provider:'codex',label:'Codex weekly',usedPct:9,spentUsd:null,resetsAt:now+86400*5,source:'official',stale:true,status:'stale'},
      {id:'openai-api',provider:'openai-api',label:'OpenAI API',usedPct:null,spentUsd:null,resetsAt:null,source:'api',stale:true,status:'unconfigured'}]);
  });
  await native.locator('.page.usage').scrollIntoViewIfNeeded();
  assert.equal(await native.locator('#uList .u-provider').count(),2,'Two connected providers become two cards');
  assert.equal(await native.locator('#uList .u-provider .brand-mark').count(),2,'Every provider shows its real mark');
  assert.equal(await native.locator('#uList .u-ring-outer').count(),2);
  assert.equal(await native.locator('#uList .u-ring-inner').count(),2);
  await native.waitForTimeout(100);
  assert.deepEqual(await native.locator('#uList .ring').evaluateAll(n=>n.map(x=>x.dataset.tone)),['ok','ok','caution','ok']);
  assert.deepEqual(await native.locator('#uList .u-provider').first().locator('.u-value-line').evaluateAll(n=>n.map(x=>x.textContent)),['5h42%','7d18%']);
  assert.match(await native.locator('#uList .u-face').first().getAttribute('aria-label'),/Claude\. 5h, 42% used.*7d, 18% used/);
  assert.equal(await native.locator('#uList .u-provider').nth(1).getAttribute('data-stale'),'true','A stale configured provider remains and dims');
  assert.equal(await native.locator('#uList .u-provider').nth(1).locator('.u-state').textContent(),'Stale','A stale provider says so');
  assert.equal(await native.locator('#uList .u-provider').first().locator('.u-state').isHidden(),true,'A fresh provider has no state word');
  assert.match(await native.locator('#uList .u-provider').first().getAttribute('title'),/resets in 2h 15m/,'Reset times live in the tooltip');
  const ringTargets = await native.locator('#uList .u-face').evaluateAll(nodes => nodes.map(n => n.getBoundingClientRect().width));
  assert.equal(ringTargets.every(size => size >= 72), true, 'Provider rings are larger than the old rings');

  // Every card fits, whatever the island width and however many providers are connected
  const fiveProviders = now => [
    {id:'claude-5h',provider:'claude',label:'Claude session (5 h)',usedPct:100,spentUsd:null,resetsAt:now+8130,source:'official',stale:false,status:'ok'},
    {id:'claude-week',provider:'claude',label:'Claude weekly',usedPct:93,spentUsd:null,resetsAt:now+86400*2,source:'official',stale:false,status:'ok'},
    {id:'codex-primary',provider:'codex',label:'Codex 5 h',usedPct:72,spentUsd:null,resetsAt:now+3630,source:'official',stale:false,status:'ok'},
    {id:'codex-secondary',provider:'codex',label:'Codex weekly',usedPct:9,spentUsd:null,resetsAt:now+86400*5,source:'official',stale:true,status:'stale'},
    {id:'anthropic-api',provider:'anthropic-api',label:'Claude API',usedPct:null,spentUsd:1234.5,resetsAt:now+86400,source:'api',stale:false,status:'ok'},
    {id:'openai-api',provider:'openai-api',label:'OpenAI API',usedPct:null,spentUsd:null,resetsAt:now+86400,source:'api',stale:true,status:'error',error:'Credential Manager read failed'},
  ];
  const usageFit = () => native.evaluate(()=>{
    const list=document.getElementById('uList'),page=document.querySelector('.page.usage').getBoundingClientRect();
    const container=document.querySelector('.pages-container').getBoundingClientRect();
    const inside=r=>r.left>=container.left-1&&r.right<=container.right+1&&r.top>=page.top-1&&r.bottom<=page.bottom+1;
    const parts=[...list.querySelectorAll('.u-face,.u-values,.u-value-line,.u-state')].filter(n=>n.getBoundingClientRect().height>0);
    const clear=[...list.querySelectorAll('.u-provider')].every(card=>card.querySelector('.u-face').getBoundingClientRect().right<=card.querySelector('.u-values').getBoundingClientRect().left&&card.querySelector('.u-values').scrollWidth<=card.querySelector('.u-values').clientWidth+1);
    const marks=[...document.querySelectorAll('.section-index-button')].map(n=>n.getBoundingClientRect());
    return {inside:parts.every(n=>inside(n.getBoundingClientRect())),clear,marksClear:marks.every(r=>r.left>=container.right-0.5),fits:list.scrollHeight<=page.height+1};
  });
  for (const width of [440,500]) {
    await setLive({openWidth:width});
    await native.waitForTimeout(50);
    assert.deepEqual(await usageFit(),{inside:true,clear:true,marksClear:true,fits:true},`Two provider cards fit at ${width}px`);
    await native.evaluate(meters=>window.__emitNative('usage',meters),fiveProviders(Math.round(Date.now()/1000)));
    await native.waitForTimeout(50);
    assert.equal(await native.locator('#uList .u-provider').count(),4,'Each connected source is one card');
    assert.deepEqual(await usageFit(),{inside:true,clear:true,marksClear:true,fits:true},`Four provider cards fit at ${width}px`);
    await native.evaluate(()=>{localStorage.setItem('notch.chatgpt',JSON.stringify({label:'ChatGPT (estimate)',cap:10,hours:3}));localStorage.removeItem('notch.chatgpt.log');window.dispatchEvent(new StorageEvent('storage',{key:'notch.chatgpt'}));});
    await native.waitForTimeout(50);
    assert.equal(await native.locator('#uList .u-provider').count(),5);
    assert.equal(await native.locator('#uList').evaluate(n=>n.classList.contains('dense')),true);
    assert.deepEqual(await usageFit(),{inside:true,clear:true,marksClear:true,fits:true},`Five provider cards fit at ${width}px`);
    // The ChatGPT estimate is logged by pressing its ring
    const logButton = native.getByRole('button',{name:/^Log a ChatGPT message/});
    await logButton.click();
    assert.equal(await native.locator('#uList .u-provider').last().locator('.u-val').textContent(),'1/10');
    assert.equal(await logButton.evaluate(n=>document.activeElement===n),true,'Logging keeps focus on the ring');
    await native.evaluate(()=>{localStorage.removeItem('notch.chatgpt');localStorage.removeItem('notch.chatgpt.log');});
    await native.evaluate(meters=>window.__emitNative('usage',meters),fiveProviders(Math.round(Date.now()/1000)).slice(0,4));
  }
  assert.equal(await native.locator('#uList .u-provider').count(),2);

  // The section indicator is an actual control and follows the same snap scroller.
  const sectionButtons = native.locator('.section-index-button');
  assert.equal(await sectionButtons.count(),2);
  assert.equal(await sectionButtons.first().getAttribute('aria-label'),'Go to AI usage');
  assert.equal(await sectionButtons.first().getAttribute('aria-current'),'true');
  const sectionTargets = await sectionButtons.evaluateAll(nodes=>nodes.map(node=>{const r=node.getBoundingClientRect();return [r.width,r.height];}));
  assert.equal(sectionTargets.every(([w,h])=>w>=24&&h>=24),true,'Section controls have 24px hit targets');
  await sectionButtons.nth(1).focus();
  assert.equal(await sectionButtons.nth(1).evaluate(node=>document.activeElement===node),true,'Section controls are keyboard focusable');
  await sectionButtons.nth(1).click();
  await native.waitForTimeout(400);
  assert.equal(await sectionButtons.nth(1).getAttribute('aria-current'),'true');
  assert.equal(await native.locator('#isl').getAttribute('data-state'),'open','Section clicks do not collapse the island');
  const usageTop = () => native.locator('#panel-usage').evaluate(el => el.scrollTop);
  assert.ok(await usageTop() > 0,'A click snaps the panel to its section');
  assert.equal(await native.evaluate(()=>JSON.parse(localStorage.getItem('tuck.live.v2')).lastTab),'usage','A section click keeps the tab remembered');
  await sectionButtons.first().focus();
  await native.keyboard.press('Enter');
  await native.waitForTimeout(400);
  assert.equal(await usageTop(),0,'The keyboard reaches the same navigation');
  assert.equal(await sectionButtons.first().evaluate(n=>getComputedStyle(n).outlineStyle),'solid','A focused mark shows its focus ring');
  // The wheel over the marks moves between sections, like over the page, and only then between tabs
  await native.waitForTimeout(400);
  await sectionButtons.first().hover();
  await native.mouse.wheel(0,120); await native.waitForTimeout(400);
  assert.equal(await sectionButtons.nth(1).getAttribute('aria-current'),'true','Wheel over the marks scrolls to the next section');
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'usage');
  // The section count can change while the tab is open
  await native.evaluate(()=>window.__emitNative('usage',[]));
  assert.equal(await native.locator('.section-index').isHidden(),true);
  assert.equal(await native.evaluate(()=>document.querySelector('.tab-panel.on').contains(document.activeElement)||document.activeElement===document.querySelector('.tab-panel.on')),true,'Focus does not fall out of the island when the marks leave');
  await native.evaluate(()=>{const now=Math.round(Date.now()/1000);window.__emitNative('usage',[
    {id:'claude-5h',provider:'claude',label:'Claude session (5 h)',usedPct:42,spentUsd:null,resetsAt:now+8130,source:'official',stale:false,status:'ok'}]);});
  assert.equal(await sectionButtons.nth(1).getAttribute('aria-current'),'true','A section the reader moved to stays in view when another returns above it');
  assert.ok(await usageTop() > 0);
  await sectionButtons.first().click(); await native.waitForTimeout(400);
  assert.equal(await sectionButtons.first().getAttribute('aria-current'),'true');
  assert.deepEqual(await sectionButtons.evaluateAll(n=>n.map(x=>getComputedStyle(x).color)),['rgb(161, 161, 166)','rgb(99, 99, 102)'],'Inactive marks are --ink-3, the active one --ink-2');
  assert.deepEqual(await sectionButtons.first().evaluate(n=>[getComputedStyle(n,'::before').left,getComputedStyle(n,'::before').right]),['4px','4px'],'The hover fill keeps a gap from the page beside it');
  // ArrowLeft from a mark moves into the tab, as anywhere else in the open island
  await sectionButtons.nth(1).focus();
  await native.keyboard.press('ArrowLeft');
  assert.equal(await native.locator('.tab-panel.on').getAttribute('data-tab'),'tools');
  assert.equal(await native.evaluate(()=>document.querySelector('.tab-panel.on').contains(document.activeElement)||document.activeElement===document.querySelector('.tab-panel.on')),true,'Focus moves into the new tab');
  // Marks belong to the tab on screen, even when two tabs have the same number of sections
  await native.getByRole('tab',{name:'Media',exact:true}).click();
  assert.deepEqual(await sectionButtons.evaluateAll(n=>n.map(x=>x.getAttribute('aria-label'))),['Go to now playing','Go to visualizer']);
  await native.getByRole('tab',{name:'Today',exact:true}).click();
  assert.equal(await sectionButtons.count(),3);
  await native.getByRole('tab',{name:'Usage',exact:true}).click();
  assert.deepEqual(await sectionButtons.evaluateAll(n=>n.map(x=>x.getAttribute('aria-label'))),['Go to AI usage','Go to system']);

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
  await native.evaluate(meters=>window.__emitNative('usage',meters),[week(Math.round(Date.now()/1000)),...codexWindows(Math.round(Date.now()/1000))]);
  assert.match(await native.evaluate(()=>document.activeElement.getAttribute('aria-label')),/^OpenAI/,'A rebuilt card keeps focus');
  await native.evaluate(()=>{window.__titleWrites=0;new MutationObserver(list=>{window.__titleWrites+=list.length;}).observe(document.querySelector('#uList .u-provider'),{attributes:true,attributeFilter:['title']});});
  await native.evaluate(meters=>window.__emitNative('usage',meters),[week(Math.round(Date.now()/1000)),...codexWindows(Math.round(Date.now()/1000))]);
  await native.waitForTimeout(20);
  assert.equal(await native.evaluate(()=>window.__titleWrites),0,'An unchanged reading writes nothing');
    await native.evaluate(meter=>window.__emitNative('usage',[meter]),week(Math.round(Date.now()/1000)));
  assert.match(await native.evaluate(()=>document.activeElement.getAttribute('aria-label')),/^Claude/,'A removed card hands focus to the next one');
  await native.evaluate(()=>window.__emitNative('usage',[]));
  assert.equal(await native.evaluate(()=>document.activeElement.id),'panel-usage','The last card hands focus to its tab');
  await native.evaluate(meter=>window.__emitNative('usage',[meter]),week(Math.round(Date.now()/1000)));
  await native.evaluate(()=>window.__emitNative('usage',[
    {id:'openai-api',provider:'openai-api',label:'OpenAI API',usedPct:null,spentUsd:null,resetsAt:null,source:'api',stale:true,status:'unconfigured'}]));
  assert.equal(await native.locator('.page.usage').isHidden(),true,'No connected provider removes the AI section');
  assert.equal(await native.locator('.section-index').isHidden(),true,'One remaining Usage section hides the index');
  await native.evaluate(()=>{const now=Math.round(Date.now()/1000);window.__emitNative('usage',[
    {id:'claude-5h',provider:'claude',label:'Claude session (5 h)',usedPct:42,spentUsd:null,resetsAt:now+8130,source:'official',stale:false,status:'ok'},
    {id:'claude-week',provider:'claude',label:'Claude weekly',usedPct:18,spentUsd:null,resetsAt:now+86400*2,source:'official',stale:false,status:'ok'},
    {id:'codex-primary',provider:'codex',label:'Codex 5 h',usedPct:72,spentUsd:null,resetsAt:now+3630,source:'official',stale:false,status:'ok'},
    {id:'codex-secondary',provider:'codex',label:'Codex weekly',usedPct:9,spentUsd:null,resetsAt:now+86400*5,source:'official',stale:false,status:'ok'}]);});
  await setLive({openWidth:500});
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
  // Below the pill-sized strip, which takes the wheel for volume
  await native.locator('.page.media').hover({position:{x:200,y:60}});
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
  // Two calls since then: the wheel over the far edge of the wider pill, collapsed and open
  assert.equal((await volumeCalls()).length,volumeCallsBeforeOpen + 2,'Wheel outside the strip never changes the volume');
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
  // A fresh session has no chosen section, so the first reading to arrive is what shows
  await native.evaluate(()=>window.__emitNative('usage',[{id:'claude-5h',provider:'claude',label:'Claude session (5 h)',usedPct:42,spentUsd:null,resetsAt:Math.round(Date.now()/1000)+8130,source:'official',stale:false,status:'ok'}]));
  await native.waitForTimeout(50);
  assert.equal(await native.locator('.section-index-button').first().getAttribute('aria-current'),'true');
  assert.equal(await native.locator('#panel-usage').evaluate(el=>el.scrollTop),0);
  await native.evaluate(()=>window.__emitNative('open-island',null));
  await native.locator('#railSettings').click();
  assert.equal(await native.evaluate(()=>window.__calls.some(call=>call.command==='open_settings_cmd')),true,'The rail settings button opens the settings window');
  await native.goto('http://tuck.test/settings.html#tools');
  await native.evaluate(()=>window.__emitNative('dictation_state',{status:'review',modelInstalled:true,modelDownloadedBytes:0}));
  assert.equal(await native.locator('#removeModel').isDisabled(),true,'Review must be discarded before model removal');
  await native.locator('#clipboardEnabled').click();
  await native.getByText('Could not save clipboard settings. Current preferences restored below.',{exact:true}).waitFor();
  assert.equal(await native.locator('#clipboardEnabled').isChecked(),false);
  assert.equal(await native.locator('#clipboardPersist').isChecked(),false);
  assert.deepEqual(await native.evaluate(()=>window.__calls.filter(call=>['clipboard_configure','clipboard_set_persistence','clipboard_set_enabled'].includes(call.command)).map(call=>call.command)),['clipboard_configure'],'Both clipboard flags use a single native mutation');
  // Activity switches save immediately and revert when the native save fails.
  await native.locator('#nav a[data-view="activities"]').click();
  await native.locator('#ext-downloads').click();
  await native.getByText('Activity settings could not be saved',{exact:true}).waitFor();
  assert.equal(await native.locator('#ext-downloads').isChecked(),false);
  assert.equal(await native.evaluate(()=>window.__calls.findLast(call=>call.command==='extension_settings').args.settings.downloads),true);
  await native.screenshot({path:path.join(root,'artifacts','settings-activities.png')});
  // Hiding the Windows flyout is opt-in and reports when it turned itself off
  await native.locator('#nav a[data-view="island"]').click();
  await native.locator('#row-hideSystemFlyout').waitFor({state:'visible'});
  assert.equal(await native.locator('#levelHud').isChecked(),true,'The HUD is on by default');
  assert.equal(await native.locator('#hideSystemFlyout').isChecked(),false,'Off by default');
  await native.locator('#hideSystemFlyout').click();
  await native.locator('#flyoutStatus').filter({hasText:'Turned off'}).waitFor({timeout:2000});
  assert.equal(await native.locator('#hideSystemFlyout').isChecked(),false,'An unavailable flyout turns the setting back off');
  assert.equal(await native.locator('#flyoutStatus').getAttribute('data-kind'),'error');
  assert.deepEqual(nativeErrors,[]);

  // With motion on: what actually keeps running while the island is collapsed
  const motionContext = await browser.newContext({viewport:{width:520,height:230}, reducedMotion:'no-preference'});
  await motionContext.route('http://tuck.test/**', serveDist);
  await motionContext.addInitScript(nativeBridge);
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
  assert.deepEqual(motionErrors,[]);
  console.log('UI smoke passed: geometry, tab rail (keys, wheel edges, swipe, memory), accessibility, keyboard seeking, timers, collapsed media text and its running animations, usage rings, section marks, icon buttons, persistence errors, volume/brightness HUD, wheel and middle-click volume, the pill-sized strip on the open island, queued flyout failure, return to open after HUD, HUD setting, charger-ramp brightness, flyout opt-in, native-bridge call priority, shelf, focused radio updates, drag hold and previews.');
} finally { await browser.close(); }

