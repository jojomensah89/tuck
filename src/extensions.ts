import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {getCurrentWebviewWindow} from '@tauri-apps/api/webviewWindow';
import {formatBytes,scrollEdges,scrollPage,ThumbnailMemo} from './extensions-types.js';
import type {ExtensionSnapshot,ExtensionSettings,ShelfItem,Thumb} from './extensions-types.js';
import './extensions.css';
import {SVG_ICONS} from './icons.js';
import {markSaved} from './settings-ui.js';
let current:ExtensionSnapshot={settings:{notifications:false},shelf:[],notificationStatus:'Disabled',errors:[]};
function el<K extends keyof HTMLElementTagNameMap>(tag:K,text='',className=''):HTMLElementTagNameMap[K]{const node=document.createElement(tag);node.textContent=text;node.className=className;return node;}
function button(text:string,action:()=>Promise<unknown>|void,key=text):HTMLButtonElement{const node=el('button',text);node.type='button';node.dataset.actionKey=key;node.addEventListener('click',event=>{event.stopPropagation();void run(action,node);});return node;}
let status:HTMLElement|null=null;
let nativeDragging=false;
let actionError='';
async function run(action:()=>Promise<unknown>|void,button?:HTMLButtonElement):Promise<void>{if(button)button.disabled=true;actionError='';try{if(!isTauri())throw new Error('Open the desktop app to use Windows integrations');await action();await refresh();}catch(error){actionError=typeof error==='string'?error:error instanceof Error?error.message:'This action failed, try again';if(status)status.textContent=actionError;}finally{if(button)button.disabled=false;}}
async function refresh():Promise<void>{if(isTauri()){current=await invoke<ExtensionSnapshot>('extension_snapshot');render();}}
let render:()=>void=()=>{};
/** An icon button from the line family; its name is the tooltip and the accessible name */
function iconButton(label:string,icon:string,action:()=>Promise<unknown>|void,key=label):HTMLButtonElement{const node=button('',action,key);node.className='icon-btn';node.innerHTML=icon;node.title=label;node.setAttribute('aria-label',label);return node;}
const EASE='cubic-bezier(0.23, 1, 0.32, 1)';
/** Movement before a press on a tile becomes a drag out instead of a click */
const DRAG_START=4;
const keyboardInUse=()=>document.documentElement.classList.contains('keyboard-input');
const reducedMotion=()=>matchMedia('(prefers-reduced-motion: reduce)').matches;
export function mountExtensions():void {
 const container=document.querySelector('#isl .pages-container');if(!container||document.getElementById('extensionsPage'))return;
 // File shelf in the Tools section.
 const page=el('section','','page extensions');page.id='extensionsPage';page.dataset.page='extensions';page.setAttribute('aria-label','Files');
 // The tab rail moves the actions into the segmented row, beside Screen snip
 const actions=el('div');actions.dataset.sectionActions='extensions';
 const add=iconButton('Add files',SVG_ICONS.plus,()=>invoke('shelf_pick'),'shelf-add');
 const clear=button('Clear',()=>invoke('shelf_remove',{id:null}),'shelf-clear');clear.className='text-btn';clear.title='Remove all files from the shelf';clear.setAttribute('aria-label','Clear shelf');
 actions.append(add,clear);
 // The one drop target: it fills an empty shelf, and covers the tiles while a file is dragged over
 const drop=button('',()=>invoke('shelf_pick'),'shelf-drop');drop.className='shelf-drop';drop.innerHTML=SVG_ICONS.tray;
 const dropText=el('span','Drop files here','drop-text');drop.append(dropText,el('span','Drag out anytime · click to open','drop-hint'));
 drop.title='Pick files';drop.setAttribute('aria-label','Drop files here, or select to pick files');drop.dataset.noSwipe='';
 // Files sit in one sideways row of equal tiles
 const shelf=el('div','','shelf');
 const list=el('div','','shelf-tiles');list.setAttribute('role','list');list.setAttribute('aria-label','Files on the shelf');list.dataset.horizontalScroll='';list.dataset.noSwipe='';
 const stride=()=>{const tile=list.querySelector<HTMLElement>('.shelf-tile');return tile?tile.getBoundingClientRect().width+(parseFloat(getComputedStyle(list).columnGap)||0):list.clientWidth;};
 const arrow=(label:string,icon:string,direction:1|-1)=>{
  const node=el('button','',`shelf-scroll ${direction<0?'prev':'next'}`);node.type='button';node.innerHTML=icon;node.title=label;node.setAttribute('aria-label',label);
  // The tiles are in the keyboard order already; the arrows are for the pointer
  node.tabIndex=-1;node.hidden=true;node.dataset.noSwipe='';
  node.addEventListener('click',event=>{event.stopPropagation();list.scrollBy({left:direction*scrollPage(list.clientWidth,stride()),behavior:(reducedMotion()?'instant':'smooth') as ScrollBehavior});});
  return node;
 };
 const prev=arrow('Scroll left',SVG_ICONS.chevronLeft,-1),next=arrow('Scroll right',SVG_ICONS.chevronRight,1);
 shelf.append(list,prev,next);
 const syncArrows=()=>{const edges=scrollEdges(list.scrollLeft,list.scrollWidth,list.clientWidth);prev.hidden=!edges.before;next.hidden=!edges.after;};
 let arrowFrame=0;
 list.addEventListener('scroll',()=>{if(!arrowFrame)arrowFrame=requestAnimationFrame(()=>{arrowFrame=0;syncArrows();});},{passive:true});
 // Width changes (the open width setting, the island opening) are the only other time the edges move
 new ResizeObserver(syncArrows).observe(list);
 status=el('p','','ext-status');status.setAttribute('role','status');
 page.append(actions,drop,shelf,status);container.append(page);

 const thumbs=new ThumbnailMemo(id=>invoke<Thumb|null>('shelf_thumbnail',{id}).then(thumb=>thumb??{kind:'none'}));
 const tiles=new Map<string,HTMLElement>();
 let painted=false;
 let pendingRender=false;
 let suppressClick=false;
 function paintThumb(tile:HTMLElement,item:ShelfItem,thumb:Thumb):void{
  const preview=tile.querySelector<HTMLElement>('.tile-preview')!;
  if(thumb.kind==='missing'){
   tile.classList.add('missing');preview.innerHTML=SVG_ICONS.fileMissing;
   tile.querySelector('.tile-size')!.textContent='File missing';
   tile.querySelector('.tile-open')!.setAttribute('aria-label',`${item.name}, file missing`);
   return;
  }
  if(thumb.kind!=='image'||!thumb.url.startsWith('data:image/png;base64,'))return;
  const image=el('img');image.alt='';image.draggable=false;image.decoding='async';
  image.addEventListener('load',()=>{preview.replaceChildren(image);tile.classList.add('has-thumb');requestAnimationFrame(()=>image.classList.add('loaded'));},{once:true});
  image.src=thumb.url;
 }
 function startDrag(item:ShelfItem):void{
  nativeDragging=true;window.dispatchEvent(new CustomEvent('tuck-drag-active',{detail:true}));
  void run(()=>invoke('shelf_drag',{id:item.id})).finally(()=>{nativeDragging=false;window.dispatchEvent(new CustomEvent('tuck-drag-active',{detail:false}));render();});
 }
 function tileAction(label:string,icon:string,action:()=>Promise<unknown>,className:string,key:string):HTMLButtonElement{
  const node=button('',action,key);node.className=`tile-action ${className}`;node.title=label;node.setAttribute('aria-label',label);node.dataset.noSwipe='';
  const dot=el('span','','tile-action-dot');dot.innerHTML=icon;node.append(dot);return node;
 }
 function makeTile(item:ShelfItem):HTMLElement{
  const tile=el('div','','shelf-tile');tile.setAttribute('role','listitem');tile.dataset.id=item.id;
  const open=el('button','','tile-open');open.type='button';open.dataset.actionKey=`open-${item.id}`;open.dataset.noSwipe='';
  const size=formatBytes(item.bytes);
  open.title=item.name;open.setAttribute('aria-label',`Open ${item.name}, ${size}`);
  const preview=el('span','','tile-preview');preview.innerHTML=SVG_ICONS.file;
  open.append(preview,el('span',item.name,'tile-name'),el('span',size,'tile-size'));
  open.addEventListener('click',event=>{event.stopPropagation();if(suppressClick){suppressClick=false;return;}void run(()=>invoke('shelf_open',{id:item.id}));});
  // A press that moves becomes the native drag out, so a still click opens the file
  let press:{x:number;y:number;pointer:number}|null=null;
  open.addEventListener('pointerdown',event=>{suppressClick=false;press=event.button===0&&event.isPrimary?{x:event.clientX,y:event.clientY,pointer:event.pointerId}:null;});
  open.addEventListener('pointermove',event=>{
   if(!press||event.pointerId!==press.pointer||nativeDragging)return;
   if(Math.hypot(event.clientX-press.x,event.clientY-press.y)<DRAG_START)return;
   press=null;suppressClick=true;startDrag(item);
  });
  const release=()=>{press=null;};open.addEventListener('pointerup',release);open.addEventListener('pointercancel',release);
  tile.append(open,
   tileAction(`Show ${item.name} in folder`,SVG_ICONS.folder,()=>invoke('shelf_reveal',{id:item.id}),'tile-reveal',`reveal-${item.id}`),
   tileAction(`Remove ${item.name}`,SVG_ICONS.close,()=>invoke('shelf_remove',{id:item.id}),'tile-remove',`remove-${item.id}`));
  void thumbs.get(item.id).then(thumb=>{if(tiles.get(item.id)===tile)paintThumb(tile,item,thumb);});
  return tile;
 }
 render=()=>{
  if(nativeDragging)return;
  // While a file is over the island the drop target covers the tiles; they change once it is gone
  if(page.classList.contains('drag-over')){pendingRender=true;return;}
  pendingRender=false;
  const items=current.shelf;
  page.classList.toggle('has-files',items.length>0);
  add.hidden=!items.length;clear.hidden=!items.length;
  const animate=painted&&!document.hidden&&!keyboardInUse()&&!reducedMotion();
  const before=new Map([...tiles].map(([id,tile])=>[id,tile.getBoundingClientRect()]));
  const listBox=list.getBoundingClientRect();
  const ids=new Set(items.map(item=>item.id));
  // A tile that leaves under the keyboard hands focus to its neighbour, so the keyboard stays in the island
  let refocus:HTMLElement|null=null;
  const order=[...tiles.keys()];
  for(const [id,tile] of tiles){
   if(ids.has(id))continue;
   if(tile.contains(document.activeElement)&&keyboardInUse()){
    const at=order.indexOf(id);
    const neighbour=order.slice(at+1).concat(order.slice(0,at).reverse()).find(other=>ids.has(other));
    refocus=neighbour?tiles.get(neighbour)!.querySelector<HTMLElement>('.tile-open'):drop;
   }
   tiles.delete(id);
   if(animate){
    const rect=before.get(id)!;
    Object.assign(tile.style,{position:'absolute',left:`${rect.left-listBox.left+list.scrollLeft}px`,top:`${rect.top-listBox.top}px`,width:`${rect.width}px`,pointerEvents:'none'});
    tile.inert=true;tile.classList.add('leaving');
    tile.animate([{opacity:1},{opacity:0}],{duration:100,easing:EASE}).finished.then(()=>tile.remove()).catch(()=>tile.remove());
   } else tile.remove();
  }
  const fresh:HTMLElement[]=[];
  // The shelf only appends and removes, so tiles already in place never move in the DOM (and keep focus)
  for(const item of items){
   if(tiles.has(item.id))continue;
   const tile=makeTile(item);tiles.set(item.id,tile);fresh.push(tile);list.append(tile);
  }
  if(animate){
   // The gap closes by sliding the remaining tiles (transform only), like the clipboard cards
   for(const [id,tile] of tiles){
    const old=before.get(id);if(!old)continue;
    const dx=old.left-tile.getBoundingClientRect().left;
    if(Math.abs(dx)>0.5)tile.animate([{transform:`translateX(${dx}px)`},{transform:'none'}],{duration:200,easing:EASE});
   }
  }
  if(painted&&fresh.length){
   for(const tile of fresh){
    if(!reducedMotion())tile.animate([{opacity:0,transform:'translateY(4px) scale(0.96)'},{opacity:1,transform:'none'}],{duration:200,easing:EASE});
    // A short highlight says which tile just arrived, then fades
    const flash=el('span','','tile-flash');flash.setAttribute('aria-hidden','true');tile.querySelector('.tile-preview')!.after(flash);
    flash.animate([{opacity:1},{opacity:0}],{duration:280,delay:700,easing:EASE,fill:'both'}).finished.then(()=>flash.remove()).catch(()=>flash.remove());
   }
   const last=fresh[fresh.length-1];
   if(last.offsetLeft+last.offsetWidth>list.scrollLeft+list.clientWidth||last.offsetLeft<list.scrollLeft)
    list.scrollTo({left:Math.max(0,last.offsetLeft+last.offsetWidth-list.clientWidth),behavior:(reducedMotion()?'instant':'smooth') as ScrollBehavior});
  }
  thumbs.keep(ids);
  painted=true;
  syncArrows();
  if(status)status.textContent=[...current.errors,actionError].filter(Boolean).join(' | ');
  if(refocus)refocus.focus();
  else if(!items.length&&actions.contains(document.activeElement)&&keyboardInUse())drop.focus();
 };
 window.addEventListener('tuck-drop-active',event=>{
  const over=(event as CustomEvent).detail===true;
  page.classList.toggle('drag-over',over);dropText.textContent=over?'Release to add':'Drop files here';
  if(!over&&pendingRender)render();
  else if(!over)syncArrows();
 });
 render();
 if(isTauri()){
  void listen<ExtensionSnapshot>('extension-state',event=>{current=event.payload;render();}).catch(()=>{if(status)status.textContent='Could not subscribe to Windows state';});
  void listen<{title:string;body:string}>('notification-preview',event=>window.dispatchEvent(new CustomEvent('tuck-preview',{detail:{title:event.payload.title,text:event.payload.body,priority:90}}))).catch(()=>{});
  // Over events fire constantly during a drag; the island only needs to hear when it starts and ends
  let dropActive=false;
  const setDropActive=(active:boolean)=>{if(active===dropActive)return;dropActive=active;window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:active}));};
  void getCurrentWebviewWindow().onDragDropEvent(event=>{
   if(event.payload.type==='enter'||event.payload.type==='over')setDropActive(true);
   if(event.payload.type==='drop'){window.dispatchEvent(new Event('tuck-drop-complete'));const paths=event.payload.paths;void run(()=>invoke('shelf_add',{paths})).finally(()=>setDropActive(false));}
   if(event.payload.type==='leave')setDropActive(false);
  }).catch(()=>{if(status)status.textContent='Native file drop is unavailable';});
  void refresh().catch(()=>{if(status)status.textContent='Windows integrations unavailable';});
 }
}
/** Binds the Activities rows in settings.html. The switch saves its flag and reverts on failure. */
export function mountExtensionSettings():void{
 const message=document.getElementById('extStatus');if(!message)return;
 status=message;
 const setMessage=(text:string,error=false)=>{message.textContent=text;if(error&&text)message.dataset.kind='error';else delete message.dataset.kind;};
 const checks=new Map<keyof ExtensionSettings,HTMLInputElement>();
 for(const key of ['notifications'] as const){const input=document.getElementById(`ext-${key}`) as HTMLInputElement|null;if(input)checks.set(key,input);}
 const all=()=>Object.fromEntries([...checks].map(([key,input])=>[key,input.checked])) as unknown as ExtensionSettings;
 for(const input of checks.values())input.addEventListener('change',async()=>{
  const row=input.closest('.set-row');checks.forEach(check=>{check.disabled=true;});
  try{if(!isTauri())throw new Error('Open the desktop app to use Windows integrations.');await invoke('extension_settings',{settings:all()});setMessage('');markSaved(input);}
  catch(error){input.checked=!input.checked;row?.classList.remove('saved');setMessage(typeof error==='string'?error:error instanceof Error?error.message:'Could not save activity settings.',true);}
  finally{checks.forEach(check=>{check.disabled=false;});}
 });
 const access=document.getElementById('requestNotificationAccess') as HTMLButtonElement|null;
 access?.addEventListener('click',()=>{void run(async()=>{setMessage(await invoke<string>('notification_access'));},access);});
 if(isTauri())void invoke<ExtensionSnapshot>('extension_snapshot').then(value=>{for(const[key,input]of checks)input.checked=value.settings[key];window.dispatchEvent(new Event('settings-loaded'));setMessage(value.notificationStatus==='Disabled'?'':value.notificationStatus);}).catch(()=>setMessage('Could not read activity settings.',true));
 else{checks.forEach(check=>{check.disabled=true;});if(access)access.disabled=true;setMessage('Windows activities need the desktop app.');}
}
