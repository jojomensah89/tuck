import { chromium } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
const root = process.cwd(); const out = path.join(root,'docs','research','screens');
let browser;
try { browser = await chromium.launch({channel:'msedge',headless:true}); } catch { browser = await chromium.launch({headless:true}); }
const types={'.html':'text/html','.js':'application/javascript','.css':'text/css','.svg':'image/svg+xml'};
async function ctx(ls){
  const c = await browser.newContext({viewport:{width:560,height:260},deviceScaleFactor:2});
  if(ls) await c.addInitScript(v=>localStorage.setItem('tuck.live.v2',v), JSON.stringify(ls));
  await c.route('http://tuck.test/**', async r=>{const p=new URL(r.request().url()).pathname;const f=path.join(root,'dist',p==='/'?'index.html':p);
    try{await r.fulfill({body:await readFile(f),contentType:types[path.extname(f)]||'application/octet-stream'});}catch{await r.fulfill({status:404,body:'nf'});}});
  return c;
}
async function mockNative(c){
  await c.addInitScript(()=>{
    window.isTauri=true;
    let serial=0;const callbacks=new Map();const listeners=new Map();
    window.__emitNative=(event,payload)=>{for(const id of listeners.get(event)||[])callbacks.get(id)?.({event,id,payload});};
    window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},transformCallback(callback){const id=++serial;callbacks.set(id,callback);return id;},async invoke(command,args={}){
      if(command==='plugin:event|listen'){const ids=listeners.get(args.event)||[];ids.push(args.handler);listeners.set(args.event,ids);return ++serial;}
      if(command==='media_now')return {active:false,trackId:'',title:'',artist:'',playing:false,positionMs:0,durationMs:0,artwork:null};
      if(command==='usage_now')return [];
      if(command==='fullscreen_now')return false;
      if(command==='extension_snapshot')return {settings:{downloads:true,calendar:true,notifications:false,radios:true},downloads:[{id:'d1',name:'ubuntu-26.04-desktop-amd64.iso',bytes:1610612736,bytesPerSecond:13421772,totalBytes:null}],call:null,calendar:[{id:'e1',title:'Design review',startsAt:Date.now()+40*60000,endsAt:Date.now()+70*60000,joinUrl:'https://example.com/meet',allDay:false},{id:'e2',title:'Release notes',startsAt:Date.now()+26*3600000,endsAt:Date.now()+27*3600000,joinUrl:null,allDay:false}],shelf:[{id:'f1',name:'island-redesign.fig',path:'C:/x',bytes:4823449},{id:'f2',name:'usage-rings-reference.png',path:'C:/y',bytes:182044}],radios:[{kind:'wifi',name:'Wi-Fi',state:'On'},{kind:'bluetooth',name:'Bluetooth',state:'Off'}],notificationStatus:'Disabled',errors:[]};
      if(command==='clipboard_get_state')return {enabled:false,persistOnExit:false,items:[]};
      if(command==='dictation_get_state')return {status:'idle',modelInstalled:false,modelDownloadedBytes:0,transcript:null,message:null,settings:{},canInsert:false};
      return null;
    }};
  });
}
// True idle, then active-media peek (hover expand off)
let c = await ctx({hoverExpand:false}); let p = await c.newPage();
await mockNative(c);
await p.goto('http://tuck.test/'); await p.waitForTimeout(200);
await p.evaluate(()=>{document.body.classList.add('dev-mode');window.__emitNative('power',{percent:82,charging:true,hasBattery:true,minutesLeft:null,statusKnown:true,percentKnown:true,acConnected:true});});
await p.waitForTimeout(200);
await p.screenshot({path:path.join(out,'01-idle.png')});
await p.evaluate(()=>{window.__emitNative('media',{active:true,trackId:'fixture-track',title:'How I Use Grok Bot Every Single Day',artist:'Ras Mic',playing:true,positionMs:401000,durationMs:1128000,artwork:null});window.__emitNative('hover',true);});
await p.waitForTimeout(800);
await p.screenshot({path:path.join(out,'02-peek.png')});
await c.close();
// open tabs: every section of every tab, in rail order
c = await ctx({pinned:true}); p = await c.newPage();
await p.goto('http://tuck.test/'); await p.waitForTimeout(600);
await p.locator('#isl').focus(); await p.keyboard.press('Enter'); await p.waitForTimeout(900);
const tabs = await p.$$eval('[role=tab]',e=>e.map(x=>x.dataset.tab));
const showSection = async (tab, section) => {
  await p.locator(`#tab-${tab}`).click({force:true}); await p.waitForTimeout(400);
  await p.locator(`#panel-${tab} > .page[data-page="${section}"]`).evaluate(el=>{el.closest('.tab-panel').scrollTop=el.offsetTop;});
  await p.waitForTimeout(400);
};
let shot = 3;
for (const tab of tabs){
  const sections = await p.$$eval(`#panel-${tab} > .page`,e=>e.map(x=>x.dataset.page));
  console.log(tab, sections.join(','));
  for (const section of sections){
    await showSection(tab, section);
    await p.screenshot({path:path.join(out,`${String(shot++).padStart(2,'0')}-open-${tab}-${section}.png`)});
  }
}
// rail keyboard focus on the settings button
await p.locator('#railSettings').focus(); await p.keyboard.press('Shift+Tab'); await p.keyboard.press('Tab'); await p.waitForTimeout(300);
await p.screenshot({path:path.join(out,`${String(shot++).padStart(2,'0')}-open-rail-focus.png`)});
// media without lyrics
await showSection('media','media');
await p.locator('.lyrics-toggle-btn').click(); await p.waitForTimeout(700);
await p.screenshot({path:path.join(out,`${String(shot++).padStart(2,'0')}-open-media-no-lyrics.png`)});
// timer running
await showSection('today','activities');
await p.locator('#actQuick25').click(); await p.waitForTimeout(700);
await p.screenshot({path:path.join(out,`${String(shot++).padStart(2,'0')}-open-today-activities-running.png`)});
await p.keyboard.press('Escape'); await p.waitForTimeout(700);
await p.screenshot({path:path.join(out,`${String(shot++).padStart(2,'0')}-idle-with-timer.png`)});
await c.close();

