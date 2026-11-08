export interface ExtensionSettings {notifications:boolean}
export interface ShelfItem {id:string;name:string;path:string;bytes:number}
export interface ExtensionSnapshot {settings:ExtensionSettings;shelf:ShelfItem[];notificationStatus:string;errors:string[]}
/** Explorer's thumbnail for a file, made once natively (src-tauri/src/shell_thumbs.rs) */
export type Thumb = {kind:'image';url:string}|{kind:'missing'}|{kind:'none'};

const UNITS=['B','KB','MB','GB','TB'];
/** File sizes: bytes and kilobytes as whole numbers, larger sizes to one decimal (`155 KB`, `17.4 MB`) */
export function formatBytes(bytes:number):string {
 if(!Number.isFinite(bytes)||bytes<=0)return '0 B';
 let unit=Math.min(UNITS.length-1,Math.floor(Math.log(bytes)/Math.log(1024)));
 for(;;){
  const value=bytes/1024**unit;
  const text=unit<2?String(Math.round(value)):value.toFixed(1);
  // Rounding can reach the next unit (1023.6 KB), which reads better as 1.0 MB
  if(Number(text)>=1024&&unit<UNITS.length-1){unit++;continue;}
  return `${text} ${UNITS[unit]}`;
 }
}

/** Which edge arrows a sideways list needs: one only where there is more to scroll (1px tolerance) */
export function scrollEdges(scrollLeft:number,scrollWidth:number,clientWidth:number):{before:boolean;after:boolean} {
 return {before:scrollLeft>1,after:scrollLeft+clientWidth<scrollWidth-1};
}

/** One arrow press moves a page of whole tiles, at least one tile */
export function scrollPage(clientWidth:number,tileStride:number):number {
 if(tileStride<=0)return clientWidth;
 return Math.max(1,Math.floor(clientWidth/tileStride)-1)*tileStride||tileStride;
}

/**
 * One thumbnail request per item for as long as it is shown. Re-renders reuse the
 * answer; nothing is asked again unless the item leaves and comes back.
 */
export class ThumbnailMemo {
 private readonly known=new Map<string,Promise<Thumb>>();
 private readonly settled=new Map<string,Thumb>();
 constructor(private readonly load:(id:string)=>Promise<Thumb>){}
 get(id:string):Promise<Thumb>{
  let answer=this.known.get(id);
  if(!answer){
   answer=this.load(id).catch(()=>({kind:'none'} as Thumb)).then(thumb=>{if(this.known.get(id)===answer)this.settled.set(id,thumb);return thumb;});
   this.known.set(id,answer);
  }
  return answer;
 }
 /** The answer if it has already arrived, so a re-render can paint it straight away */
 peek(id:string):Thumb|undefined{return this.settled.get(id);}
 /** Forgets items that are gone */
 keep(ids:Iterable<string>):void{const live=new Set(ids);for(const id of this.known.keys())if(!live.has(id)){this.known.delete(id);this.settled.delete(id);}}
 get size():number{return this.known.size;}
}
