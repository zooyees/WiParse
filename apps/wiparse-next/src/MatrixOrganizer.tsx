import {useState,type Dispatch,type SetStateAction} from 'react';
import {invoke} from '@tauri-apps/api/core';
import type {MatrixRow} from './model';
import {LocalizedLabel} from './localization';

export function MatrixOrganizer({project,plugin,rows,visible,selection,setSelection,disabled,setOwner,onUpdated}:{project:string;plugin:string;rows:MatrixRow[];visible:MatrixRow[];selection:string[];setSelection:Dispatch<SetStateAction<string[]>>;disabled:boolean;setOwner:Dispatch<SetStateAction<string|null>>;onUpdated:()=>void}){
  const [tags,setTags]=useState(''),[busy,setBusy]=useState(false),[error,setError]=useState('');
  const apply=async(enabled:boolean|null)=>{
    if(!selection.length)return;
    const chosen=rows.filter(row=>selection.includes(row.id));
    if(chosen.length!==selection.length||chosen.some(row=>row.revision===null||row.sha256.length!==64)){setError('所选配置版本不完整，请刷新核对。');return;}
    const expected=Object.fromEntries(chosen.map(row=>[row.id,{expected_revision:row.revision,expected_sha256:row.sha256}]));
    setBusy(true);setOwner('matrix-batch');setError('');
    try{await invoke('hub_config_batch',{project,plugin,ids:selection,expected,enabled,tags:enabled===null?tags.split(/[,，]/).map(value=>value.trim()).filter(Boolean):null});onUpdated();}
    catch(reason){setError(`${String(reason)}；请刷新核对，不会自动重复提交。`);}
    finally{setBusy(false);setOwner(previous=>previous==='matrix-batch'?null:previous);}
  };
  return <div className="matrix-organizer"><span><LocalizedLabel text="运行勾选"/> {selection.length} / {rows.length}</span><button className="button secondary" disabled={disabled||busy} onClick={()=>setSelection(previous=>[...new Set([...previous,...visible.map(row=>row.id)])])}><LocalizedLabel text="勾选筛选结果"/></button><button className="button secondary" disabled={disabled||busy} onClick={()=>setSelection(previous=>[...previous.filter(id=>!visible.some(row=>row.id===id)),...visible.filter(row=>!previous.includes(row.id)).map(row=>row.id)])}><LocalizedLabel text="反选筛选结果"/></button><button className="button secondary" disabled={disabled||busy||!selection.length} onClick={()=>setSelection([])}><LocalizedLabel text="清空勾选"/></button><details><summary><LocalizedLabel text="批量操作"/></summary><div className="assistant-buttons"><button className="button secondary" disabled={disabled||busy||!selection.length} onClick={()=>void apply(true)}><LocalizedLabel text="启用所选"/></button><button className="button secondary" disabled={disabled||busy||!selection.length} onClick={()=>void apply(false)}><LocalizedLabel text="禁用所选"/></button><input aria-label="批量标签" placeholder="标签，逗号分隔" value={tags} disabled={disabled||busy} onChange={event=>setTags(event.target.value)}/><button className="button secondary" disabled={disabled||busy||!selection.length} onClick={()=>void apply(null)}><LocalizedLabel text="应用标签"/></button></div></details>{error&&<span className="detail-warning" role="alert">{error}</span>}</div>;
}
