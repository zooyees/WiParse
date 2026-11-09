/** Space changes between horizontal and vertical cursors, except while editing text. */
export function cursorAxisOnSpace(key: string, typing: boolean, repeat: boolean): boolean {
  return key === ' ' && !typing && !repeat;
}

/** Five percent margin on each side. Equal cursors do not define a window. */
export function cursorWindow(first: number, second: number): {start: number; end: number} | null {
  if (!Number.isFinite(first) || !Number.isFinite(second)) return null;
  const start = Math.min(first, second);
  const end = Math.max(first, second);
  if (!Number.isFinite(end-start)||Math.abs(end-start)<Number.EPSILON) return null;
  const pad = (end - start) * 0.05;
  const low=start-pad,high=end+pad;
  return Number.isFinite(low)&&Number.isFinite(high)&&high>low?{start:low,end:high}:null;
}

/** Eight percent margin around the samples inside a horizontal cursor window. */
export function verticalMargin(minimum: number, maximum: number): {minimum: number; maximum: number} | null {
  if (!Number.isFinite(minimum) || !Number.isFinite(maximum)) return null;
  const pad = Math.max(Math.abs(maximum - minimum) * 0.08, 1e-12);
  const low=minimum-pad,high=maximum+pad;
  return Number.isFinite(low)&&Number.isFinite(high)&&high>low ? {minimum:low,maximum:high} : null;
}

export function nextCursorPair<T>(current: readonly T[], value: T): T[] {
  if (current.length < 2) return [...current, value];
  return [value];
}

/** Hit-test cursor handles in screen pixels without changing their identities. */
export function nearestCursorIndex(positions:readonly number[],pixel:number,tolerance=8):number|null{
  if(!Number.isFinite(pixel)||!Number.isFinite(tolerance)||tolerance<0)return null;
  let selected:number|null=null,distance=tolerance;
  positions.forEach((position,index)=>{const delta=Math.abs(position-pixel);if(Number.isFinite(position)&&delta<=distance&&(selected===null||delta<distance)){selected=index;distance=delta;}});
  return selected;
}

export function waveBoxZoom(start:number,end:number,extent:{minimum:number;maximum:number},from:{x:number;y:number},to:{x:number;y:number},width:number,height:number){
  if(![start,end,extent.minimum,extent.maximum,from.x,from.y,to.x,to.y,width,height].every(Number.isFinite)||end<=start||extent.maximum<=extent.minimum||width<=0||height<=0)return null;
  if(!Number.isFinite(end-start)||!Number.isFinite(extent.maximum-extent.minimum))return null;
  const clamp=(value:number)=>Math.max(0,Math.min(1,value));
  const a={x:clamp(from.x),y:clamp(from.y)},b={x:clamp(to.x),y:clamp(to.y)};
  const range=Math.abs(a.x-b.x)*width>=6?{start:start+Math.min(a.x,b.x)*(end-start),end:start+Math.max(a.x,b.x)*(end-start)}:null;
  const vertical=Math.abs(a.y-b.y)*height>=6?{minimum:extent.maximum-Math.max(a.y,b.y)*(extent.maximum-extent.minimum),maximum:extent.maximum-Math.min(a.y,b.y)*(extent.maximum-extent.minimum)}:null;
  return range||vertical?{range,extent:vertical}:null;
}

/** Same vertical mapping as the waveform plot: top is the maximum. */
export function pointerY(pointer: number, top: number, height: number, minimum: number, maximum: number): number | null {
  if (!(height > 0) || pointer < top || pointer > top + height) return null;
  const span = maximum - minimum;
  if (!Number.isFinite(span) || span === 0) return null;
  return maximum - ((pointer - top) / height) * span;
}

export function plotExtent(main: {minimum: number; maximum: number}, overlays: {minimum: number; maximum: number}[]): {minimum: number; maximum: number} | null {
  let minimum = Math.min(main.minimum, ...overlays.map(item => item.minimum));
  let maximum = Math.max(main.maximum, ...overlays.map(item => item.maximum));
  if (!Number.isFinite(minimum) || !Number.isFinite(maximum)) return null;
  if (minimum === maximum) {
    const padding = Math.max(Math.abs(minimum) * 0.05, 1e-9);
    minimum -= padding;
    maximum += padding;
  }
  if (!Number.isFinite(maximum - minimum) || maximum === minimum) return null;
  // Keep both plateaus inside the clipping area rather than on its border.
  return main.minimum === main.maximum && overlays.every(item=>item.minimum===main.minimum&&item.maximum===main.maximum)
    ? {minimum, maximum} : verticalMargin(minimum, maximum);
}

/** Bound navigation to the source and at least two sample intervals. */
export function waveWindow(start:number,end:number,fullStart:number,fullEnd:number,points:number):{start:number;end:number}|null{
  if(![start,end,fullStart,fullEnd].every(Number.isFinite)||end<=start||fullEnd<=fullStart)return null;
  const full=fullEnd-fullStart;
  if(!Number.isFinite(full))return null;
  const minimum=Math.min(full,Math.max(full/Math.max(1,points-1)*2,Number.EPSILON*Math.max(Math.abs(fullStart),Math.abs(fullEnd))*8));
  const span=Math.min(full,Math.max(minimum,end-start));
  const left=Math.max(fullStart,Math.min(fullEnd-span,start));
  return {start:left,end:left+span};
}

export function zoomWaveWindow(current:{start:number;end:number},fraction:number,factor:number,source:{start:number;end:number;points:number}):{start:number;end:number}|null{
  if(!Number.isFinite(factor)||factor<=0||!Number.isFinite(fraction))return null;
  const anchor=Math.max(0,Math.min(1,fraction)),width=current.end-current.start;
  if(!(width>0))return null;
  const point=current.start+width*anchor;
  const desired=Math.min(source.end-source.start,width*factor);
  // Clamp the span before anchoring so the pointer stays over the same sample.
  const bounded=waveWindow(source.start,source.start+desired,source.start,source.end,source.points);
  if(!bounded)return null;
  const span=bounded.end-bounded.start;
  return waveWindow(point-span*anchor,point+span*(1-anchor),source.start,source.end,source.points);
}
