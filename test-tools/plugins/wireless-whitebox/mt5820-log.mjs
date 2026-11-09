// Observations only: exported terminal receive timestamps are not device time.
// Process each receive block independently; incomplete split tokens are not
// guessed or joined because export line endings do not prove UART boundaries.
export function inspectTerminalExport(text) {
  if (typeof text !== 'string') throw new TypeError('text required');
  const header = /^\[(\d{2}:\d{2}:\d{2}\.\d{3})\]收←◆/gm;
  const blocks = [...text.matchAll(header)], events = [];
  for (let i=0;i<blocks.length;i++) {
    const match=blocks[i], start=match.index+match[0].length, end=blocks[i+1]?.index??text.length;
    const body=text.slice(start,end), time=match[1];
    const add=(kind, raw, offset, fields={})=>events.push({kind, raw, offset:start+offset, receive_time:time, time_source:'terminal_receive_block', fields});
    for (const m of body.matchAll(/\bFW\s+([^\r\n]+)/g)) add('firmware',m[0],m.index,{description:m[1].trim()});
    for (const m of body.matchAll(/\bStart CHG\b/g)) add('charge_start',m[0],m.index);
    for (const m of body.matchAll(/\bFO Alert!/g)) add('fo_alert',m[0],m.index);
    for (const m of body.matchAll(/\bReset Vbrg:\s*(-?\d+)\s+Vbrg_act:\s*(-?\d+)/g)) add('bridge_reset',m[0],m.index,{requested_raw:Number(m[1]),actual_raw:Number(m[2]),unit:'unknown'});
    for (const m of body.matchAll(/\bPloss=(-?\d+)\b/g)) add('ploss_observed',m[0],m.index,{raw_value:Number(m[1]),unit:'unknown'});
    for (const m of body.matchAll(/\bFodCnt\s*=\s*(\d+)\b/g)) add('fod_count_observed',m[0],m.index,{raw_value:Number(m[1]),unit:'count'});
    for (const m of body.matchAll(/\bEPT_REASON:(\d+)\b/g)) add('ept_reason_observed',m[0],m.index,{raw_value:Number(m[1]),meaning:'unconfirmed'});
  }
  events.sort((a,b)=>a.offset-b.offset);
  return {format:'mt5820-terminal-export-observations/1',blocks:blocks.length,events,complete:false,product_verdict:'INCONCLUSIVE', limitations:['终端接收时间不是设备事件时间','分包可能截断字段','字段单位及原因码待固件接口确认']};
}

// Candidate sequence is for investigation only; a terminal export cannot prove
// causation, exact timing, electrical shutdown, or that no events were lost.
export function findAlertRecoveryCandidates(events) {
  return events.flatMap((alert,index)=>{
    if(alert.kind!=='fo_alert')return [];
    const nextAlert=events.findIndex((e,i)=>i>index&&e.kind==='fo_alert');
    const after=events.slice(index+1,nextAlert<0?events.length:nextAlert);
    const before=events.slice(0,index).reverse();
    return [{alert, prior_count:before.find(e=>e.kind==='fod_count_observed')??null,
      reset:after.find(e=>e.kind==='bridge_reset')??null,
      reason:after.find(e=>e.kind==='ept_reason_observed')??null,
      next_charge:after.find(e=>e.kind==='charge_start')??null,
      verdict:'INCONCLUSIVE'}];
  });
}
