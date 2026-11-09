import {useWorkspaceActive} from './WorkspaceSlot';
import {LocalizedLabel,localizeLabel} from './localization';
import {useEffect,useMemo,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {readLegacy} from './bridge';
import {object,string} from './model';

// The host owns the bounded recording buffer. The chart only receives its tail;
// exports are produced from a frozen host snapshot, never from chart pixels.
export function AcquisitionPanel({device,disabled}:{device:unknown;disabled:boolean}) {
  const workspaceActive=useWorkspaceActive();
  const item=object(device),id=item.device_id,resource=string(item.resource);
  const [snapshot,setSnapshot]=useState<Record<string,unknown>>({}),[interval,setIntervalDraft]=useState('1000');
  const [busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState(''),[job,setJob]=useState<number|null>(null);
  const operation=useRef(0);
  useEffect(()=>{if(!workspaceActive)return;let active=true,timer:ReturnType<typeof setTimeout>;const poll=async()=>{
    try{const value=object(await invoke('instrument_acquisition',{deviceId:id,resource,action:'state'}));if(active)setSnapshot(value);}
    catch(reason){if(active)setError(String(reason));}
    if(active)timer=setTimeout(poll,1000);
  };void poll();return()=>{active=false;operation.current++;clearTimeout(timer);};},[id,resource,workspaceActive]);
  useEffect(()=>{if(job===null)return;let active=true,timer:ReturnType<typeof setTimeout>;const started=Date.now();const poll=async()=>{
    try{if(Date.now()-started>60000)throw Error('导出状态未确认，请先检查目标文件，不会自动重试');const value=object(await readLegacy('instrument.job.get',{job_id:job}));if(!active)return;
      if(value.state==='pending_or_unknown'){timer=setTimeout(poll,500);return;}
      if(value.state!=='completed'||value.job_id!==job||value.device_id!==id||value.kind!=='samples_export')throw Error('导出任务状态不匹配');
      if(value.ok!==true)throw Error(string(object(value.detail).error,'导出失败'));
      setNotice(`已导出 ${String(object(value.detail).rows)} 条保留记录`);setJob(null);
    }catch(reason){if(active){setError(String(reason));setJob(null);}}
  };void poll();return()=>{active=false;clearTimeout(timer);};},[job,id]);
  const act=async(action:string)=>{
    const value=Number(interval);if((action==='start'||action==='resume')&&(!Number.isInteger(value)||value<100||value>60000)){setError(localizeLabel("采样周期需为 100–60000 ms"));return;}
    const token=++operation.current;setBusy(true);setError('');setNotice('');
    try{const result=object(await invoke('instrument_acquisition',{deviceId:id,resource,action,intervalMs:value}));if(token===operation.current){setSnapshot(result);setNotice(action==='stop'?localizeLabel("已停止调度；已发出的读取会正常收尾"):localizeLabel("采集状态已更新"));}}
    catch(reason){if(token===operation.current)setError(`${String(reason)}；请核对当前采集状态，不会自动重试`);}
    finally{if(token===operation.current)setBusy(false);}
  };
  const exportRows=async()=>{const token=++operation.current;setBusy(true);setError('');setNotice('');try{const raw=await invoke('instrument_export_samples',{deviceId:id,resource});if(token!==operation.current)return;if(raw===null)return;const value=object(raw);if(typeof value.job_id!=='number')throw Error('导出结果未确认');setJob(value.job_id);}catch(reason){if(token===operation.current)setError(String(reason));}finally{if(token===operation.current)setBusy(false);}};
  const rows=Array.isArray(snapshot.rows)?snapshot.rows.map(object):[];
  const series=useMemo(()=>{const groups=new Map<string,number[]>();for(const row of rows){if(typeof row.value!=='number'||!Number.isFinite(row.value))continue;const key=`${string(row.channel)} ${string(row.unit)}`;const values=groups.get(key)??[];values.push(row.value);groups.set(key,values);}return [...groups].slice(0,8).map(([label,values])=>{const low=Math.min(...values),high=Math.max(...values),span=high-low||1;return {label,low,high,points:values.map((value,index)=>`${8+index/Math.max(1,values.length-1)*584},${132-(value-low)/span*120}`).join(' ')};});},[snapshot]);
  const blocked=disabled||busy||item.disconnecting===true,acquiring=snapshot.acquiring===true,paused=snapshot.paused===true;
  return <section className="manual-control-card acquisition-panel"><div className="section-heading"><h3><LocalizedLabel text={"测量、趋势与导出"}/></h3><span className="table-count">{acquiring?(paused?localizeLabel("已暂停"):localizeLabel("采集中")):localizeLabel("已停止")}</span></div>
    <div className="acquisition-toolbar"><label><LocalizedLabel text={"周期 ms"}/><input inputMode="numeric" value={interval} disabled={blocked} onChange={event=>setIntervalDraft(event.target.value)}/></label><button className="button secondary" disabled={blocked||acquiring&&!paused} onClick={()=>void act(acquiring?'resume':'start')}>{paused?localizeLabel("继续"):localizeLabel("开始采集")}</button><button className="button secondary" disabled={blocked||!acquiring||paused} onClick={()=>void act('pause')}><LocalizedLabel text={"暂停"}/></button><button className="button secondary" disabled={blocked||!acquiring} onClick={()=>void act('stop')}><LocalizedLabel text={"停止"}/></button></div>
    <small><LocalizedLabel text={"设备共享采样周期："}/>{String(snapshot.interval_ms??'—')}<LocalizedLabel text={"ms · 保留"}/>{String(snapshot.total??0)}<LocalizedLabel text={"条 · 图表显示最近"}/>{rows.length}<LocalizedLabel text={"条"}/></small>
    <div className="acquisition-trends">{series.length?series.map((line,index)=><figure key={line.label}><figcaption><strong>{line.label}</strong><span>{line.low.toPrecision(5)} – {line.high.toPrecision(5)}</span></figcaption><svg viewBox="0 0 600 144" role="img" aria-label={`${line.label} 趋势，独立纵轴`}><path d="M8 12V132H592" fill="none" stroke="var(--line)"/><polyline points={line.points} fill="none" stroke={['#3487c7','#299871','#b67b24','#8c64c1'][index%4]} strokeWidth="1.6" vectorEffect="non-scaling-stroke"/></svg></figure>):<div className="empty"><span><LocalizedLabel text={"开始采集后显示各通道趋势"}/></span></div>}</div>
    <div className="assistant-buttons"><button className="button secondary" disabled={blocked||job!==null||!Number(snapshot.total)} onClick={()=>void exportRows()}>{job!==null?localizeLabel("正在导出…"):localizeLabel("导出全部保留记录")}</button></div>
    {notice&&<div className="detail-help" role="status">{notice}</div>}{error&&<div className="detail-warning" role="alert">{error}</div>}
  </section>;
}
