import {useEffect,useRef} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {readLegacy} from './bridge';
import {navigationRequest,type WorkbenchPage} from './navigation';

/** A platform page request is acknowledged only after the React commit. */
export function useRemoteNavigation(owner:string|null,page:WorkbenchPage,activate:(page:WorkbenchPage)=>void,onBlocked:(message:string)=>void) {
  const currentOwner=useRef(owner),currentPage=useRef(page),pending=useRef<{owner:number;revision:number;page:WorkbenchPage}|null>(null),last=useRef('');
  useEffect(()=>{currentOwner.current=owner;currentPage.current=page;},[owner,page]);
  useEffect(()=>{
    if(!('__TAURI_INTERNALS__' in window))return;
    let active=true,timer:ReturnType<typeof setTimeout>;
    const poll=async()=>{
      try{
        const request=navigationRequest(await readLegacy('ui.navigation.get_safe'));
        if(!active)return;
        if(request){const key=`${request.owner}:${request.revision}`;if(key!==last.current){last.current=key;
          // Allow queued input/owner effects to settle before external navigation.
          await new Promise(resolve=>setTimeout(resolve,0));if(!active)return;
          if(currentOwner.current){onBlocked('平台请求切换页面，当前有未保存的编辑或操作，已阻止本次切换。请处理后重新请求。');await invoke('navigation_ack_safe',{owner:request.owner,revision:request.revision,state:'blocked'});}
          else if(currentPage.current===request.page){await invoke('navigation_ack_safe',{owner:request.owner,revision:request.revision,state:'applied'});}
          else{pending.current=request;activate(request.page);}
        }}
      }catch{/* Host bootstrap/recovery is reported by the workspace connection path. */}
      if(active)timer=setTimeout(poll,1000);
    };
    void poll();return()=>{active=false;clearTimeout(timer);};
  },[activate,onBlocked]);
  // Changed pages are acknowledged from the post-commit effect. The already
  // visible page is handled above without requiring a no-op React render.
  useEffect(()=>{const request=pending.current;if(!request||request.page!==page)return;pending.current=null;void invoke('navigation_ack_safe',{owner:request.owner,revision:request.revision,state:owner?'blocked':'applied'}).catch(()=>{});},[page,owner]);
}