// Populated sections through the mocked native bridge (synthetic fixtures), at 500 and 440 wide
for (const width of [500, 440]) {
  c = await ctx({pinned:true, openWidth:width}); await mockNative(c);
  await c.addInitScript(()=>{
    localStorage.setItem('notch.chatgpt', JSON.stringify({label:'ChatGPT (estimate)', cap:40, hours:3}));
    const now = Math.round(Date.now()/1000);
    localStorage.setItem('notch.chatgpt.log', JSON.stringify(Array.from({length:31},(_,i)=>now-3000+i*60)));
  });
  p = await c.newPage();
  await p.goto('http://tuck.test/'); await p.waitForTimeout(300);
  const now = Date.now()/1000;
  await p.evaluate(now=>{
    document.body.classList.add('dev-mode');
    window.__emitNative('power',{percent:14,charging:false,hasBattery:true,minutesLeft:47,statusKnown:true,percentKnown:true,acConnected:false});
    window.__emitNative('media',{active:true,trackId:'fixture-track',title:'How I Use Grok Bot Every Single Day',artist:'Ras Mic',playing:true,positionMs:401000,durationMs:1128000,artwork:null});
    window.__emitNative('usage',[
      {id:'claude-5h',provider:'claude',label:'Claude session (5 h)',usedPct:93,spentUsd:null,resetsAt:now+2400,source:'official',stale:false},
      {id:'claude-7d',provider:'claude',label:'Claude weekly',usedPct:74,spentUsd:null,resetsAt:now+86400*2+3600*5,source:'official',stale:false},
      {id:'codex-installed-codex-primary',provider:'codex',label:'Codex 5 h',usedPct:22,spentUsd:null,resetsAt:now+3600*3,source:'official',stale:false},
      {id:'codex-installed-codex-secondary',provider:'codex',label:'Codex weekly',usedPct:9,spentUsd:null,resetsAt:now+86400*5,source:'official',stale:true},
      {id:'anthropic-api',provider:'anthropic-api',label:'Anthropic API',usedPct:null,spentUsd:12.4,resetsAt:null,source:'api',stale:false},
      {id:'openai-api',provider:'openai-api',label:'OpenAI API',usedPct:null,spentUsd:null,resetsAt:null,source:'api',stale:false,status:'unavailable',error:'No admin key'},
    ]);
  }, now);
  await p.locator('#isl').focus(); await p.keyboard.press('Enter'); await p.waitForTimeout(900);
  await p.evaluate(()=>{
    window.__emitNative('telemetry',{sampledAt:Date.now(),cpuPct:23,ramUsedBytes:11.6*1073741824,ramTotalBytes:16*1073741824,gpus:[{id:'gpu0',name:'Fixture GPU',usedPct:81}]});
    window.__emitNative('clipboard_history_changed',{enabled:true,items:[
      {id:'a',text:'https://github.com/jojomensah89/tuck/pull/14',pinned:true,createdAt:Date.now()},
      {id:'b',text:'bun run test:ui',pinned:false,createdAt:Date.now()},
      {id:'c',text:'The island is a black shape that merges into the display edge',pinned:false,createdAt:Date.now()}]});
    window.__emitNative('dictation_state',{status:'recording',transcript:'Remind me to rename the usage tab',modelReady:true});
  });
  await p.waitForTimeout(600);
  const shotW = async name => p.screenshot({path:path.join(out,`${String(shot++).padStart(2,'0')}-w${width}-${name}.png`)});
  await showSection('media','media'); await shotW('media-lyrics');
  await showSection('today','today'); await shotW('today-low-battery');
  await showSection('today','activities'); await p.locator('#actQuick5').click(); await p.waitForTimeout(1300); await p.locator('#actPause').click(); await p.waitForTimeout(500); await shotW('timer-paused');
  await showSection('tools','clipboard'); await shotW('clipboard');
  await showSection('tools','dictation'); await shotW('dictation-recording');
  await showSection('tools','extensions'); await shotW('files');
  for (const view of ['Calls','Radios','Downloads']) { await p.getByRole('button',{name:view,exact:true}).click(); await p.waitForTimeout(200); await shotW(view.toLowerCase()); }
  await showSection('today','calendar'); await shotW('calendar');
  await showSection('usage','usage'); await p.waitForTimeout(500); await shotW('usage-dense');
  await showSection('usage','stats');
  await p.evaluate(()=>{const t=Date.now();[31,28,40,36,22,25,19,27,33,23].forEach((cpu,i)=>window.__emitNative('telemetry',{sampledAt:t-1000+i*100,cpuPct:cpu,ramUsedBytes:(11.2+i*0.05)*1073741824,ramTotalBytes:16*1073741824,gpus:[{id:'gpu0',name:'Fixture GPU',usedPct:70+i}]}));});
  await p.waitForTimeout(500); await shotW('stats');
  await c.close();
}
// Volume and brightness HUD from mocked native level events (reduced motion freezes the bar fill)
c = await ctx({hoverExpand:false}); await mockNative(c);
p = await c.newPage(); await p.emulateMedia({reducedMotion:'reduce'});
await p.goto('http://tuck.test/'); await p.waitForTimeout(200);
await p.evaluate(()=>document.body.classList.add('dev-mode'));
for (const [name, payload] of [['volume',{kind:'volume',level:62,muted:false}],['volume-low',{kind:'volume',level:18,muted:false}],['volume-muted',{kind:'volume',level:62,muted:true}],['brightness',{kind:'brightness',level:80,muted:false}]]) {
  await p.evaluate(payload=>window.__emitNative('system-level',payload), payload); await p.waitForTimeout(450);
  await p.screenshot({path:path.join(out,`${String(shot++).padStart(2,'0')}-hud-${name}.png`)});
}
await c.close();
await browser.close();
