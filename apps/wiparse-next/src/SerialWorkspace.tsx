import {SERIAL_FRAME_MS,pausesSerialFollow,readSerialPage} from './serialLive';
import {useWorkspaceActive} from './WorkspaceSlot';
import {LocalizedLabel,localizeLabel} from './localization';
import {useCallback,useEffect,useLayoutEffect,useMemo,useRef,useState} from 'react';
import {Channel,invoke} from '@tauri-apps/api/core';
import {latestTaskQueue} from './latestTaskQueue';
import {clearLiveSerialLog,closeSerialLog,openSerialLog,readLegacy,sendSerial,setSerialRecording,startSerial,stopSerial} from './bridge';
import {object,string} from './model';
import {resolveLogTab,retainedRowInput,searchSerialHistory,serialLogShortcut,collectSerialMatches,type SerialMatches} from './serialSearch';
import {SerialRecordingSettings} from './SerialRecordingSettings';
import {SerialLogBrowser,type OpenedLog} from './SerialLogBrowser';
import {SerialLineDetails,type SelectedLogLine} from './SerialLineDetails';
import {SerialLogEditor} from './SerialLogEditor';

export function SerialLog({connected,supported,clearSupported,aiOpen,onToggleAi,requested,setOwner}: {setOwner:React.Dispatch<React.SetStateAction<string|null>>;requested?:OpenedLog|null;connected: boolean;supported:boolean;clearSupported:boolean;aiOpen:boolean;onToggleAi:()=>void}) {
  const workspaceActive=useWorkspaceActive();
  const [logFont,setLogFont]=useState(13);
  useEffect(()=>{const element=logElement.current;if(!element)return;const zoom=(event:WheelEvent)=>{if(!event.ctrlKey)return;event.preventDefault();event.stopPropagation();setLogFont(size=>Math.max(10,Math.min(28,size+(event.deltaY<0?1:-1))));};element.addEventListener('wheel',zoom,{passive:false});return()=>element.removeEventListener('wheel',zoom);},[]);
  const [tabs,setTabs] = useState<unknown[]>([]);
  const [tab,setTab] = useState<number | null>(null);
  const [tabView,setTabView] = useState<number | null>(null);
  const [activeTab,setActiveTab] = useState(0);
  const [lines,setLines] = useState<string[]>([]);
  const [error,setError] = useState('');
  const [streamError,setStreamError]=useState(''),[refresh,setRefresh]=useState(0);
  const refreshLog=useRef<(()=>void)|null>(null),followingNow=useRef(true);
  const [selectedLine,setSelectedLine]=useState<SelectedLogLine|null>(null),[explainOpen,setExplainOpen]=useState(false);
  const [editorOpen,setEditorOpen]=useState(false);
  useEffect(()=>{setSelectedLine(null);setExplainOpen(false);setTargetRow(null);},[tabView]);
  const [query,setQuery] = useState(''),[caseSensitive,setCaseSensitive]=useState(false),[onlyMatches,setOnlyMatches]=useState(true),[gotoValue,setGotoValue]=useState(''),[targetRow,setTargetRow]=useState<number|null>(null);
  const searchInput=useRef<HTMLInputElement>(null),gotoInput=useRef<HTMLInputElement>(null);
  const [total,setTotal] = useState(0);
  const [offset,setOffset] = useState<number | null>(null);
  const [following,setFollowing]=useState(true);
  const logElement=useRef<HTMLDivElement>(null);
  followingNow.current=following;
  const resume=()=>{setOffset(null);setFollowing(true);setRefresh(value=>value+1);};
  const browse=(row:number)=>{setFollowing(false);setOffset(row);};
  const [first,setFirst] = useState(0);
  const [dropped,setDropped] = useState(0);
  const [searching,setSearching]=useState(false),[searchStatus,setSearchStatus]=useState('');
  const [results,setResults]=useState<SerialMatches|null>(null);
  const [fileBusy,setFileBusy]=useState(false);
  const [fileError,setFileError]=useState('');
  const searchGeneration=useRef(0),lastMatch=useRef<{view:number;query:string;row:number}|null>(null);
  useEffect(()=>{if(requested){setTab(requested.tab);setTabView(requested.view);setOffset(null);setFollowing(true);setFileError('');}},[requested?.nonce]);
  useEffect(()=>()=>{searchGeneration.current++;},[]);
  useEffect(()=>{searchGeneration.current++;setSearching(false);setSearchStatus('');setResults(null);lastMatch.current=null;},[query,caseSensitive,tab,tabView,connected,tab===null?activeTab:tab]);
  useEffect(()=>{
    if(!workspaceActive||editorOpen)return;
    if (!connected) { setLines([]); setTabs([]); setTotal(0); setDropped(0);setFirst(0); return; }
    if (!following && offset===null) return;
    const queue=latestTaskQueue<void,{list:Record<string,unknown>;entries:unknown[];entry:Record<string,unknown>|undefined;page:Record<string,unknown>|null}>(async(_,alive)=>{
      const list=object(await readLegacy('log.tabs.list'));
      const entries=Array.isArray(list.tabs)?list.tabs:[];
      const entry=resolveLogTab(entries,tab,tabView,typeof list.active_tab==='number'?list.active_tab:0),chosen=entry?.tab_id;
      if(!alive())return {list,entries,entry,page:null};
      const page=entry&&typeof chosen==='number'&&Number.isSafeInteger(chosen)?await readSerialPage(params=>readLegacy('log.lines.get',params),chosen,typeof entry.view_id==='number'?entry.view_id:undefined,typeof entry.total==='number'?entry.total:typeof entry.lines==='number'?entry.lines:0,offset,alive):null;
      return {list,entries,entry,page};
    },({list,entries,entry,page})=>{
      setTabs(entries);if(typeof list.active_tab==='number')setActiveTab(list.active_tab);
      if(!entry){setLines([]);setTotal(0);setDropped(0);if(tabView!==null)setError(localizeLabel('所选日志已关闭或清空，请重新选择。'));return;}
      if(!page)return;
      const received=Array.isArray(page.lines)?page.lines.filter((v):v is string=>typeof v==='string'):[];
      const next=typeof page.next_row==='number'?page.next_row:received.length;
      setTotal(typeof page.total==='number'?page.total:0);setFirst(Math.max(0,next-received.length));setDropped(typeof page.dropped==='number'?page.dropped:0);
      setLines(previous=>previous.length===received.length&&previous.every((line,index)=>line===received[index])?previous:received);setError('');
    },reason=>setError(reason instanceof Error?reason.message:String(reason)),SERIAL_FRAME_MS);
    const notify=()=>queue.submit();refreshLog.current=notify;notify();
    return()=>{if(refreshLog.current===notify)refreshLog.current=null;queue.dispose();};
  },[connected,tab,tabView,offset,following,editorOpen,workspaceActive,refresh]);
  useEffect(()=>{
    if(!connected||!workspaceActive)return;
    const id=crypto.randomUUID(),channel=new Channel<{kind:string;message?:string}>();let active=true;
    channel.onmessage=event=>{if(!active)return;if(event.kind==='error')setStreamError(event.message||localizeLabel('串口实时连接中断，正在重连。'));else if(event.kind==='changed'){setStreamError('');if(followingNow.current)refreshLog.current?.();}};
    void invoke('serial_events_start',{id,channel}).catch(reason=>{if(active)setStreamError(String(reason));});
    return()=>{active=false;void invoke('serial_events_stop',{id}).catch(()=>{});};
  },[connected,workspaceActive]);
  useLayoutEffect(()=>{const element=logElement.current;if(element&&following)element.scrollTop=element.scrollHeight;},[lines,following,logFont,query,onlyMatches]);
  const matching=useMemo(()=>lines.map((line,index)=>({line,row:first+index})).filter(({line})=>!onlyMatches||!query||(caseSensitive?line.includes(query):line.toLocaleLowerCase().includes(query.toLocaleLowerCase()))),[lines,query,caseSensitive,first,onlyMatches]);
  const selectedLog=resolveLogTab(tabs,tab,tabView,activeTab);
  const selectLog=async(id:number)=>{const entry=tabs.map(object).find(item=>item.tab_id===id);if(typeof entry?.view_id!=='number')return;setFileBusy(true);setFileError('');try{await invoke('serial_select_log_safe',{tabId:id,viewId:entry.view_id});setTab(id);setTabView(entry.view_id);resume();}catch(reason){setFileError(String(reason));}finally{setFileBusy(false);}};

  const openFile=async()=>{setFileBusy(true);setFileError('');try{const response=await openSerialLog();if(response===null)return;const result=object(response);if(typeof result.tab_id!=='number'||!Number.isSafeInteger(result.tab_id)||typeof result.view_id!=='number'||!Number.isSafeInteger(result.view_id))throw Error('日志打开结果未确认，请刷新检查。');setTab(result.tab_id);setTabView(result.view_id);resume();}catch(reason){const message=String(reason);setFileError(message.includes('未确认')?`打开结果未确认：${message}`:`打开日志失败：${message}`);}finally{setFileBusy(false);}};
  const closeFile=async()=>{const entry=resolveLogTab(tabs,tab,tabView,activeTab),id=entry?.tab_id,view=entry?.view_id;if(typeof id!=='number'||typeof view!=='number')return;setFileBusy(true);setFileError('');try{const result=object(await closeSerialLog(id,view));if(Array.isArray(result.tabs))setTabs(result.tabs);if(typeof result.active_tab==='number')setActiveTab(result.active_tab);setLines([]);setTab(null);setTabView(null);resume();}catch(reason){const message=String(reason);setFileError(message.includes('LOG.VIEW_CHANGED')?localizeLabel("日志页已变化，请刷新后重试。"):message.includes('LOG.EDIT_IN_PROGRESS')?localizeLabel("日志仍在编辑，请先保存或放弃编辑。"):message.includes('LOG.LIVE_SESSION')?localizeLabel("正在监控的日志不能关闭。"):message.includes('未确认')?`关闭结果未确认：${message}`:`关闭日志失败：${message}`);}finally{setFileBusy(false);}};
  const newLog=async()=>{
    const live=tabs.map(object).find(item=>item.tab_id===0);if(typeof live?.view_id!=='number'||typeof live.content_epoch!=='number')return;
    if(!window.confirm(localizeLabel("新建实时日志页？请先停止所有串口。已有文件保留，未保存的显示内容不会自动写入历史文件。")))return;
    setFileBusy(true);setFileError('');
    try{const result=object(await invoke('serial_new_log_safe',{viewId:live.view_id,contentEpoch:live.content_epoch,consent:true}));if(!Array.isArray(result.tabs))throw Error('新建结果未确认，请检查日志页');setTabs(result.tabs);setTab(null);setTabView(null);setLines([]);setTotal(0);setDropped(0);setFirst(0);resume();}
    catch(reason){setFileError(String(reason).includes('SERIAL.MONITOR_IN_PROGRESS')?localizeLabel("请先停止所有串口，再新建实时日志。"):String(reason));}
    finally{setFileBusy(false);}
  };
  const clearLive=async()=>{
    const view=selectedLog?.view_id,epoch=selectedLog?.content_epoch;
    if(selectedLog?.tab_id!==0||typeof view!=='number'||typeof epoch!=='number'||!Number.isSafeInteger(view)||!Number.isSafeInteger(epoch))return;
    if(!window.confirm(localizeLabel("清空实时显示中的日志？未保存的内容将无法恢复。")))return;
    setFileBusy(true);setFileError('');try{const result=object(await clearLiveSerialLog(view,epoch));const entries=Array.isArray(result.tabs)?result.tabs:[];const live=entries.map(object).find(item=>item.tab_id===0);if(typeof live?.view_id!=='number')throw Error('清空结果未确认，请刷新检查。');setTabs(entries);setLines([]);setTotal(0);setDropped(0);setFirst(0);setTab(0);setTabView(live.view_id);resume();}
    catch(reason){const message=String(reason);setFileError(message.includes('LOG.VIEW_CHANGED')?localizeLabel("实时日志已变化，请刷新后重试。"):message.includes('RUN.STATION_BUSY')?localizeLabel("测试运行中不能清空日志。"):message.includes('未确认')?`清空结果未确认：${message}`:`清空失败：${message}`);}finally{setFileBusy(false);}
  };
  const searchHistory=async(previous=false)=>{
    const needle=caseSensitive?query:query.toLocaleLowerCase();if(!needle||searching)return;
    const entry=resolveLogTab(tabs,tab,tabView,activeTab),selected=entry?.tab_id,view=entry?.view_id;
    if(!entry){setSearchStatus(localizeLabel("所选日志已变化，请重新选择。"));return;}
    if(typeof selected!=='number'||typeof view!=='number'||!Number.isSafeInteger(view)){setSearchStatus(localizeLabel("当前服务不支持安全的日志历史查找。"));return;}
    const token=++searchGeneration.current;setSearching(true);setSearchStatus(localizeLabel("正在查找已加载的日志…"));
    try{
      const after=lastMatch.current?.view===view&&lastMatch.current.query===needle?lastMatch.current.row:null;
      const result=await searchSerialHistory(params=>readLegacy('log.lines.get',params),selected,view,needle,after,()=>token!==searchGeneration.current,(cursor,end)=>setSearchStatus(`已检查 ${cursor.toLocaleString('zh-CN')} / ${end.toLocaleString('zh-CN')} 行…`),()=>new Promise(resolve=>setTimeout(resolve,25)),caseSensitive,previous?'previous':'next');
      if(!result||token!==searchGeneration.current)return;
      if(result.row!==null){lastMatch.current={view,query:needle,row:result.row};setOnlyMatches(false);setTargetRow(result.row);browse(Math.max(result.dropped,result.row-20));setSearchStatus(`找到第 ${(result.row+1).toLocaleString('zh-CN')} 行。`);return;}
      lastMatch.current=null;setSearchStatus(result.dropped>0?localizeLabel("已查找开始时保留的记录；未找到更多结果，更早的日志已移出内存。"):localizeLabel("已查找开始时已加载的记录；未找到更多结果。"));
    }catch(reason){if(token===searchGeneration.current){const message=String(reason);setSearchStatus(message.includes('LOG.LINE_TOO_LARGE')?localizeLabel("查找中断：单行日志超过 1 MiB，未跳过该行。"):message.includes('LOG.VIEW_CHANGED')||message.includes('LOG.CONTENT_CHANGED')?localizeLabel("查找中断：日志已变化，请重新查找。"):`查找中断：${message}`);}}finally{if(token===searchGeneration.current)setSearching(false);}
  };
  const findAll=async()=>{
    if(!query||searching)return;
    const entry=resolveLogTab(tabs,tab,tabView,activeTab);
    if(typeof entry?.tab_id!=='number'||typeof entry.view_id!=='number'){setSearchStatus(localizeLabel('所选日志已变化，请重新选择。'));return;}
    const token=++searchGeneration.current;setSearching(true);setResults(null);setSearchStatus(localizeLabel('正在查找已加载的日志…'));
    try{
      const found=await collectSerialMatches(params=>readLegacy('log.lines.get',params),entry.tab_id,entry.view_id,query,()=>token!==searchGeneration.current,(cursor,end)=>setSearchStatus(`已检查 ${cursor.toLocaleString('zh-CN')} / ${end.toLocaleString('zh-CN')} 行…`),()=>new Promise(resolve=>setTimeout(resolve,25)),caseSensitive);
      if(found&&token===searchGeneration.current){setResults(found);setSearchStatus('');setFollowing(false);}
    }catch(reason){if(token===searchGeneration.current)setSearchStatus(`查找中断：${String(reason)}`);}finally{if(token===searchGeneration.current)setSearching(false);}
  };
  const selectResult=async(row:number)=>{
    if(!results)return;const source=results,token=++searchGeneration.current;
    try{
      const page=object(await readLegacy('log.lines.get',{tab_id:source.tabId,view_id:source.viewId,content_epoch:source.epoch,from_row:row,limit:1,max_bytes:1_048_576}));
      if(token!==searchGeneration.current)return;
      if(page.view_id!==source.viewId||page.content_epoch!==source.epoch||page.dropped!==source.dropped||page.truncated===true||page.next_row!==row+1)throw Error('日志内容已变化，请重新查找');
      setOnlyMatches(false);setTargetRow(row);browse(Math.max(source.dropped,row-20));setSearchStatus(`已定位第 ${row+1} 行。`);
    }catch(reason){if(token===searchGeneration.current){setResults(null);setSearchStatus(`定位中断：${String(reason)}`);}}
  };
  const goToRow=()=>{const row=retainedRowInput(gotoValue,dropped,total);if(row===null){setSearchStatus(`请输入 ${dropped+1}–${total} 范围内的行号；更早的记录可能已移出内存。`);return;}setOnlyMatches(false);setTargetRow(row);browse(Math.max(dropped,row-20));setSearchStatus(`已定位第 ${row+1} 行。`);};
  useEffect(()=>{if(targetRow===null||following)return;const index=targetRow-first;if(index<0||index>=lines.length)return;setSelectedLine({row:targetRow,text:lines[index]});const item=logElement.current?.querySelector<HTMLElement>(`[data-row="${targetRow}"]`);item?.scrollIntoView({block:'center'});},[targetRow,first,lines,following]);
  useEffect(()=>{
    const keydown=(event:KeyboardEvent)=>{
      if(!workspaceActive||editorOpen||event.defaultPrevented||(event.target instanceof Element&&event.target.closest('[role="dialog"],.serial-assistant')))return;
      const action=serialLogShortcut(event.key,event.ctrlKey||event.metaKey,event.shiftKey,event.altKey,event.repeat);
      if(!action)return;
      event.preventDefault();
      if(action==='goto'){gotoInput.current?.focus();gotoInput.current?.select();}
      else if(action==='find'){searchInput.current?.focus();searchInput.current?.select();}
      else if(action==='escape'){setSelectedLine(null);setExplainOpen(false);}
      else if(connected&&query&&!searching)void searchHistory(action==='previous');
    };
    window.addEventListener('keydown',keydown,true);
    return()=>window.removeEventListener('keydown',keydown,true);
  });
  return <section className="card serial-log"><div className="section-heading"><div><h2><LocalizedLabel text={"串口日志"}/></h2><p><LocalizedLabel text={"每页最多 500 行 ·"}/>{following?localizeLabel("实时跟随"):localizeLabel("已暂停跟随，可滚动查看上下文")}<LocalizedLabel text={"· 选中行后可查看协议说明"}/></p></div><div className="assistant-buttons"><span className="table-count"><LocalizedLabel text={"共"}/>{total}<LocalizedLabel text={"行"}/></span><button type="button" className={`button secondary ${aiOpen?'active':''}`} onClick={onToggleAi}><LocalizedLabel text={"串口 AI"}/></button></div></div>
    <div className="log-controls"><label><LocalizedLabel text={"日志页"}/><select value={tabView!==null?String(tabs.map(object).find(item=>item.view_id===tabView)?.tab_id??'missing'):tab===null?'':String(tab)} disabled={fileBusy} onChange={event=>void selectLog(Number(event.target.value))}><option value="" disabled><LocalizedLabel text={"当前日志页"}/></option><option value="missing" disabled><LocalizedLabel text={"所选日志已变化"}/></option>{tabs.map((entry)=>{const t=object(entry),id=typeof t.tab_id==='number'?t.tab_id:0;return <option key={id} value={id}>{string(t.title,`日志 ${id+1}`)}</option>})}</select></label><button type="button" className="button secondary" disabled={!connected||!supported||fileBusy} onClick={()=>void openFile()}><LocalizedLabel text={"打开日志"}/></button><button type="button" className="button secondary" disabled={!connected||!supported||fileBusy} onClick={()=>void newLog()}><LocalizedLabel text={"新建实时页"}/></button>{selectedLog?.live===false&&typeof selectedLog.tab_id==='number'&&typeof selectedLog.view_id==='number'&&<SerialLogEditor key={selectedLog.view_id} tabId={selectedLog.tab_id} viewId={selectedLog.view_id} disabled={!connected||!supported||fileBusy||selectedLog.loading===true||!!selectedLog.load_error} setOwner={setOwner} onEditingChange={setEditorOpen} onOpened={()=>{setTab(Number(selectedLog.tab_id));setTabView(Number(selectedLog.view_id));setFollowing(false);}} onChanged={()=>{setTab(null);setTabView(null);setLines([]);resume();}}/>}<button type="button" className="button secondary" disabled={!connected||!supported||fileBusy||selectedLog?.live===true||typeof selectedLog?.tab_id!=='number'||selectedLog.tab_id===0||typeof selectedLog.view_id!=='number'} onClick={()=>void closeFile()}><LocalizedLabel text={"关闭日志页"}/></button>{selectedLog?.tab_id===0&&<button type="button" className="button secondary" title={localizeLabel("清空实时显示")} disabled={!connected||!clearSupported||fileBusy||typeof selectedLog?.view_id!=='number'||typeof selectedLog?.content_epoch!=='number'} onClick={()=>void clearLive()}><LocalizedLabel text={"清空"}/></button>}</div><div className="log-controls"><label className="search"><span>⌕</span><input ref={searchInput} value={query} onChange={e=>setQuery(e.target.value)} placeholder={localizeLabel("筛选当前页")} aria-label={localizeLabel("筛选当前页日志")} /></label><label className="log-case"><input type="checkbox" checked={caseSensitive} onChange={e=>setCaseSensitive(e.target.checked)}/><LocalizedLabel text={"区分大小写"}/></label><label className="log-case"><input type="checkbox" checked={onlyMatches} onChange={event=>setOnlyMatches(event.target.checked)}/><LocalizedLabel text="仅显示匹配行"/></label><button className="button secondary" type="button" title="Shift + F3" disabled={!connected||!query||searching||selectedLog?.loading===true||!!selectedLog?.load_error} onClick={()=>void searchHistory(true)}><LocalizedLabel text="上一匹配"/></button><button className="button secondary" type="button" disabled={!connected||!query||searching||selectedLog?.loading===true||!!selectedLog?.load_error} onClick={()=>void searchHistory()}><LocalizedLabel text={"查找已加载日志"}/></button><button className="button secondary" type="button" disabled={!connected||!query||searching||selectedLog?.loading===true||!!selectedLog?.load_error} onClick={()=>void findAll()}><LocalizedLabel text="查找全部"/></button>{searching&&<button className="button secondary" type="button" onClick={()=>{searchGeneration.current++;setSearching(false);setSearchStatus(localizeLabel("查找已取消。"));}}><LocalizedLabel text={"取消"}/></button>}<label className="log-goto"><LocalizedLabel text="行号"/><input ref={gotoInput} inputMode="numeric" aria-label={localizeLabel('跳转到日志行')} value={gotoValue} onChange={event=>setGotoValue(event.target.value)} onKeyDown={event=>{if(event.key==='Enter'){event.preventDefault();goToRow();}}}/></label><button className="button secondary" type="button" disabled={!connected||total<=dropped} onClick={goToRow}><LocalizedLabel text="跳转"/></button><button className="button secondary" type="button" disabled={!connected||first<=dropped} onClick={()=>browse(Math.max(dropped,first-500))}><LocalizedLabel text={"上一页"}/></button><button className="button secondary" type="button" disabled={!connected||first+lines.length>=total} onClick={()=>browse(first+lines.length)}><LocalizedLabel text={"下一页"}/></button><button className="button secondary" type="button" disabled={!connected||following} onClick={resume}><LocalizedLabel text={"最新"}/></button><button className="button secondary" type="button" disabled={!selectedLine} onClick={()=>{setFollowing(false);setExplainOpen(value=>!value);}}><LocalizedLabel text={"协议说明"}/></button><span className="table-count">{lines.length ? `${first+1}–${first+lines.length} / ${total}` : localizeLabel("暂无日志")}{query&&onlyMatches ? ` · 筛选 ${matching.length} 行` : ''}</span></div>
    {results&&<aside className="serial-search-results" aria-label={localizeLabel('查找结果')}><header><strong><LocalizedLabel text="查找结果"/> · {results.count.toLocaleString('zh-CN')} <LocalizedLabel text="匹配行"/></strong><small><LocalizedLabel text="查找开始时保留的日志快照"/>{results.limited&&<> · <LocalizedLabel text="仅列出前 1000 行"/></>}</small><button type="button" className="icon-button" aria-label={localizeLabel('关闭查找结果')} onClick={()=>setResults(null)}>×</button></header><div>{results.rows.map(hit=><button type="button" key={hit.row} onClick={()=>void selectResult(hit.row)}><span>{hit.row+1}</span><code>{hit.preview}</code></button>)}{!results.count&&<p><LocalizedLabel text="没有匹配行"/></p>}</div></aside>}
    {explainOpen&&selectedLine&&<SerialLineDetails line={selectedLine} onClose={()=>setExplainOpen(false)}/>}
    {searchStatus&&<div className="detail-help" role="status">{searchStatus}</div>}
    {selectedLog?.loading===true&&<div className="detail-help" role="status"><LocalizedLabel text={"正在加载日志，已索引"}/>{String(selectedLog.index_progress??0)}<LocalizedLabel text={"行。"}/></div>}
    {!!selectedLog?.load_error&&<div className="log-error" role="alert"><LocalizedLabel text={"日志打开失败："}/>{string(selectedLog.load_error)}</div>}
    {fileError&&<div className="log-error" role="alert">{fileError}</div>}
    {streamError&&<div className="log-error" role="alert">{streamError}</div>}
    {error && <div className="log-error"><LocalizedLabel text={"日志读取失败："}/>{error}</div>}
    {dropped>0 && <div className="log-error"><LocalizedLabel text={"已有"}/>{dropped.toLocaleString('zh-CN')}<LocalizedLabel text={"行移出内存；若事先启用了主串口日志记录，可检查保存的文件。"}/></div>}
    <div className="serial-log-pane"><div ref={logElement} className="log-content" style={{fontSize:logFont}} role="log" aria-live={following?'polite':'off'} aria-label={localizeLabel("串口日志")} tabIndex={0} onWheel={event=>{if(pausesSerialFollow({kind:'wheel',deltaY:event.deltaY,control:event.ctrlKey,meta:event.metaKey}))setFollowing(false);}} onKeyDown={event=>{if(pausesSerialFollow({kind:'key',key:event.key}))setFollowing(false);}} onPointerDown={event=>{const element=event.currentTarget,rect=element.getBoundingClientRect();if(pausesSerialFollow({kind:'pointer',button:event.button,x:event.clientX,right:rect.right,scrollbar:element.offsetWidth-element.clientWidth}))setFollowing(false);}}>{matching.length?matching.map(({line,row})=><div data-row={row} className={`log-line ${selectedLine?.row===row?'log-line-selected':''}`} key={row} onClick={()=>setSelectedLine({row,text:line})} style={{gridTemplateColumns:`${Math.max(4,String(total).length)}ch minmax(0,1fr)`}}><span className="log-line-number" aria-label={`第 ${row+1} 行`}>{row+1}</span><span>{line || '\u00a0'}</span></div>):<div className="log-empty">{connected?localizeLabel("当前没有可显示的日志"):localizeLabel("设备宿主就绪后显示日志")}</div>}</div><div className="log-scroll-actions"><button type="button" disabled={!connected||first<=dropped} title={localizeLabel("上一页")} aria-label={localizeLabel("查看更早日志")} onClick={()=>browse(Math.max(dropped,first-500))}>▴</button><span/><button type="button" disabled={!connected} title={localizeLabel("回到最新并恢复刷新")} aria-label={localizeLabel("回到最新并恢复刷新")} onClick={resume}>▾</button></div></div>
  </section>;
}

