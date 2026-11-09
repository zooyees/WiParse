import {useEffect,useRef} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {getCurrentWindow} from '@tauri-apps/api/window';
import {getLanguage} from './language';

/** Close is explicit; pending instrument actions are never silently cancelled. */
export function useCloseGuard(owner:string|null,onError:(message:string)=>void) {
  const current=useRef(owner),prompting=useRef(false);
  useEffect(()=>{current.current=owner;},[owner]);
  useEffect(()=>{
    if(!('__TAURI_INTERNALS__' in window))return;
    let active=true,unlisten:(()=>void)|undefined;
    void getCurrentWindow().onCloseRequested(async event=>{
      if(!current.current)return;
      event.preventDefault();if(prompting.current)return;prompting.current=true;
      try{if(await invoke<boolean>('confirm_unsaved_exit',{language:getLanguage()}))await getCurrentWindow().destroy();}
      catch(reason){onError(`关闭操作未完成：${String(reason)}`);}
      finally{prompting.current=false;}
    }).then(stop=>{if(active)unlisten=stop;else stop();}).catch(reason=>onError(`无法建立未保存内容保护：${String(reason)}`));
    return()=>{active=false;unlisten?.();};
  },[onError]);
}
