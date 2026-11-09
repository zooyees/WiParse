import type {View} from './WavePlot';
import {overlayBuckets} from './waveAnalysis.ts';
export type ViewportRequest={channel:number;overlays:number[];start:number;end:number};
export type ViewportRead={channel:number;start:number;end:number;buckets:number};
export type ViewportResult={view:View;overlays:View[];failures:{channel:number;reason:string}[]};
/** Read a complete viewport as one publishable batch, with at most three native
 * reads in flight. Every worker settles before the batch returns, even when the
 * primary fails; the next queued viewport cannot overlap abandoned workers. */
export async function readWaveViewport(request:ViewportRequest,read:(input:ViewportRead)=>Promise<View>,active:()=>boolean=()=>true):Promise<ViewportResult>{
 const overlays=[...new Set(request.overlays)].filter(channel=>channel!==request.channel);
 const channels=[request.channel,...overlays],results:PromiseSettledResult<View>[]=new Array(channels.length);
 let next=0,primaryFailed=false;
 const worker=async()=>{while(active()&&!primaryFailed&&next<channels.length){const slot=next++,channel=channels[slot];try{
  const value=await read({channel,start:request.start,end:request.end,buckets:slot===0?1500:overlayBuckets(overlays.length)});
  results[slot]={status:'fulfilled',value:{...value,channelIndex:channel}};
 }catch(reason){results[slot]={status:'rejected',reason};if(slot===0)primaryFailed=true;}}};
 await Promise.all(Array.from({length:Math.min(3,channels.length)},worker));
 if(!active())throw Error('Viewport source changed');
 const main=results[0];if(main.status==='rejected')throw main.reason;
 const available:View[]=[],failures:{channel:number;reason:string}[]=[];
 for(let slot=1;slot<results.length;slot++){const result=results[slot];if(result.status==='fulfilled')available.push(result.value);else failures.push({channel:channels[slot],reason:String(result.reason)});}
 return {view:main.value,overlays:available,failures};
}