export function SerialStatus({connected, initial,setOwner,onOpened}: {connected: boolean; initial: unknown;onOpened:(value:OpenedLog)=>void;setOwner:React.Dispatch<React.SetStateAction<string|null>>}) {
  const workspaceActive=useWorkspaceActive();
  const [recordingEditing,setRecordingEditing]=useState(false);
  useEffect(()=>{if(!recordingEditing)return;setOwner(current=>current??'serial-recording');return()=>setOwner(current=>current==='serial-recording'?null:current);},[recordingEditing,setOwner,workspaceActive]);
  const [status,setStatus] = useState<unknown>(initial),[ports,setPorts]=useState<{device:string;label:string}[]>([]),[selected,setSelected]=useState(''),[baud,setBaud]=useState('115200'),[busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState(''),[attempted,setAttempted]=useState(false);
  const [pollError,setPollError]=useState('');
  const [sendMode,setSendMode]=useState('text'),[sendValue,setSendValue]=useState(''),[appendNewline,setAppendNewline]=useState(false);
  const supported=object(initial).safe_control_version===1;
  const refreshPorts=useCallback(async()=>{try{const data=await readLegacy('serial.ports');const list=(Array.isArray(data)?data:[]).flatMap(raw=>{const row=object(raw),device=string(row.device);return device?[{device,label:string(row.label)||device}]:[];});setPorts(list);setError('');}catch(reason){setError(`串口列表读取失败：${String(reason)}`);}},[]);
  useEffect(()=>{if(connected&&workspaceActive)void refreshPorts();},[connected,refreshPorts,workspaceActive]);
  useEffect(()=>{if(!selected&&ports.length)setSelected(ports[0].device);},[ports,selected]);
  useEffect(()=>{
    if(!workspaceActive)return;
    if (!connected) { setStatus(null); return; }
    let active=true, timer: ReturnType<typeof setTimeout>;
    const poll=async()=>{
      try { const next=await readLegacy('serial.monitor.status'); if (active) {setStatus(next);setPollError('');} }
      catch (reason) { if (active) {setStatus(null);setPollError(String(reason));} }
      if (active) timer=setTimeout(poll,1000);
    };
    void poll(); return ()=>{active=false;clearTimeout(timer);};
  },[connected,workspaceActive]);
  const serial=object(status),sessions=Array.isArray(serial.sessions)?serial.sessions.map(object):[],session=sessions.find(item=>string(item.port).toUpperCase()===selected.toUpperCase()),errors=object(serial.errors),openError=string(Object.entries(errors).find(([port])=>port.toUpperCase()===selected.toUpperCase())?.[1]);
  const recording=object(serial.recording),recordingSupported=object(initial).safe_recording_version===1;
  const statusAvailable=Array.isArray(serial.sessions);const state=!statusAvailable?'状态未知':session?.confirmed===true?'已连接':session?.confirmed===false?'正在连接':session?'状态未知':attempted&&openError?'连接失败':'未连接';
  useEffect(()=>{if(session?.confirmed===true)setNotice('');},[session?.confirmed]);
  const start=async()=>{const rate=Number(baud);if(!selected||!Number.isInteger(rate)||rate<1||rate>12000000){setError(localizeLabel("请选择串口并填写有效波特率。"));return;}const adding=sessions.length>0&&!sessions.some(item=>string(item.port).toUpperCase()===selected.toUpperCase());setBusy(true);setError('');setNotice('');setAttempted(true);try{await startSerial(selected,rate);setNotice(adding?localizeLabel("附加串口启动请求已提交。它使用单独的日志页，主串口记录文件不包含这一路。"):localizeLabel("启动请求已提交，等待设备打开。"));}catch(reason){const message=String(reason);setError(message.includes('SERIAL.TEST_IN_PROGRESS')?localizeLabel("测试运行中，串口由测试接管。"):/timeout|timed out/i.test(message)?localizeLabel("启动结果未确认，请查看当前状态；不会自动重试。"):message);}finally{setBusy(false);}};
  const stopPort=async(port:string)=>{if(!port||!sessions.some(item=>string(item.port).toUpperCase()===port.toUpperCase()))return;setBusy(true);setError('');setNotice('');try{await stopSerial(port);if(port.toUpperCase()===selected.toUpperCase())setAttempted(false);setNotice(localizeLabel("停止请求已提交。"));}catch(reason){const message=String(reason);setError(message.includes('SERIAL.TEST_IN_PROGRESS')?localizeLabel("测试运行中，串口由测试接管。"):/timeout|timed out/i.test(message)?localizeLabel("停止结果未确认，请查看当前状态；不会自动重试。"):message);}finally{setBusy(false);}};
  const setRecording=async(enabled:boolean)=>{setBusy(true);setError('');try{const result=await setSerialRecording(enabled);setStatus((previous:unknown)=>({...object(previous),recording:result}));setNotice(enabled?localizeLabel("已启用主串口日志记录。"):localizeLabel("已关闭主串口日志记录。"));}catch(reason){const message=String(reason);setError(message.includes('SERIAL.MONITOR_IN_PROGRESS')?localizeLabel("请先停止所有串口，再修改记录设置。"):message.includes('SERIAL.TEST_IN_PROGRESS')?localizeLabel("测试运行中，暂不能修改记录设置。"):/timeout|timed out/i.test(message)?localizeLabel("设置结果未确认，请查看当前状态；不会自动重试。"):message);}finally{setBusy(false);}};
  const presetSource=Array.isArray(serial.baud_options)&&serial.baud_options.length?serial.baud_options.map(String):['115200','1000000','2000000'];
  const baudPresets=[...new Set(presetSource.filter(value=>/^[0-9]+$/.test(value)&&Number(value)>=1&&Number(value)<=12000000))].slice(0,32);
  const send=async()=>{let bytes:number[];if(sendMode==='hex'){const raw=sendValue.trim();if(!raw||!/^[0-9a-f\s,]+$/i.test(raw)){setError(localizeLabel("请输入十六进制字节。"));return;}const clean=raw.replace(/[\s,]/g,'');if(clean.length%2){setError(localizeLabel("十六进制必须是完整字节。"));return;}bytes=Array.from(clean.match(/../g)||[],value=>parseInt(value,16));}else{bytes=Array.from(new TextEncoder().encode(sendValue+(appendNewline?'\r\n':'')));}if(!bytes.length||bytes.length>4096){setError(localizeLabel("发送内容须为 1 至 4096 字节。"));return;}setBusy(true);setError('');setNotice('');try{await sendSerial(selected,bytes);setNotice(localizeLabel("数据已加入发送队列；设备接收情况请以日志为准。"));}catch(reason){const message=String(reason);setError(message.includes('SERIAL.TEST_IN_PROGRESS')?localizeLabel("测试运行中，串口由测试接管。"):/timeout|timed out/i.test(message)?localizeLabel("发送结果未确认，请查看日志；不会自动重试。"):message);}finally{setBusy(false);}};
  return <section className="card"><div className="serial-stats"><strong><LocalizedLabel text={"连接与采集"}/></strong><label><LocalizedLabel text={"串口"}/><select value={selected} disabled={!connected||busy} onChange={e=>{setSelected(e.target.value);setAttempted(false);setError('');setNotice('');}}><option value=""><LocalizedLabel text={"请选择"}/></option>{ports.map(port=><option key={port.device} value={port.device}>{port.label}</option>)}{selected&&!ports.some(port=>port.device===selected)&&<option value={selected}>{selected}</option>}</select></label><button type="button" className="button secondary" disabled={!connected||busy} onClick={()=>void refreshPorts()}><LocalizedLabel text={"刷新串口"}/></button><label><LocalizedLabel text={"常用波特率"}/><select value={baudPresets.includes(baud)?baud:'custom'} disabled={busy||!!session} onChange={e=>{if(e.target.value==='custom'){if(baudPresets.includes(baud))setBaud('');}else setBaud(e.target.value);}}>{baudPresets.map(value=><option key={value} value={value}>{value}</option>)}<option value="custom"><LocalizedLabel text={"自定义"}/></option></select></label><label><LocalizedLabel text={"波特率"}/><input type="number" min="1" max="12000000" value={baud} disabled={busy||!!session} onChange={e=>setBaud(e.target.value)}/></label><label className="item-enabled"><input type="checkbox" checked={recording.enabled===true} disabled={!connected||!recordingSupported||!statusAvailable||busy||sessions.length>0||recordingEditing} onChange={e=>void setRecording(e.target.checked)}/><LocalizedLabel text={"保存主串口日志"}/></label>{object(initial).safe_recording_settings_version===1&&<SerialRecordingSettings onEditingChange={setRecordingEditing} recording={recording} disabled={!connected||!statusAvailable||busy||sessions.length>0} onSaved={value=>setStatus((previous:unknown)=>({...object(previous),recording:value}))}/>}{recording.enabled===true&&!!recording.error&&<div className="detail-warning" role="alert"><LocalizedLabel text={"日志记录失败："}/>{string(recording.error)}</div>}{recording.active===true&&<div className="detail-help" title={string(recording.file_path)}><LocalizedLabel text={"正在记录"}/>{string(recording.file_name)?`：${string(recording.file_name)}`:''}</div>}<div className="serial-buttons"><button type="button" className="button primary" disabled={!connected||!supported||!statusAvailable||busy||recordingEditing||!!session||!selected} onClick={()=>void start()}><LocalizedLabel text={"开始"}/></button><button type="button" className="button secondary" disabled={!connected||!supported||!statusAvailable||busy||!session} onClick={()=>void stopPort(selected)}><LocalizedLabel text={"停止"}/></button></div>{sessions.length>0&&<div className="serial-sessions">{sessions.map(item=>{const port=string(item.port);if(!port)return null;return <div className="serial-session" key={port}><strong>{item.tab_id===0?localizeLabel("主串口"):localizeLabel("附加串口")} · {port}</strong><small>{item.confirmed===true&&typeof item.baud==='number'?`${item.baud} baud`:item.confirmed===true?localizeLabel("已连接"):localizeLabel("正在连接")}</small><button type="button" className="button secondary" disabled={!connected||!supported||!statusAvailable||busy} onClick={()=>void stopPort(port)}><LocalizedLabel text={"停止"}/></button></div>;})}</div>}<div><span><LocalizedLabel text={"连接状态"}/></span><strong>{connected?state:localizeLabel("服务未连接")}</strong>{session?.confirmed===true&&typeof session.baud==='number'&&<small>{String(session.baud)} baud</small>}</div><label><LocalizedLabel text={"发送格式"}/><select value={sendMode} disabled={busy} onChange={e=>setSendMode(e.target.value)}><option value="text"><LocalizedLabel text={"文本（UTF-8）"}/></option><option value="hex"><LocalizedLabel text={"十六进制"}/></option></select></label><label><LocalizedLabel text={"发送内容"}/><textarea rows={3} value={sendValue} disabled={busy} onChange={e=>setSendValue(e.target.value)}/></label>{sendMode==='text'&&<label className="item-enabled"><input type="checkbox" checked={appendNewline} disabled={busy} onChange={e=>setAppendNewline(e.target.checked)}/><LocalizedLabel text={"追加换行"}/></label>}<button type="button" className="button secondary" disabled={!connected||!supported||busy||session?.confirmed!==true||!sendValue} onClick={()=>void send()}><LocalizedLabel text={"发送"}/></button>{pollError&&<div className="detail-warning" role="alert"><LocalizedLabel text={"状态读取失败："}/>{pollError}</div>}{!supported&&connected&&<div className="detail-help"><LocalizedLabel text={"此功能需要更新现有 WiParse 后端。"}/></div>}{notice&&<div className="detail-help" role="status">{notice}</div>}{attempted&&openError&&!session&&<div className="detail-warning" role="alert">{openError}</div>}{error&&<div className="detail-warning" role="alert">{error}</div>}<SerialLogBrowser disabled={!connected||busy||recordingEditing} onOpened={onOpened}/></div></section>;
}

