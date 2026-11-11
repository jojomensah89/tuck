import {chromium} from '@playwright/test';
import {readFile,mkdir} from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
const phase=process.argv[2]||'after';
const browser=await chromium.launch({channel:'msedge',headless:true});
try {const page=await browser.newPage({viewport:{width:1000,height:740},reducedMotion:'reduce'});
await page.route('http://tuck.test/**',async route=>{const p=new URL(route.request().url()).pathname;try{await route.fulfill({body:await readFile(path.join('dist',p)),contentType:p.endsWith('.html')?'text/html':p.endsWith('.js')?'application/javascript':p.endsWith('.css')?'text/css':'image/svg+xml'});}catch{await route.fulfill({status:404});}});
await page.addInitScript(() => {
 if(location.hash !== '#welcome') return;
 window.__TAURI_INTERNALS__ = { transformCallback: () => 1, async invoke(command) {
 if(command==='installed_usage_providers') return [{provider:'claude',installed:true,enabled:false},{provider:'antigravity',installed:true,enabled:false}];
 if(command==='dictation_get_state') return {status:'idle',transcript:'',modelInstalled:false,settings:{toggleShortcut:'Ctrl+Shift+Space',holdToTalkShortcut:'Left+RightShift',reviewBeforeInsert:false},models:[{id:'base',label:'Base',bytes:147951465,speed:'Very fast',accuracy:'Good',state:'found',foundIn:'another app',active:false},{id:'tiny',label:'Tiny',bytes:77691713,speed:'Fastest',accuracy:'Lower',state:'available',active:false}]};
 if(command==='get_autostart') return false;
 if(command==='extension_snapshot') return {settings:{notifications:false},shelf:[],notificationStatus:'Disabled',errors:[]};
 if(command==='clipboard_get_state') return {enabled:false,persistOnExit:false,items:[]};
 if(command==='get_hide_system_flyout') return {enabled:false,state:'disabled'};
 return null;
 }};
});
await mkdir(`docs/screens/settings-${phase}`,{recursive:true});
for(const theme of ['light','dark']){await page.emulateMedia({colorScheme:theme});for(const view of ['general','island','media','activities','tools','usage','privacy','about',...(phase==='after'?['welcome']:[])]){await page.goto(`http://tuck.test/settings.html#${view}`);await page.reload();await page.waitForTimeout(200);
const contrast = await page.evaluate(() => {
 const style=getComputedStyle(document.documentElement);
 const luminance=token=>{let hex=style.getPropertyValue(token).trim().slice(1);if(hex.length===3) hex=hex.split('').map(c=>c+c).join('');return [0,2,4].map(i=>parseInt(hex.slice(i,i+2),16)/255).map(v=>v<=0.04045?v/12.92:((v+0.055)/1.055)**2.4).reduce((sum,v,i)=>sum+v*[0.2126,0.7152,0.0722][i],0);};
 return ['--ink','--ink-2','--ink-3'].flatMap(ink=>['--ground','--panel','--control','--control-hover'].map(bg=>{const a=luminance(ink),b=luminance(bg);return {ink,bg,ratio:(Math.max(a,b)+0.05)/(Math.min(a,b)+0.05)};}));
});
if(phase==='after') for(const result of contrast) assert.ok(result.ratio>=4.5,`${theme} ${result.ink} on ${result.bg}: ${result.ratio}`);
if(view==='privacy') { assert.equal(await page.locator('#sensorIndicator').isChecked(),true); assert.equal(await page.locator('#micShortcut').innerText(),'Ctrl+Alt+M'); await page.screenshot({path:`docs/screens/mic-camera-settings-${theme}.png`,fullPage:true});
 await page.locator('#sensorIndicator').click(); assert.equal(await page.locator('#sensorIndicator').isChecked(),false);
 await page.locator('#micShortcutOff').click(); await page.waitForFunction(()=>document.getElementById('micShortcut').textContent==='Off');
 await page.locator('#micShortcut').click(); await page.keyboard.press('Escape'); assert.equal(await page.locator('#micShortcut').innerText(),'Off');
 await page.locator('#micShortcut').click(); await page.keyboard.press('Control+Alt+N'); await page.waitForFunction(()=>document.getElementById('micShortcut').textContent==='Ctrl+Alt+N');
 }
await page.screenshot({path:`docs/screens/settings-${phase}/${view}-${theme}.png`,fullPage:true});
if(view==='welcome') {
 assert.equal(await page.locator('#welcome #usage-claude').isChecked(),false);
 await page.locator('#welcome #usage-antigravity').click();
 await page.getByRole('dialog').waitFor(); await page.keyboard.press('Escape');
 assert.equal(await page.getByRole('dialog').count(),0);
 assert.equal(await page.locator('#usage-antigravity').isChecked(),false);
 await page.getByRole('button',{name:'Done',exact:true}).click();
 assert.equal(await page.locator('#view-general #autostart').count(),1);
 await page.locator('#settingsSearch').fill('Antigravity');
 assert.equal(await page.locator('#row-usage-antigravity').isVisible(),true,'Search indexes controls restored from welcome');
 await page.locator('#settingsSearch').fill('');
 await page.goto('http://tuck.test/settings.html#about');
 await page.getByRole('button',{name:'Show welcome',exact:true}).click();
 await page.getByRole('button',{name:'Skip',exact:true}).click();
 assert.equal(await page.locator('#view-tools #modelRows').count(),1);
}
}}
if(phase==='after') for(const width of [900,640]) {
 await page.setViewportSize({width,height:740});
 for(const theme of ['light','dark']) { await page.emulateMedia({colorScheme:theme});
 for(const view of ['general','island','media','activities','tools','usage','privacy','about']) {
 await page.goto(`http://tuck.test/settings.html#${view}`); await page.reload();
 assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth <= innerWidth),true,`${view} ${theme} ${width}: no horizontal overflow`);
 }}
}
}finally{await browser.close();}
