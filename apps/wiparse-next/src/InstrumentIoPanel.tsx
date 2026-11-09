import {useEffect,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {readLegacy} from './bridge';
import {object,string} from './model';
import {useWorkspaceActive} from './WorkspaceSlot';

export function InstrumentIoPanel({device}:{device:unknown}){
  const active=useWorkspaceActive(),item=object(device),deviceId=item.device_id,resource=string(item.resource);
  const [open,setOpen]=useState(false),[follow,setFollow]=useState(true),[snapshot,setSnapshot]=useState<Record<string,unknown>>({}),[error,setError]=useState(''),[busy,setBusy]=useState(false);
  useEffect(()=>{if(!active||!open||!follow)return;let current=true,timer:ReturnType<typeof setTimeout>;const poll=async()=>{try{const reply=object(await readLegacy('ui.instrument.io_log.get_safe',{device_id:deviceId,resource}));if(current){setSnapshot(reply);setError('');}}catch(reason){if(current)setError(String(reason));}if(current)timer=setTimeout(poll,1000);};void poll();return()=>{current=false;clearTimeout(timer);};},[active,open,follow,deviceId,resource]);
  const clear=async()=>{if(!window.confirm('清空当前设备保留的通信记录？不会发送设备命令，也不会删除测试证据。'))return;setBusy(true);try{setSnapshot(object(await invoke('instrument_log_clear',{deviceId,resource,revision:string(snapshot.revision)})));setError('');}catch(reason){setError(`${String(reason)}；请刷新核对，不会自动重试。`);}finally{setBusy(false);}};
  return <details className="execution-log" open={open} onToggle={event=>setOpen(event.currentTarget.open)}><summary>设备通信记录</summary><div className="assistant-buttons"><button className="button secondary" onClick={()=>setFollow(value=>!value)}>{follow?'暂停刷新':'恢复刷新'}</button><button className="button secondary" disabled={busy||!string(snapshot.revision)} onClick={()=>void clear()}>清空记录</button><small>当前设备的保留窗口 · {resource}</small></div>{error&&<p className="detail-warning" role="alert">{error}</p>}<pre onWheel={event=>{if(event.deltaY<0)setFollow(false);}}>{string(snapshot.text)||'尚无通信记录。'}</pre></details>;
}
