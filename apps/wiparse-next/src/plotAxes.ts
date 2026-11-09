export type AxisScale = {values:number[];labels:string[];annotation:string};
/** Offset/scientific annotations keep ticks readable without discarding raw values. */
export function axisScale(minimum:number,maximum:number):AxisScale {
  if(!Number.isFinite(minimum)||!Number.isFinite(maximum)||maximum<minimum)throw Error('坐标范围无效');
  const span=maximum-minimum;
  const offset=span>0&&Number.isFinite(span)&&Math.max(Math.abs(minimum),Math.abs(maximum))/span>100_000?minimum:0;
  const values=Array.from({length:5},(_,i)=>i===0?minimum:i===4?maximum:minimum*(1-i/4)+maximum*(i/4));
  const relative=values.map(value=>value-offset),magnitude=Math.max(...relative.map(Math.abs));
  const exponent=magnitude>0&&(magnitude>=1000||magnitude<0.001)?Math.max(-321,Math.min(306,Math.floor(Math.log10(magnitude)/3)*3)):0;
  const factor=10**exponent,scaled=relative.map(value=>value/factor);
  const step=Math.abs(scaled[4]-scaled[0])/4;
  const scaledMagnitude=Math.max(...scaled.map(Math.abs));
  const decimals=step>0?Math.max(0,Math.min(10,Math.max(Math.ceil(-Math.log10(step))+1,5-Math.floor(Math.log10(scaledMagnitude))))):0;
  const labels=scaled.map(value=>{const rounded=Number(value.toFixed(decimals));return Object.is(rounded,-0)?'0':String(rounded);});
  const annotation=[exponent?`×10^${exponent}`:'',offset?`offset ${offset}`:''].filter(Boolean).join(' · ');
  return {values,labels,annotation};
}
export const PLOT_INSETS={left:82,right:20,top:28,bottom:44} as const;

/** Keep nearby absolute coordinates distinguishable when their common origin is large. */
export function coordinateReadout(value:number,span:number):string {
  if(!Number.isFinite(value))return '—';
  const width=Math.abs(span),magnitude=Math.abs(value);
  const precision=width>0&&Number.isFinite(width)?Math.max(7,Math.min(17,Math.ceil(Math.log10(Math.max(magnitude,Number.MIN_VALUE))-Math.log10(width))+3)):17;
  return value.toLocaleString('zh-CN',{maximumSignificantDigits:precision});
}
