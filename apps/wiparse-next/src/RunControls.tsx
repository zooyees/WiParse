import {LocalizedLabel,localizeLabel} from './localization';
import {useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {object,string} from './model';

export function RunControls({project,plugin,control,disabled}:{project:string;plugin:string;control:unknown;disabled:boolean}){
  const state=object(control),runId=string(state.run_id),phase=string(state.state);
  const [busy,setBusy]=useState(false),[error,setError]=useState('');
  if(state.supported!==true||!runId)return null;
  const action=async(action:'pause'|'resume')=>{setBusy(true);setError('');try{await invoke('hub_run_control',{project,plugin,runId,action});}catch(reason){setError(`运行状态待核对：${String(reason)}；不会自动重试。`);}finally{setBusy(false);}};
  return <span className="run-controls"><span>{phase==='paused'?localizeLabel("已暂停"):phase==='pause_requested'?localizeLabel("等待安全暂停点"):phase==='resume_requested'?localizeLabel("等待恢复"):localizeLabel("运行中")}</span>{['paused','pause_requested'].includes(phase)?<button type="button" className="button secondary" disabled={busy||disabled} onClick={()=>void action('resume')}><LocalizedLabel text={"继续"}/></button>:<button type="button" className="button secondary" disabled={busy||disabled||phase==='resume_requested'} onClick={()=>void action('pause')}><LocalizedLabel text={"暂停"}/></button>}{error&&<span className="detail-warning" role="alert">{error}</span>}</span>;
}
