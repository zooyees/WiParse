import {LocalizedLabel,localizeLabel} from './localization';
import {useEffect,useState,type Dispatch,type SetStateAction} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {object,string} from './model';

export function HubTransfers({project,plugin,disabled,setOwner,onUpdated}:{project:string;plugin:string;disabled:boolean;setOwner:Dispatch<SetStateAction<string|null>>;onUpdated:()=>void}){
  const [open,setOpen]=useState(false),[target,setTarget]=useState('config'),[strategy,setStrategy]=useState('skip'),[busy,setBusy]=useState(false),[feedback,setFeedback]=useState(''),[error,setError]=useState(false);
  useEffect(()=>{if(busy)setOwner(previous=>previous??'hub-transfer');return()=>setOwner(previous=>previous==='hub-transfer'?null:previous);},[busy,setOwner]);
  const transfer=async(direction:'import'|'export')=>{
    setBusy(true);setFeedback('');setError(false);
    try{
      const reply=await invoke<unknown>('hub_file_transfer',{project,plugin,target,direction,strategy});if(reply===null)return;
      const result=object(object(reply).result);
      setFeedback(direction==='export'?`已导出 ${string(object(reply).filename)}。`:target==='project'?`已导入并切换配置集：${string(result.name)||string(result.id)}。`:`已导入 ${Number(result.imported??0)} 项，跳过 ${Number(result.skipped??0)} 项。`);
      if(direction==='import')onUpdated();
    }catch(reason){setError(true);setFeedback(`操作未完成或结果待核对：${String(reason)}。不会自动重复提交。`);}finally{setBusy(false);}
  };
  return <div className="hub-transfers"><button type="button" className="button secondary" disabled={disabled||busy} onClick={()=>setOpen(value=>!value)}><LocalizedLabel text={"导入 / 导出"}/></button>{open&&<div className="hub-transfer-options"><label><LocalizedLabel text={"范围"}/><select value={target} disabled={busy} onChange={event=>setTarget(event.target.value)}><option value="config"><LocalizedLabel text={"测试项配置"}/></option><option value="project"><LocalizedLabel text={"完整工程配置集"}/></option></select></label>{target==='config'&&<label><LocalizedLabel text={"同名处理"}/><select value={strategy} disabled={busy} onChange={event=>setStrategy(event.target.value)}><option value="skip"><LocalizedLabel text={"保留已有，跳过同名"}/></option><option value="rename"><LocalizedLabel text={"新条目自动重命名"}/></option></select></label>}{target==='project'&&<span className="detail-help"><LocalizedLabel text={"导入会切换配置集；当前配置集由宿主保留。"}/></span>}<div className="assistant-buttons"><button type="button" className="button secondary" disabled={busy} onClick={()=>void transfer('import')}>{target==='project'?localizeLabel("导入并切换"):localizeLabel("导入文件")}</button><button type="button" className="button secondary" disabled={busy} onClick={()=>void transfer('export')}><LocalizedLabel text={"导出到文件"}/></button></div></div>}{feedback&&<div className={error?'detail-warning':'detail-help'} role="status">{feedback}</div>}</div>;
}
