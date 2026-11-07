export interface ExtensionSettings {notifications:boolean}
export interface ShelfItem {id:string;name:string;path:string;bytes:number}
export interface ExtensionSnapshot {settings:ExtensionSettings;shelf:ShelfItem[];notificationStatus:string;errors:string[]}
export function formatBytes(bytes:number):string {if(!Number.isFinite(bytes)||bytes<=0)return '0 B';const unit=Math.min(4,Math.floor(Math.log(bytes)/Math.log(1024)));return `${(bytes/1024**unit).toFixed(unit?1:0)} ${['B','KB','MB','GB','TB'][unit]}`;}
