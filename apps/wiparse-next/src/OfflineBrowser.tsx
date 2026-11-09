import {useEffect,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {useWorkspaceActive} from './WorkspaceSlot';
import {LocalizedLabel} from './localization';

export type OfflineFile={session:number;relative:string};
type Listing={session:number;name:string;relative:string;parent:string|null;truncated:boolean;entries:{name:string;directory:boolean;relative:string}[]};
export function OfflineBrowser({scope,disabled,onOpen}:{scope:'wave'|'data';disabled:boolean;onOpen:(file:OfflineFile)=>void}){
  const active=useWorkspaceActive(),[listing,setListing]=useState<Listing|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState('');
  const navigate=async(relative?:string)=>{setBusy(true);try{setListing(await invoke<Listing|null>(relative!==undefined&&listing?'offline_browser_list':'offline_browser_status',relative!==undefined&&listing?{scope,session:listing.session,relative}:{scope}));setError('');}catch(reason){setError(String(reason));}finally{setBusy(false);}};
  useEffect(()=>{if(active)void navigate();},[active,scope]);
  const choose=async(cache=false)=>{setBusy(true);setError('');try{const result=await invoke<Listing|null>('offline_browser_pick',{scope,useCache:cache});if(result)setListing(result);}catch(reason){setError(String(reason));}finally{setBusy(false);}};
  const blocked=busy||disabled;
  return <section className="offline-browser"><div className="section-heading"><strong><LocalizedLabel text="文件目录"/></strong><button className="button secondary" disabled={blocked} onClick={()=>void navigate(listing?.relative)}><LocalizedLabel text="刷新"/></button></div><div className="assistant-buttons"><button className="button secondary" disabled={blocked} onClick={()=>void choose()}><LocalizedLabel text="选择目录"/></button>{scope==='data'&&<button className="button secondary" disabled={blocked} onClick={()=>void choose(true)}><LocalizedLabel text="提取缓存"/></button>}</div>{listing&&<><small title={listing.relative}>{listing.name}{listing.relative?` / ${listing.relative}`:''}</small><ul>{listing.parent!==null&&<li><button disabled={blocked} onClick={()=>void navigate(listing.parent!)}>↰ <LocalizedLabel text="上一级"/></button></li>}{listing.entries.map(entry=><li key={entry.relative}><button disabled={blocked} title={entry.name} onClick={()=>entry.directory?void navigate(entry.relative):onOpen({session:listing.session,relative:entry.relative})}>{entry.directory?'▸':'·'} {entry.name}</button></li>)}</ul>{listing.truncated&&<small className="detail-warning">目录较大，仅显示前 1000 项；可进入子目录或用文件选择器打开。</small>}</>}{error&&<small role="alert" className="detail-warning">{error}</small>}</section>;
}
