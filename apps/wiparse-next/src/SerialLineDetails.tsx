import {LocalizedLabel} from './localization';
import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {object,string} from './model';

export type SelectedLogLine={row:number;text:string};
export function SerialLineDetails({line,onClose}:{line:SelectedLogLine;onClose:()=>void}) {
  const area=useRef<HTMLElement>(null),[font,setFont]=useState(13);
  useEffect(()=>{const element=area.current;if(!element)return;const zoom=(event:WheelEvent)=>{if(!event.ctrlKey)return;event.preventDefault();event.stopPropagation();setFont(value=>Math.max(10,Math.min(28,value+(event.deltaY<0?1:-1))));};element.addEventListener('wheel',zoom,{passive:false});return()=>element.removeEventListener('wheel',zoom);},[]);
  const [result,setResult]=useState<Record<string,unknown>|null>(null),[error,setError]=useState('');
  useEffect(()=>{let active=true;setResult(null);setError('');void invoke('serial_explain_line',{text:line.text}).then(value=>{if(active)setResult(object(value));}).catch(reason=>{if(active)setError(String(reason));});return()=>{active=false;};},[line.text]);
  const details=Array.isArray(result?.details)?result.details.map(object):[];
  return <section ref={area} className="serial-line-details" style={{fontSize:font}}><div className="section-heading"><strong><LocalizedLabel text={"第"}/>{line.row+1}<LocalizedLabel text={"行 · 协议说明快照"}/></strong><button type="button" className="button secondary" onClick={onClose}><LocalizedLabel text={"收起"}/></button></div><code>{line.text}</code>{error?<div className="detail-warning">{error}</div>:!result?<small><LocalizedLabel text={"正在解析…"}/></small>:details.length?<div className="serial-line-fields">{details.map((field,index)=><div className={`qi-field qi-${string(field.role)}`} key={index}>{string(field.text)}</div>)}</div>:<small><LocalizedLabel text={"该行没有识别到 Qi 报文；原文保持不变。"}/></small>}</section>;
}
