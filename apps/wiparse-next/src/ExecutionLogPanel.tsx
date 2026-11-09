import {useEffect,useRef,useState} from 'react';
import {readLegacy} from './bridge';
import {object,string} from './model';
import {useWorkspaceActive} from './WorkspaceSlot';
import {LocalizedLabel} from './localization';
import {applyExecutionPage,emptyExecutionText} from './executionLog';

/** Host execution identity is distinct from the plugin's persisted run_id. */
export function ExecutionLogPanel({project,plugin}:{project:string;plugin:string}){
  const active=useWorkspaceActive(),[open,setOpen]=useState(false),[sessions,setSessions]=useState<Record<string,unknown>[]>([]),[chosen,setChosen]=useState(''),[text,setText]=useState(''),[error,setError]=useState(''),[id,setId]=useState(''),[following,setFollowing]=useState(true),[dropped,setDropped]=useState(0),[incomplete,setIncomplete]=useState(false);
  const sourceKey=JSON.stringify([project,plugin,chosen]),liveSource=useRef(sourceKey);liveSource.current=sourceKey;
  const cursor=useRef(0),execution=useRef(''),retained=useRef(emptyExecutionText()),area=useRef<HTMLPreElement>(null);
  useEffect(()=>{setChosen('');setSessions([]);setFollowing(true);},[project,plugin]);
  useEffect(()=>{setText('');setId('');setError('');setDropped(0);setIncomplete(false);cursor.current=0;execution.current='';retained.current=emptyExecutionText();},[project,plugin,chosen]);
  useEffect(()=>{
    if(!active||!open||!following)return;
    let current=true,timer:ReturnType<typeof setTimeout>;
    const poll=async()=>{let delay=1000;try{const reply=object(await readLegacy('ui.test_tool.execution_log_safe',{project,plugin,execution_id:execution.current||null,selected_execution_id:chosen||null,from:cursor.current}));if(!current||liveSource.current!==sourceKey)return;if(Array.isArray(reply.sessions))setSessions(reply.sessions.map(object));
      const next=applyExecutionPage(retained.current,reply,project,plugin);retained.current=next;cursor.current=next.cursor;execution.current=next.id;setText(next.text);setId(next.id);setDropped(next.dropped);setIncomplete(next.incomplete);
      if(reply.changed===true||next.cursor<Number(reply.total))delay=25;
      setError(reply.available!==true&&chosen?'所选执行会话已不在保留范围内，请选择其他会话。':'');
    }catch(reason){if(current&&liveSource.current===sourceKey)setError(String(reason));}if(current&&liveSource.current===sourceKey)timer=setTimeout(poll,delay);};
    void poll();return()=>{current=false;clearTimeout(timer);};
  },[active,open,following,project,plugin,chosen]);
  useEffect(()=>{if(following&&area.current)area.current.scrollTop=area.current.scrollHeight;},[text,following]);
  return <details className="execution-log" open={open} onToggle={event=>setOpen(event.currentTarget.open)}><summary><LocalizedLabel text="执行日志"/> · {plugin}</summary><div className="section-heading"><label>执行会话<select value={chosen} onChange={event=>{setChosen(event.target.value);setFollowing(true);}}><option value="">最新会话</option>{chosen&&!sessions.some(session=>session.id===chosen)&&<option value={chosen}>会话已不在保留范围内</option>}{sessions.map(session=><option key={string(session.id)} value={string(session.id)}>{string(session.id).slice(0,8)} · {String(session.bytes)} 字节</option>)}</select></label><small>执行会话 {id||'—'} · {project}{dropped>0?` · 较早 ${dropped} 字节已移出保留窗口`:''}</small><button className="button secondary" onClick={()=>setFollowing(value=>!value)}>{following?'暂停跟随':'跳到最新并跟随'}</button></div>{incomplete&&<p className="detail-warning">子进程日志管道未正常结束，尾部完整性未确认。</p>}{error&&<p role="alert" className="detail-warning">{error}</p>}<pre ref={area} onWheel={event=>{if(event.deltaY<0)setFollowing(false);}}>{text||'当前工程/插件尚无执行日志。'}</pre></details>;
}
