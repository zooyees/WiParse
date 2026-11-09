import {cursorWindow,waveWindow} from './waveCursor.ts';
export type ProtocolFrame={index:number;t_start:number;t_end:number;summary:string;error:string;bytes:number;preview:string;hex?:string};
export function filterFrames(frames:ProtocolFrame[],query:string,issues:boolean){const text=query.trim().toLocaleLowerCase();if(!text&&!issues)return frames;return frames.filter(frame=>(!issues||frame.error!=='none')&&(!text||`${frame.index+1} ${frame.summary} ${frame.hex??frame.preview} ${frame.error}`.toLocaleLowerCase().includes(text)));}
export function stepFrame(frames:ProtocolFrame[],selected:number|null,delta:-1|1):number|null {if(!frames.length)return null;const at=frames.findIndex(frame=>frame.index===selected);return frames[at<0?(delta<0?frames.length-1:0):Math.max(0,Math.min(frames.length-1,at+delta))].index;}
export function zoomVertical(extent:{minimum:number;maximum:number},fraction:number,factor:number){if(![extent.minimum,extent.maximum,fraction,factor].every(Number.isFinite)||factor<=0||extent.maximum<=extent.minimum)return null;const span=extent.maximum-extent.minimum,anchor=Math.max(0,Math.min(1,fraction));const width=span*factor;const point=extent.maximum-span*anchor;const minimum=point-width*(1-anchor),maximum=point+width*anchor;return Number.isFinite(minimum)&&Number.isFinite(maximum)&&maximum>minimum&&width>Number.EPSILON*Math.max(1,Math.abs(point))*4?{minimum,maximum}:null;}
export function cursorFrequency(first:number,second:number,unit:string){const seconds=({s:1,ms:1e-3,us:1e-6,'µs':1e-6,ns:1e-9} as Record<string,number>)[unit];const delta=Math.abs(second-first);return seconds&&delta>0&&Number.isFinite(delta)&&Number.isFinite(1/(delta*seconds))?1/(delta*seconds):null;}
type Channel={index:number;x_unit:string};type Source={channels:Channel[];files:{id:string;channels:number[]}[]};
export function channelKeys(source:Source|null){return new Map<number,string>(source?.files.flatMap(file=>file.channels.map((index,ordinal)=>[index,`${file.id}:${ordinal}`] as const))??[]);}
export function reconcileChannels(before:Source|null,after:Source,primary:number,overlays:number[]){const old=channelKeys(before),fresh=channelKeys(after),byKey=new Map([...fresh].map(([index,key])=>[key,index]));const selected=byKey.get(old.get(primary)??'')??after.channels[0]?.index??0;const main=after.channels.find(channel=>channel.index===selected);const priorKeys=new Set(old.values());const retained=[primary,...overlays].flatMap(index=>{const next=byKey.get(old.get(index)??'');return next===undefined?[]:[next];});const added=[...fresh].filter(([,key])=>!priorKeys.has(key)).map(([index])=>index);return {primary:selected,overlays:[...new Set([...retained,...added])].filter(index=>index!==selected&&after.channels.find(channel=>channel.index===index)?.x_unit===main?.x_unit)};}

export function visibleExtent(channels:{index:number;start:number;end:number;points:number}[],visible:number[]){const shown=channels.filter(channel=>visible.includes(channel.index));if(!shown.length)return null;const start=Math.min(...shown.map(channel=>channel.start)),end=Math.max(...shown.map(channel=>channel.end));const intervals=shown.map(channel=>(channel.end-channel.start)/Math.max(1,channel.points-1)).filter(step=>step>0&&Number.isFinite(step));const step=intervals.length?Math.min(...intervals):end-start;return {start,end,points:Math.max(2,Math.min(Number.MAX_SAFE_INTEGER,step>0?Math.ceil((end-start)/step)+1:2))};}

/** Frame-sized context rather than a percentage of a potentially hours-long record. */
export function frameFocusWindow(frame:{t_start:number;t_end:number},source:{start:number;end:number;points:number}){
 if(![frame.t_start,frame.t_end,source.start,source.end,source.points].every(Number.isFinite)||frame.t_end<frame.t_start)return null;
 const resolution=(source.end-source.start)/Math.max(1,source.points-1),pad=Math.max(frame.t_end-frame.t_start,resolution,Number.EPSILON*Math.max(1,Math.abs(frame.t_start))*8)*6;
 return waveWindow(frame.t_start-pad,frame.t_end+pad,source.start,source.end,source.points);
}

/** Bound each overlay request to the native viewport limit while sharing the aggregate budget. */
export function overlayBuckets(count:number){return Math.max(32,Math.min(2048,Math.floor(4200/Math.max(1,count))));}

/** Decode gates are exact evidence boundaries, never the padded display window.
 * DDSSS measurement cursors must not interrupt correlator acquisition; explicit
 * viewport gating remains available when the operator intentionally crops it. */
export function protocolGate(kind:string,enabled:boolean,mode:'cursors'|'view',cursors:{x:number}[],view:{start:number;end:number}|null){
 if(!enabled)return null;
 if(mode==='view')return view;
 if(kind==='ddsss'||cursors.length!==2)return null;
 const a=cursors[0].x,b=cursors[1].x;
 return Number.isFinite(a)&&Number.isFinite(b)?{start:Math.min(a,b),end:Math.max(a,b)}:null;
}
/** Display-only padding is clamped to the visible document and sample resolution. */
export function cursorFocusWindow(cursors:{x:number}[],source:{start:number;end:number;points:number}){
 if(cursors.length!==2)return null;
 const padded=cursorWindow(cursors[0].x,cursors[1].x);
 return padded?waveWindow(padded.start,padded.end,source.start,source.end,source.points):null;
}
