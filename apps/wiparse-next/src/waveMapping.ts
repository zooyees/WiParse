export type ChannelMapping={scale:number;offset:number};
export type Bounds={start:number;end:number};
export const identityMapping:ChannelMapping={scale:1,offset:0};
export function displayValue(value:number,mapping:ChannelMapping){return value*mapping.scale+mapping.offset;}
export function sourceValue(value:number,mapping:ChannelMapping){return (value-mapping.offset)/mapping.scale;}
export function validMapping(mapping:ChannelMapping){return Number.isFinite(mapping.scale)&&mapping.scale>=1e-12&&mapping.scale<=1e12&&Number.isFinite(mapping.offset);}
export function measurementWindow(scope:'auto'|'full'|'view',full:Bounds,view:Bounds,cursors:{x:number}[]):Bounds{
  if(scope==='view')return view;
  if(scope==='auto'&&cursors.length===2)return {start:Math.min(cursors[0].x,cursors[1].x),end:Math.max(cursors[0].x,cursors[1].x)};
  return full;
}

export function stackMappings(channels:{index:number;minimum:number;maximum:number}[]):Record<number,ChannelMapping>|null{
 const result:Record<number,ChannelMapping>={};
 for(const [slot,channel] of channels.entries()){
  const span=channel.maximum-channel.minimum,center=channel.minimum+span/2;
  if(!Number.isFinite(span)||span<0||!Number.isFinite(center))return null;
  const scale=span>0?Math.max(1e-12,Math.min(1e12,2/span)):1;
  const mapping={scale,offset:-slot*3-center*scale};if(!validMapping(mapping))return null;result[channel.index]=mapping;
 }return result;
}

export function overlayCompatible(source:{x_unit:string},selected?:{x_unit:string}){return !!selected&&source.x_unit===selected.x_unit;}

/** Label the shared Y axis in the selected source's original units when representable. */
export function axisReadoutExtent(extent:{minimum:number;maximum:number},mapping:ChannelMapping){
 const minimum=sourceValue(extent.minimum,mapping),maximum=sourceValue(extent.maximum,mapping);
 return validMapping(mapping)&&Number.isFinite(minimum)&&Number.isFinite(maximum)&&Number.isFinite(maximum-minimum)&&maximum>minimum?{minimum,maximum,original:true}:{...extent,original:false};
}
