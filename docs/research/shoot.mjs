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
await browser.close();
