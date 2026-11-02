import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {getCurrentWebviewWindow} from '@tauri-apps/api/webviewWindow';
import {formatBytes,formatDuration} from './extensions-types.js';
import type {ExtensionSnapshot,ExtensionSettings,CalendarEvent,TuckActivityEventDetail} from './extensions-types.js';
import './extensions.css';
let current:ExtensionSnapshot={settings:{downloads:false,calendar:false,notifications:false,radios:false},downloads:[],call:null,calendar:[],shelf:[],radios:[],notificationStatus:'Disabled',errors:[]};
function el<K extends keyof HTMLElementTagNameMap>(tag:K,text='',className=''):HTMLElementTagNameMap[K]{const node=document.createElement(tag);node.textContent=text;node.className=className;return node;}
function button(text:string,action:()=>Promise<unknown>|void,key=text):HTMLButtonElement{const node=el('button',text,'ext-button');node.type='button';node.dataset.actionKey=key;node.addEventListener('click',event=>{event.stopPropagation();void run(action,node);});return node;}
let status:HTMLElement|null=null;
let nativeDragging=false;
let actionError='';
async function run(action:()=>Promise<unknown>|void,button?:HTMLButtonElement):Promise<void>{if(button)button.disabled=true;actionError='';try{if(!isTauri())throw new Error('Open the desktop app to use Windows integrations.');await action();await refresh();}catch(error){actionError=typeof error==='string'?error:error instanceof Error?error.message:'This action failed. Please try again.';if(status)status.textContent=actionError;}finally{if(button)button.disabled=false;}}
async function refresh():Promise<void>{if(isTauri()){current=await invoke<ExtensionSnapshot>('extension_snapshot');render(true);}}
let render: (force?:boolean)=>void=()=>{};
export function mountExtensions():void {
 const container=document.querySelector('#isl .pages-container');if(!container||document.getElementById('extensionsPage'))return;
 const page=el('section','','page extensions');page.id='extensionsPage';page.dataset.page='extensions';page.setAttribute('aria-label','Windows activities');page.inert=true;page.setAttribute('aria-hidden','true');
 const tabs=el('div','','ext-tabs');tabs.setAttribute('aria-label','Windows activity views');const body=el('div','','ext-body');status=el('p','','ext-status');status.setAttribute('role','status');
 let selected='Calls';const buttons:HTMLButtonElement[]=[];
 const show=(name:string)=>{selected=name;for(const node of buttons)node.setAttribute('aria-pressed',String(node.textContent===name));render();};
 for(const name of ['Calls','Files','Calendar','Radios','Downloads']){const tab=el('button',name,'ext-button');tab.type='button';tab.addEventListener('click',event=>{event.stopPropagation();show(name);});buttons.push(tab);tabs.append(tab);}
 page.append(tabs,body,status);container.append(page);
 const activity=(detail:TuckActivityEventDetail)=>window.dispatchEvent(new CustomEvent('tuck-activity',{detail}));
 render=(_force=false)=>{
  activity({id:'native-call',type:'call',title:'Manual call timer',text:current.call?formatDuration(Date.now()-current.call.startedAt):'',priority:95,active:!!current.call});
  activity({id:'native-downloads',type:'download',title:'Downloads',text:`${current.downloads.length} active`,priority:70,active:current.settings.downloads&&current.downloads.length>0});
  // User-triggered refresh updates controls and restores the corresponding focus target.
  const focusKey=body.contains(document.activeElement)?(document.activeElement as HTMLElement).dataset.actionKey:undefined;
  const elapsed=document.getElementById('extCallElapsed');if(elapsed&&current.call)elapsed.textContent=formatDuration(Date.now()-current.call.startedAt);
  if(nativeDragging)return;
  body.replaceChildren();
  if(selected==='Calls'){
   body.append(el('p','Manual call timer. This does not place calls or control your microphone.','ext-note'));
   if(current.call){const elapsed=el('strong',formatDuration(Date.now()-current.call.startedAt),'ext-time');elapsed.id='extCallElapsed';body.append(elapsed);}
   body.append(button(current.call?'End timer':'Start call timer',()=>invoke('call_set_active',{active:!current.call}),'manual-call'));
  }else if(selected==='Files'){
   body.append(el('p','Drop files onto the island to stage them. Originals stay in place.','ext-note'),button('Add files',()=>invoke('shelf_pick')),button('Clear shelf',()=>invoke('shelf_remove',{id:null})));
   for(const item of current.shelf){const row=el('div','','ext-row');row.append(el('span',`${item.name} ? ${formatBytes(item.bytes)}`),button('Reveal',()=>invoke('shelf_reveal',{id:item.id}),`reveal-${item.id}`),button('Remove',()=>invoke('shelf_remove',{id:item.id}),`remove-${item.id}`));
    const drag=el('button','Drag out','ext-button');drag.type='button';drag.dataset.actionKey=`drag-${item.id}`;drag.setAttribute('aria-label',`Drag ${item.name} to another app`);drag.addEventListener('pointerdown',event=>{if(event.button!==0)return;event.preventDefault();nativeDragging=true;window.dispatchEvent(new CustomEvent('tuck-drag-active',{detail:true}));void run(()=>invoke('shelf_drag',{id:item.id})).finally(()=>{nativeDragging=false;window.dispatchEvent(new CustomEvent('tuck-drag-active',{detail:false}));render(true);});});row.append(drag);body.append(row);}
   if(!current.shelf.length)body.append(el('p','Shelf is empty.','ext-note'));
  }else if(selected==='Calendar'){
   if(!current.settings.calendar)body.append(el('p','Enable calendar in Tuck settings and import an ICS file.','ext-note'));
   const upcoming=current.calendar.filter(event=>event.endsAt>=Date.now()).slice(0,20);
   for(const event of upcoming){const row=el('div','','ext-row');row.append(el('span',`${event.title} ? ${new Date(event.startsAt).toLocaleString()}`));if(event.joinUrl)row.append(button('Join',()=>invoke('calendar_join',{id:event.id}),`join-${event.id}`));body.append(row);}
   if(current.settings.calendar&&!upcoming.length)body.append(el('p','No upcoming imported events.','ext-note'));
  }else if(selected==='Radios'){
   body.append(el('p','Radio controls require opt-in and Windows permission. Focus opens Windows Settings.','ext-note'));
   for(const radio of current.radios){body.append(el('span',`${radio.name}: ${radio.state}`),button(radio.state==='On'?'Turn off':'Turn on',()=>invoke('radio_set',{kind:radio.kind,enabled:radio.state!=='On'}),`radio-${radio.kind}-${radio.name}`));}
   if(!current.settings.radios)body.append(el('p','Enable radio controls in Tuck settings.','ext-note'));else if(!current.radios.length)body.append(el('p','No radio state reported. Check Windows permissions and hardware.','ext-note'));
   body.append(button('Focus settings',()=>invoke('quick_action',{name:'focus'})));
  }else{
   if(!current.settings.downloads)body.append(el('p','Enable download monitoring in Tuck settings.','ext-note'));
   for(const item of current.downloads){const row=el('div','','ext-row');row.append(el('span',`${item.name} ? ${formatBytes(item.bytes)} ? ${formatBytes(item.bytesPerSecond)}/s ? total unknown`));const progress=el('progress');progress.setAttribute('aria-label',`${item.name}: total size unknown`);row.append(progress);body.append(row);}
   if(current.settings.downloads&&!current.downloads.length)body.append(el('p','No partial downloads in your Downloads folder.','ext-note'));
  }
  if(status)status.textContent=[...current.errors,actionError].filter(Boolean).join(' ? ');
  if(focusKey)[...body.querySelectorAll('button')].find(node=>node.dataset.actionKey===focusKey)?.focus();
  activity({id:'native-call',type:'call',title:'Manual call timer',text:current.call?formatDuration(Date.now()-current.call.startedAt):'',priority:95,active:!!current.call});
  activity({id:'native-downloads',type:'download',title:'Downloads',text:`${current.downloads.length} active`,priority:70,active:current.settings.downloads&&current.downloads.length>0});
 };
 show('Calls');
 if(isTauri()){
  void listen<ExtensionSnapshot>('extension-state',event=>{current=event.payload;render();}).catch(()=>{if(status)status.textContent='Could not subscribe to Windows state.';});
  void listen<CalendarEvent>('calendar-reminder',event=>{window.dispatchEvent(new CustomEvent('tuck-preview',{detail:{title:event.payload.title,text:'Meeting starts within 5 minutes',priority:90}}));}).catch(()=>{});
  void listen<{title:string;body:string}>('notification-preview',event=>window.dispatchEvent(new CustomEvent('tuck-preview',{detail:{title:event.payload.title,text:event.payload.body,priority:90}}))).catch(()=>{});
  void getCurrentWebviewWindow().onDragDropEvent(event=>{
   if(event.payload.type==='enter'||event.payload.type==='over'){window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:true}));show('Files');}
   if(event.payload.type==='drop'){const paths=event.payload.paths;void run(()=>invoke('shelf_add',{paths})).finally(()=>window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:false})));}
   if(event.payload.type==='leave')window.dispatchEvent(new CustomEvent('tuck-drop-active',{detail:false}));
  }).catch(()=>{if(status)status.textContent='Native file drop is unavailable.';});
  void refresh().catch(()=>{if(status)status.textContent='Windows integrations unavailable.';});
 }
}
export function mountExtensionSettings():void{
 if(document.getElementById('extensionSettings'))return;
 const section=el('section');section.id='extensionSettings';section.append(el('h2','Windows activities'));
 const message=el('p','','ext-settings-status');message.setAttribute('role','status');
 const checks=new Map<keyof ExtensionSettings,HTMLInputElement>();
 for(const [key,label]of [['downloads','Monitor partial downloads in Downloads'],['calendar','Calendar reminders from imported ICS'],['notifications','Preview Windows notifications (permission required)'],['radios','Read and control Wi-Fi and Bluetooth radios']] as const){const row=el('label','','ext-settings-toggle');const input=el('input');input.type='checkbox';input.id=`ext-${key}`;checks.set(key,input);row.append(input,document.createTextNode(label));section.append(row);}
 const save=button('Save activity settings',async()=>{const settings=Object.fromEntries([...checks].map(([key,input])=>[key,input.checked]));await invoke('extension_settings',{settings});message.textContent='Activity settings saved.';});section.append(save);
 const file=el('input');file.type='file';file.accept='.ics,text/calendar';file.id='calendarFile';file.setAttribute('aria-label','Import calendar ICS file');file.addEventListener('change',()=>{void run(async()=>{const selected=file.files?.[0];if(!selected)return;if(selected.size>2*1024*1024)throw new Error('Calendar must be no larger than 2 MB.');const count=await invoke<number>('calendar_import',{text:await selected.text()});message.textContent=`Imported ${count} calendar events.`;});});
 section.append(el('h3','Calendar import'),el('p','Supports individual UTC, local-time and all-day events. Recurrence and named time zones return an explicit error.'),file);
 section.append(el('h3','Notification permission'),el('p','Windows may require package identity and the User Notification Listener capability. Access is requested only when you press the button.'),button('Request notification access',async()=>{message.textContent=await invoke<string>('notification_access');}),message);
 document.body.append(section);status=message;
 if(isTauri())void invoke<ExtensionSnapshot>('extension_snapshot').then(value=>{for(const[key,input]of checks)input.checked=value.settings[key];message.textContent=value.notificationStatus;}).catch(()=>{message.textContent='Could not read activity settings.';});
}
