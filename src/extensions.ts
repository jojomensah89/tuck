import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {getCurrentWebviewWindow} from '@tauri-apps/api/webviewWindow';
import {formatBytes} from './extensions-types.js';
import type {ExtensionSnapshot,ExtensionSettings} from './extensions-types.js';
import './extensions.css';
import {SVG_ICONS} from './icons.js';
import {markSaved} from './settings-ui.js';
let current:ExtensionSnapshot={settings:{notifications:false},shelf:[],notificationStatus:'Disabled',errors:[]};
function el<K extends keyof HTMLElementTagNameMap>(tag:K,text='',className=''):HTMLElementTagNameMap[K]{const node=document.createElement(tag);node.textContent=text;node.className=className;return node;}
function button(text:string,action:()=>Promise<unknown>|void,key=text):HTMLButtonElement{const node=el('button',text,'ext-button');node.type='button';node.dataset.actionKey=key;node.addEventListener('click',event=>{event.stopPropagation();void run(action,node);});return node;}
let status:HTMLElement|null=null;
let nativeDragging=false;
let actionError='';
async function run(action:()=>Promise<unknown>|void,button?:HTMLButtonElement):Promise<void>{if(button)button.disabled=true;actionError='';try{if(!isTauri())throw new Error('Open the desktop app to use Windows integrations');await action();await refresh();}catch(error){actionError=typeof error==='string'?error:error instanceof Error?error.message:'This action failed, try again';if(status)status.textContent=actionError;}finally{if(button)button.disabled=false;}}
async function refresh():Promise<void>{if(isTauri()){current=await invoke<ExtensionSnapshot>('extension_snapshot');render(true);}}
let render: (force?:boolean)=>void=()=>{};
/** An icon button from the line family; its name is the tooltip and the accessible name */
function iconButton(label:string,icon:string,action:()=>Promise<unknown>|void,key=label):HTMLButtonElement{const node=button('',action,key);node.className='icon-btn';node.innerHTML=icon;node.title=label;node.setAttribute('aria-label',label);return node;}
export function mountExtensions():void {
 const container=document.querySelector('#isl .pages-container');if(!container||document.getElementById('extensionsPage'))return;
 // File shelf in the Tools section.
 const page=el('section','','page extensions');page.id='extensionsPage';page.dataset.page='extensions';page.setAttribute('aria-label','Files');
 const head=el('div','','ext-head');const actions=el('div','','ext-actions');head.append(actions);
 const body=el('div','','ext-body');status=el('p','','ext-status');status.setAttribute('role','status');
 page.append(head,body,status);container.append(page);
 render=(_force=false)=>{
  // User-triggered refresh updates controls and restores the corresponding focus target.
  const focusOwner=[body,actions].find(node=>node.contains(document.activeElement));
  const focusKey=focusOwner?(document.activeElement as HTMLElement).dataset.actionKey:undefined;

  if(nativeDragging)return;
  body.replaceChildren();actions.replaceChildren();

   actions.append(iconButton('Add files',SVG_ICONS.plus,()=>invoke('shelf_pick'),'shelf-add'));
   const clear=iconButton('Clear shelf',SVG_ICONS.trash,()=>invoke('shelf_remove',{id:null}),'shelf-clear');clear.disabled=!current.shelf.length;actions.append(clear);
   const drop=button('',()=>invoke('shelf_pick'),'shelf-drop');drop.className='shelf-drop';drop.innerHTML=SVG_ICONS.folder+'<span>Drop files here</span>';drop.setAttribute('aria-label','Drop files here');body.append(drop);
   body.classList.toggle('empty',!current.shelf.length);
   for(const item of current.shelf){const row=el('div','','ext-row');
    const drag=el('button','','icon-btn grip');drag.type='button';drag.innerHTML=SVG_ICONS.grip;drag.title=`Drag ${item.name} to another app`;drag.dataset.noSwipe='';drag.dataset.actionKey=`drag-${item.id}`;drag.setAttribute('aria-label',`Drag ${item.name} to another app`);drag.addEventListener('pointerdown',event=>{if(event.button!==0)return;event.preventDefault();nativeDragging=true;window.dispatchEvent(new CustomEvent('tuck-drag-active',{detail:true}));void run(()=>invoke('shelf_drag',{id:item.id})).finally(()=>{nativeDragging=false;window.dispatchEvent(new CustomEvent('tuck-drag-active',{detail:false}));render(true);});});
    const text=el('span','','ext-row-text');text.append(el('span',item.name,'ext-row-name'),el('span',formatBytes(item.bytes),'ext-row-meta'));
    row.append(drag,text,iconButton(`Show ${item.name} in folder`,SVG_ICONS.folder,()=>invoke('shelf_reveal',{id:item.id}),`reveal-${item.id}`),iconButton(`Remove ${item.name}`,SVG_ICONS.close,()=>invoke('shelf_remove',{id:item.id}),`remove-${item.id}`));body.append(row);}


  if(status)status.textContent=[...current.errors,actionError].filter(Boolean).join(' | ');
  if(focusKey&&focusOwner)[...page.querySelectorAll('button')].find(node=>node.dataset.actionKey===focusKey)?.focus();
 };
 window.addEventListener('tuck-drop-active',event=>page.classList.toggle('drag-over',(event as CustomEvent).detail===true));
 render();
 if(isTauri()){
  void listen<ExtensionSnapshot>('extension-state',event=>{current=event.payload;render();}).catch(()=>{if(status)status.textContent='Could not subscribe to Windows state';});
  void listen<{title:string;body:string}>('notification-preview',event=>window.dispatchEvent(new CustomEvent('tuck-preview',{detail:{title:event.payload.title,text:event.payload.body,priority:90}}))).catch(()=>{});
  void getCurrentWebviewWindow().onDragDropEvent(event=>{
   if(event.payload.type==='enter'||event.payload.type==='over'){window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:true}));render();}
   if(event.payload.type==='drop'){const paths=event.payload.paths;void run(()=>invoke('shelf_add',{paths})).finally(()=>window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:false})));}
   if(event.payload.type==='leave')window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:false}));
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
