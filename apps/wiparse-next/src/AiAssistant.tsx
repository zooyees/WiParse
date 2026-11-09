import {useWorkspaceActive} from './WorkspaceSlot';
import {LocalizedLabel,localizeLabel} from './localization';
import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {readLegacy} from './bridge';
import {object,string} from './model';

type Entry={role:string;text:string;time:string;error:boolean;automatic:boolean};
type AiState={messages_revision:string;binding:string;source:Record<string,unknown>;enabled:boolean;destination:string;model:string;busy:boolean;controls:boolean;status:string;status_error:boolean;monitoring:boolean;ai_paused:boolean;alerts:number;interval:number;entries:Entry[]};
type Preview={token:string;destination:string;model:string;question:string;source:Record<string,unknown>;observation:Record<string,unknown>;history:Array<{role:string;content:string}>;controls:boolean;watch:unknown};
type Limit={key:string;min:string;max:string};

const message=(reason:unknown)=>{
  const text=String(reason);
  if(/SOURCE_CHANGED|PREVIEW_STALE/.test(text))return '工程、串口源、配置或权限已变化，请重新检查发送内容。';
  if(text.includes('LIVE_SOURCE_REQUIRED'))return '请先启动并确认实时串口连接。';
  if(text.includes('CONTROL_LOCKED'))return '测试或 AI 任务进行中，请先结束任务再授权控制。';
  if(text.includes('BUSY'))return '已有任务正在处理，请稍候。';
  return text;
};

