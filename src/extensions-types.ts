export interface ExtensionSettings {downloads:boolean;calendar:boolean;notifications:boolean;radios:boolean}
export interface CalendarEvent {id:string;title:string;startsAt:number;endsAt:number;joinUrl:string|null;allDay:boolean}
export interface ShelfItem {id:string;name:string;path:string;bytes:number}
export interface ExtensionSnapshot {settings:ExtensionSettings;downloads:{id:string;name:string;bytes:number;bytesPerSecond:number;totalBytes:null}[];call:{title:string;startedAt:number}|null;calendar:CalendarEvent[];shelf:ShelfItem[];radios:{kind:string;name:string;state:string}[];notificationStatus:string;errors:string[]}
export interface TuckActivityEventDetail {id:string;type:string;title:string;text:string;priority:number;active:boolean}
export function formatBytes(bytes:number):string {if(!Number.isFinite(bytes)||bytes<=0)return '0 B';const unit=Math.min(4,Math.floor(Math.log(bytes)/Math.log(1024)));return `${(bytes/1024**unit).toFixed(unit?1:0)} ${['B','KB','MB','GB','TB'][unit]}`;}
export function formatDuration(ms:number):string {const total=Math.floor(Math.max(0,ms)/1000);return `${Math.floor(total/60).toString().padStart(2,'0')}:${(total%60).toString().padStart(2,'0')}`;}
export function validateHttpUrl(raw:string):string|null {if(/[\x00-\x1f]/.test(raw))return null;try{const url=new URL(raw);return ['http:','https:'].includes(url.protocol)&&!url.username&&!url.password?url.href:null;}catch{return null;}}
