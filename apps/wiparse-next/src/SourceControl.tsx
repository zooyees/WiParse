import {LocalizedLabel,localizeLabel} from './localization';
import {useEffect,useRef,useState} from 'react';
import {useDeviceInterlock} from './useDeviceInterlock';
import {readLegacy,sourceAction} from './bridge';
import {object,string} from './model';

export function SourceControl({device,supported,disabled}:{device:unknown;supported:boolean;disabled:boolean}){
  const source=object(device),caps=object(source.capabilities),id=source.device_id,resource=string(source.resource),channels=typeof caps.channels==='number'?Math.min(4,Math.max(0,caps.channels)):0;
  const [drafts,setDrafts]=useState<Record<number,Record<string,string>>>({}),[busy,setBusy]=useState(false),[job,setJob]=useState<number|null>(null),[message,setMessage]=useState('');
  const {uncertain,setUncertain,begin,finish,acknowledge}=useDeviceInterlock('source',resource),operation=useRef<string|null>(null);
  useEffect(()=>{if(!job||typeof id!=='number')return;let active=true,timer:ReturnType<typeof setTimeout>;const start=Date.now();const poll=async()=>{try{if(Date.now()-start>60000)throw Error('命令超时，设备状态未确认');const result=object(await readLegacy('instrument.job.get',{job_id:job}));if(!active)return;if(result.state==='pending_or_unknown'){timer=setTimeout(poll,500);return;}if(result.state!=='completed'||result.job_id!==job||result.device_id!==id)throw Error('设备任务状态不匹配');if(result.ok!==true)setUncertain(true);else if(!operation.current||!finish(operation.current)){setMessage(localizeLabel("命令已完成，但无法清除恢复标记；请核对设备状态。"));setJob(null);return;}setMessage(result.ok===true?localizeLabel("命令完成；输出状态请以设备读数为准。"):`命令失败：${string(object(result.detail).error,'请检查设备')}。请核对设备状态后继续。`);setJob(null);}catch(reason){if(active){setUncertain(true);setMessage(`结果未确认：${String(reason)}`);setJob(null);}}};void poll();return()=>{active=false;clearTimeout(timer);};},[job,id]);
  const send=async(channel:number,action:'voltage'|'current'|'ovp'|'ocp'|'output',value:number|null,enabled:boolean|null)=>{if(typeof id!=='number'||!resource)return;const voltageLike=action==='voltage'||action==='ovp';if(action!=='output'&&(value===null||!Number.isFinite(value)||value<0||value>(voltageLike?60:20))){setMessage(voltageLike?localizeLabel("请输入 0–60 V 的有效电压。"):localizeLabel("请输入 0–20 A 的有效电流。"));return;}setBusy(true);setMessage('');let token:string;try{token=begin();}catch(reason){setUncertain(true);setMessage(`无法确认设备操作状态：${String(reason)}。请核对设备后继续。`);setBusy(false);return;}operation.current=token;try{const result=object(await sourceAction({deviceId:id,resource,channel,action,value,enabled}));if(typeof result.job_id!=='number'||!Number.isSafeInteger(result.job_id))throw Error('命令已提交但任务编号无效');setJob(result.job_id);setMessage(`CH${channel} 命令已提交，等待设备完成。`);}catch(reason){const text=String(reason);const rejected=['INSTRUMENT.TEST_IN_PROGRESS','INSTRUMENT.SESSION_CHANGED','INSTRUMENT.BUSY','INSTRUMENT.INVALID_VALUE','INSTRUMENT.INVALID_CHANNEL','INSTRUMENT.INVALID_REQUEST','INSTRUMENT.UNSUPPORTED'].some(code=>text.includes(code));if(!rejected)setUncertain(true);else if(!finish(token))setUncertain(true);setMessage(text.includes('INSTRUMENT.TEST_IN_PROGRESS')?localizeLabel("测试运行中不能手动控制。"):text.includes('INSTRUMENT.SESSION_CHANGED')?localizeLabel("设备会话已变化，请刷新。"):text.includes('INSTRUMENT.BUSY')?localizeLabel("设备正在处理其他操作。"):text);}finally{setBusy(false);}};
  const blocked=disabled||!supported||busy||!!job||uncertain||source.source_action_pending===true||source.disconnecting===true||channels===0;
  const change=(channel:number,key:string,value:string)=>setDrafts(previous=>({...previous,[channel]:{...previous[channel],[key]:value}}));
  return <div className="source-controls">
    <div className="section-heading"><div><h2><LocalizedLabel text={"电源控制"}/></h2><p><LocalizedLabel text={"各通道独立设置 · 操作按设备顺序执行"}/></p></div></div>
    <div className="source-channel-grid">{Array.from({length:channels},(_,index)=>{
      const channel=index+1,values=drafts[channel]??{};
      const field=(key:string,label:string,action:'voltage'|'current'|'ovp'|'ocp')=><>
        <label>{localizeLabel(label)}<input aria-label={`CH${channel} ${localizeLabel(label)}`} inputMode="decimal" value={values[key]??''} disabled={blocked} onChange={event=>change(channel,key,event.target.value)}/></label>
        <button type="button" className="button secondary" disabled={blocked||!values[key]?.trim()} onClick={()=>void send(channel,action,Number(values[key]),null)}><LocalizedLabel text={"设置"}/></button>
      </>;
      return <section className="source-channel" key={channel} aria-label={`电源通道 CH${channel}`}>
        <h3>CH{channel}</h3>
        <div className="source-control-grid">{field('voltage','电压（V）','voltage')}{field('current','电流上限（A）','current')}</div>
        <div className="source-output-actions">
          <button type="button" className="button secondary" disabled={blocked||caps.source_output!==true} onClick={()=>void send(channel,'output',null,true)}><LocalizedLabel text={"开启输出"}/></button>
          <button type="button" className="button secondary" disabled={blocked||caps.source_output!==true} onClick={()=>void send(channel,'output',null,false)}><LocalizedLabel text={"关闭输出"}/></button>
        </div>
        {caps.source_protection===true&&<div className="source-protection"><h3><LocalizedLabel text={"保护设置"}/></h3><div className="source-control-grid">{field('ovp','过压保护（V）','ovp')}{field('ocp','过流保护（A）','ocp')}</div></div>}
      </section>;
    })}</div>
    {(message||uncertain)&&<div className={uncertain?'detail-warning':'detail-help'} role="status">{message||localizeLabel("上次设备操作的结果未确认，请先核对设备实际状态。")}</div>}
    {uncertain&&<button type="button" className="button secondary" disabled={busy||!!job||source.source_action_pending===true} onClick={()=>{setMessage(acknowledge()?localizeLabel("请以设备实际读数为准。"):localizeLabel("无法清除恢复标记，请检查本机存储。"));}}><LocalizedLabel text={"已核对设备状态"}/></button>}
  </div>;
}