/** Shares the original serial AI engine, tool guards and local alert observer. */
export function AiAssistant({onClose}:{onClose:()=>void}) {
  const workspaceActive=useWorkspaceActive();
  const [state,setState]=useState<AiState|null>(null),[error,setError]=useState(''),[connectionError,setConnectionError]=useState(''),[busy,setBusy]=useState(false);
  const [text,setText]=useState(''),[scope,setScope]=useState('recent'),[tab,setTab]=useState<'chat'|'monitor'>('chat');
  const [preview,setPreview]=useState<Preview|null>(null),[interval,setInterval]=useState('5'),[contains,setContains]=useState('ERROR,FAULT,PANIC,异常,故障'),[limits,setLimits]=useState<Limit[]>([]);
  const conversation=useRef<HTMLDivElement>(null),following=useRef(true),messagesRevision=useRef('');
  useEffect(()=>{if(!workspaceActive)return;let active=true,timer:ReturnType<typeof setTimeout>;const poll=async()=>{
    try{const value=await readLegacy('ui.ai.serial.state_safe',{messages_revision:messagesRevision.current}) as AiState;if(active){messagesRevision.current=value.messages_revision;setState(previous=>({...value,entries:value.entries??previous?.entries??[]}));setConnectionError('');}}
    catch(reason){if(active)setConnectionError(message(reason));}
    if(active)timer=setTimeout(poll,1000);
  };void poll();return()=>{active=false;clearTimeout(timer);};},[workspaceActive]);
  useEffect(()=>{const element=conversation.current;if(element&&following.current)element.scrollTop=element.scrollHeight;},[state?.entries]);
  const action=async(name:string,params:Record<string,unknown>={})=>{
    if(!state||busy)return;
    setBusy(true);setError('');
    try{
      const result=await invoke<AiState>('serial_ai_action',{action:name,params:{binding:state.binding,...params}});
      setState(result);if(name==='send'){setPreview(null);setText('');following.current=true;}
    }catch(reason){setError(message(reason));}finally{setBusy(false);}
  };
  const prepare=async()=>{
    if(!state||!text.trim())return;
    setBusy(true);setError('');
    try{
      const rules=limits.map(rule=>({key:rule.key.trim(),min:rule.min.trim()?Number(rule.min):null,max:rule.max.trim()?Number(rule.max):null}));
      if(rules.some(rule=>(rule.min!==null&&!Number.isFinite(rule.min))||(rule.max!==null&&!Number.isFinite(rule.max))))throw Error('请填写有效的数值限值。');
      setPreview(await invoke<Preview>('serial_ai_action',{action:'preview',params:{binding:state.binding,question:text.trim(),scope:tab==='monitor'?'recent':scope,
        watch:tab==='monitor'?{interval:Number(interval),contains:contains.split(/[,，\n]/).map(word=>word.trim()).filter(Boolean),limits:rules}:null}}));
    }catch(reason){setError(message(reason));}finally{setBusy(false);}
  };
  const blocked=busy||!state||state.busy;
  const source=object(state?.source);
  return <aside className="serial-assistant card" aria-label={localizeLabel("串口 AI 助手")}>
    <header className="assistant-header"><strong><LocalizedLabel text={"串口 AI"}/></strong><span className="assistant-engine">{state?.busy?localizeLabel("处理中"):state?.monitoring?localizeLabel("监控中"):localizeLabel("就绪")}</span><button type="button" className="icon-button" aria-label={localizeLabel("关闭串口 AI 面板")} onClick={onClose}>×</button></header>
    <div className="assistant-source" title={string(source.device)||string(source.selected_device)}><span>{string(source.status,'等待连接')}</span><span>{string(source.source_title)||localizeLabel("当前串口源")}</span></div>
    <div className="assistant-tabs" role="tablist"><button type="button" role="tab" aria-selected={tab==='chat'} onClick={()=>{setTab('chat');setPreview(null);}}><LocalizedLabel text={"对话分析"}/></button><button type="button" role="tab" aria-selected={tab==='monitor'} onClick={()=>{setTab('monitor');setPreview(null);}}><LocalizedLabel text={"监控任务"}/>{state?.alerts?` · ${state.alerts}`:''}</button></div>
    <div className="assistant-body" ref={conversation} onScroll={event=>{const element=event.currentTarget;following.current=element.scrollHeight-element.scrollTop-element.clientHeight<32;}}>
      {tab==='monitor'&&<section className="assistant-monitor">
        <div className="assistant-monitor-status"><strong>{state?.monitoring?(state.ai_paused?localizeLabel("AI 分析已暂停，本地告警继续"):localizeLabel("正在监控新增数据")):localizeLabel("尚未启动监控")}</strong><span>{state?.alerts??0}<LocalizedLabel text={"次本地告警"}/></span></div>
        <label><LocalizedLabel text={"分析间隔（秒）"}/><input type="number" min="1" max="300" value={interval} disabled={blocked||state?.monitoring} onChange={event=>{setInterval(event.target.value);setPreview(null);}}/></label>
        <label><LocalizedLabel text={"本地预警关键词"}/><input value={contains} disabled={blocked||state?.monitoring} onChange={event=>{setContains(event.target.value);setPreview(null);}} placeholder={localizeLabel("用逗号分隔")}/></label>
        <div className="section-heading"><strong><LocalizedLabel text={"数值预警"}/></strong><button type="button" className="button secondary" disabled={blocked||state?.monitoring||limits.length>=16} onClick={()=>setLimits(previous=>[...previous,{key:'',min:'',max:''}])}><LocalizedLabel text={"添加"}/></button></div>
        {limits.map((rule,index)=><div className="assistant-limit" key={index}>{(['key','min','max'] as const).map(key=><input key={key} aria-label={`${index+1} ${key==='key'?'字段':key==='min'?'下限':'上限'}`} placeholder={key==='key'?localizeLabel("字段"):key==='min'?localizeLabel("下限"):localizeLabel("上限")} value={rule[key]} disabled={blocked||state?.monitoring} onChange={event=>{setLimits(previous=>previous.map((value,i)=>i===index?{...value,[key]:event.target.value}:value));setPreview(null);}}/>)}<button type="button" aria-label={localizeLabel("移除预警")} disabled={blocked||state?.monitoring} onClick={()=>{setLimits(previous=>previous.filter((_,i)=>i!==index));setPreview(null);}}>×</button></div>)}
        <p><LocalizedLabel text={"本地规则随接收批次检查；AI 按 1–300 秒有界采样，服务慢时顺延。AI 结论不作为测试判定。"}/></p>
        <div className="assistant-buttons">{state?.monitoring&&<button type="button" className="button secondary" disabled={busy} onClick={()=>void action('stop')}><LocalizedLabel text={"停止监控"}/></button>}{state?.ai_paused&&<button type="button" className="button secondary" disabled={busy} onClick={()=>void action('resume')}><LocalizedLabel text={"恢复 AI 分析"}/></button>}</div>
      </section>}
      {!state?.entries.length&&<div className="assistant-empty"><strong><LocalizedLabel text={"分析串口数据，协助执行测试"}/></strong><p><LocalizedLabel text={"可以分析最近日志、持续识别异常，或在授权后请求串口启停及已连接仪器操作。"}/></p></div>}
      {state?.entries.map((entry,index)=><article className={`assistant-message role-${entry.role} ${entry.error?'message-error':''}`} key={`${entry.time}:${index}`}><div className="assistant-message-meta"><strong>{entry.role==='user'?localizeLabel("你"):entry.role==='alert'?localizeLabel("本地预警"):entry.role==='assistant'?'AI':localizeLabel("系统")}</strong><time>{entry.time}</time>{entry.role==='assistant'&&<button type="button" onClick={()=>void navigator.clipboard.writeText(entry.text).catch(()=>setError(localizeLabel("复制失败")))}><LocalizedLabel text={"复制"}/></button>}</div><div className="assistant-markdown"><ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml components={{a:({children})=><span>{children}</span>,img:()=>null,table:({children})=><div className="assistant-table"><table>{children}</table></div>}}>{entry.text}</ReactMarkdown></div></article>)}
      {preview&&<section className="assistant-preview"><strong><LocalizedLabel text={"确认"}/>{preview.watch?localizeLabel("开启持续监控"):localizeLabel("发送分析")}</strong><div>{preview.destination}</div><small><LocalizedLabel text={"模型："}/>{preview.model}<LocalizedLabel text={"· 控制："}/>{preview.controls?localizeLabel("已授权"):localizeLabel("关闭")}</small><p>{preview.question}</p><details><summary><LocalizedLabel text={"待发送的数据及历史"}/></summary><pre>{(Array.isArray(preview.observation.recent_lines)?preview.observation.recent_lines:[]).join('\n')||localizeLabel("不附带日志")}</pre>{preview.history.map((entry,index)=><div key={index}><strong>{entry.role==='user'?localizeLabel("你"):'AI'}</strong><p>{entry.content}</p></div>)}</details><p>{preview.watch?localizeLabel("监控期间会持续发送当前来源的新增日志摘要、统计与有限异常样例。"):localizeLabel("仅发送本次预览数据和上述历史；新到日志不会替换已确认内容。")}</p><div className="assistant-buttons"><button type="button" className="button primary" disabled={blocked} onClick={()=>void action('send',{token:preview.token,consent:true})}><LocalizedLabel text={"确认"}/>{preview.watch?localizeLabel("启动"):localizeLabel("发送")}</button><button type="button" className="button secondary" disabled={busy} onClick={()=>setPreview(null)}><LocalizedLabel text={"返回修改"}/></button></div></section>}
    </div>
    <footer className="assistant-composer">
      {(error||connectionError)&&<div className="detail-warning" role="alert">{error||connectionError}</div>}
      {!state?.enabled&&<div className="detail-help"><LocalizedLabel text={"请先在设置中配置并启用全局 AI。"}/></div>}
      <div className="assistant-options"><select aria-label={localizeLabel("分析数据范围")} value={scope} disabled={tab==='monitor'||blocked} onChange={event=>{setScope(event.target.value);setPreview(null);}}><option value="recent"><LocalizedLabel text={"最近 100 行"}/></option><option value="none"><LocalizedLabel text={"仅问题与历史"}/></option></select><label><input type="checkbox" checked={state?.controls===true} disabled={busy||!state||source.automation_locked===true} onChange={event=>void action('control',{enabled:event.target.checked})}/><LocalizedLabel text={"允许设备控制"}/></label></div>
      <textarea aria-label={tab==='monitor'?localizeLabel("监控目标"):localizeLabel("分析问题")} rows={3} maxLength={8000} value={text} disabled={blocked} placeholder={tab==='monitor'?localizeLabel("描述监控目标与异常条件…"):localizeLabel("输入问题或测试目标；Ctrl+Enter 检查发送…")} onChange={event=>{setText(event.target.value);setPreview(null);}} onKeyDown={event=>{if(event.key==='Enter'&&event.ctrlKey&&!blocked){event.preventDefault();void prepare();}}}/>
      <div className="assistant-buttons"><button type="button" className="button primary" disabled={blocked||!state?.enabled||!text.trim()||(tab==='monitor'&&state.monitoring)} onClick={()=>void prepare()}>{tab==='monitor'?localizeLabel("检查监控任务"):localizeLabel("检查发送内容")}</button>{state?.busy&&<button type="button" className="button secondary" disabled={busy} onClick={()=>void action('cancel')}><LocalizedLabel text={"取消"}/></button>}<button type="button" className="button secondary" disabled={blocked||state?.monitoring} onClick={()=>void action('clear')}><LocalizedLabel text={"新对话"}/></button></div>
      <div className="assistant-footnote">{state?.model||localizeLabel("全局 AI 接口")}{state?.status?` · ${state.status.split(' / ')[0]}`:''}</div>
    </footer>
  </aside>;
}
