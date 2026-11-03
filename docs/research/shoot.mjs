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
// idle + peek (hover expand off)
let c = await ctx({hoverExpand:false}); let p = await c.newPage();
await p.goto('http://tuck.test/'); await p.waitForTimeout(600);
await p.screenshot({path:path.join(out,'01-idle.png')});
await p.mouse.move(280,10); await p.waitForTimeout(800);
await p.screenshot({path:path.join(out,'02-peek.png')});
await c.close();
// open pages
c = await ctx({pinned:true}); p = await c.newPage();
await p.goto('http://tuck.test/'); await p.waitForTimeout(600);
await p.locator('#isl').focus(); await p.keyboard.press('Enter'); await p.waitForTimeout(900);
const names = await p.$$eval('.page',e=>e.map(x=>x.dataset.page));
console.log(names.join(','));
for (let i=0;i<names.length;i++){
  await p.locator('#dots button').nth(i).click({force:true}); await p.waitForTimeout(600);
  await p.screenshot({path:path.join(out,`${String(3+i).padStart(2,'0')}-open-${names[i]}.png`)});
}
// media without lyrics
await p.locator('#dots button').nth(0).click({force:true}); await p.waitForTimeout(400);
await p.locator('.lyrics-toggle-btn').click(); await p.waitForTimeout(700);
await p.screenshot({path:path.join(out,'13-open-media-no-lyrics.png')});
// activities running
await p.locator('#dots button').nth(2).click({force:true}); await p.waitForTimeout(400);
await p.locator('#actQuick25').click(); await p.waitForTimeout(700);
await p.screenshot({path:path.join(out,'14-open-activities-running.png')});
await p.keyboard.press('Escape'); await p.waitForTimeout(700);
await p.screenshot({path:path.join(out,'15-idle-with-timer.png')});
await browser.close();
