import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {LocalizedLabel,localizeLabel} from './localization';
import {readLegacy} from './bridge';
import {object,string} from './model';
import {engineeringPrompt} from './engineeringPrompt';

type Turn={role:'user'|'assistant';text:string};
/** Explicit user text only: no implicit logs, files, credentials or device commands. */
export function EngineeringAssistant({onEditingChange}:{onEditingChange:(editing:boolean)=>void}){
  const [settings,setSettings]=useState<Record<string,unknown>|null>(null),[text,setText]=useState(''),[context,setContext]=useState('');
  const [turns,setTurns]=useState<Turn[]>([]),[history,setHistory]=useState(false),[preview,setPreview]=useState(''),[token,setToken]=useState('');
  const [pollAttempt,setPollAttempt]=useState(0),[busy,setBusy]=useState(false),[error,setError]=useState(''),[status,setStatus]=useState('');
  const bottom=useRef<HTMLDivElement>(null),mounted=useRef(true);
  useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;};},[]);
  useEffect(()=>{onEditingChange(busy||!!token||!!preview||!!text||!!context);return()=>onEditingChange(false);},[busy,token,preview,text,context,onEditingChange]);
  const load=async()=>{setBusy(true);setError('');try{const value=object(await readLegacy('ui.ai.settings.get_safe'));if(mounted.current)setSettings(value);}catch(reason){if(mounted.current)setError(String(reason));}finally{if(mounted.current)setBusy(false);}};
  useEffect(()=>{void load();},[]);
  useEffect(()=>{bottom.current?.scrollIntoView({block:'nearest'});},[turns,status]);
  const fields=object(settings),enabled=fields.enabled===true,locked=busy||!!token;
  const inspect=()=>{
    let value:string;try{value=engineeringPrompt(text,context,history?turns:[]);}catch(reason){setError(String(reason));return;}
    setError('');setPreview(value);
  };
  const submit=async()=>{
    if(!preview||!settings||locked)return;setBusy(true);setError('');setStatus('');
    try{const response=object(await invoke('ai_chat_submit_safe',{revision:string(settings.revision),text:preview,consent:true}));if(!mounted.current)return;
      if(response.accepted!==true||!string(response.token))throw Error('AI.REQUEST_UNCONFIRMED');
      setTurns(previous=>[...previous.slice(-15),{role:'user',text}]);setToken(string(response.token));setText('');setContext('');setPreview('');setStatus(localizeLabel('正在等待 AI 回复…'));
    }catch(reason){if(mounted.current){setError(`${String(reason)}；${localizeLabel('不会自动重发；请核对 AI 服务状态。')}`);setPreview('');}}
    finally{if(mounted.current)setBusy(false);}
  };
  useEffect(()=>{if(!token)return;let active=true,timer:ReturnType<typeof setTimeout>;const started=Date.now();
    const poll=async()=>{if(Date.now()-started>330000){if(active)setError(localizeLabel('AI 结果仍未确认，请重新查询；不会自动重发。'));return;}try{const result=object(await readLegacy('ui.ai.chat.state_safe',{token}));if(!active)return;
      if(result.state==='running'){timer=setTimeout(poll,800);return;}
      if(result.state==='completed'){setTurns(previous=>[...previous.slice(-15),{role:'assistant',text:string(result.text)}]);setStatus(localizeLabel('分析完成'));setError('');setToken('');}
      else if(result.state==='failed'){setError(string(result.message,'AI request failed'));setStatus('');setToken('');}
      else throw Error('AI.RESULT_UNCONFIRMED');
    }catch(reason){if(active){setError(String(reason));if(String(reason).includes('AI.CHAT_NOT_FOUND')){setToken('');setStatus('');}else timer=setTimeout(poll,2500);}}};void poll();return()=>{active=false;clearTimeout(timer);};
  },[token,pollAttempt]);
  return <section className="engineering-assistant">
    <div className="section-heading"><div><h2><LocalizedLabel text="工程分析助手"/></h2><p><LocalizedLabel text="分析问题、日志片段和测试结果；设备自动化请使用串口 AI。"/></p></div><span className="table-count">{string(fields.model)||localizeLabel('未配置模型')}</span></div>
    {!enabled&&<div className="detail-help"><LocalizedLabel text="请先在 AI 服务中配置并启用模型。"/></div>}
    <div className="engineering-conversation" aria-live="polite">{!turns.length?<div className="empty"><strong><LocalizedLabel text="描述需要分析的问题"/></strong><span><LocalizedLabel text="只发送下方确认的文字，不自动附加工程文件或串口数据。"/></span></div>:turns.map((turn,index)=><article className={`engineering-turn role-${turn.role}`} key={index}><header>{turn.role==='user'?localizeLabel('你'):localizeLabel('AI')}<button className="button secondary" onClick={()=>void navigator.clipboard.writeText(turn.text).catch(reason=>setError(String(reason)))}><LocalizedLabel text="复制"/></button></header><div className="report-markdown"><ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml components={{a:({children})=><span>{children}</span>,img:({alt})=><span>{alt}</span>,table:({children})=><div className="report-table"><table>{children}</table></div>}}>{turn.text}</ReactMarkdown></div></article>)}<div ref={bottom}/></div>
    {status&&<p className="detail-help" role="status">{status}</p>}{error&&<div className="detail-warning" role="alert">{error}</div>}
    <div className="engineering-composer"><label><LocalizedLabel text="问题与分析要求"/><textarea value={text} maxLength={24000} disabled={locked||!!preview} onChange={event=>setText(event.target.value)}/></label><details><summary><LocalizedLabel text="补充上下文（可选）"/></summary><textarea aria-label={localizeLabel('补充上下文')} value={context} maxLength={24000} disabled={locked||!!preview} onChange={event=>setContext(event.target.value)}/></details><label className="settings-check"><input type="checkbox" checked={history} disabled={locked||!!preview} onChange={event=>setHistory(event.target.checked)}/><LocalizedLabel text="附加最近对话（最多 8 条）"/></label>
      {preview&&<section className="assistant-preview"><strong><LocalizedLabel text="即将发送的完整内容"/></strong><p>{string(fields.base_url)} · {string(fields.model)}</p><pre>{preview}</pre><div className="assistant-buttons"><button className="button primary" disabled={locked} onClick={()=>void submit()}><LocalizedLabel text="确认发送"/></button><button className="button secondary" disabled={locked} onClick={()=>setPreview('')}><LocalizedLabel text="返回编辑"/></button></div></section>}
      <div className="settings-actions"><button className="button secondary" disabled={locked||(!text&&!context&&!preview)} onClick={()=>{setText('');setContext('');setPreview('');}}><LocalizedLabel text="清空草稿"/></button>{token&&error&&<button className="button secondary" onClick={()=>{setError('');setPollAttempt(value=>value+1);}}><LocalizedLabel text="重新查询结果"/></button>}<button className="button secondary" disabled={locked||!!preview} onClick={()=>void load()}><LocalizedLabel text="重新读取设置"/></button><button className="button primary" disabled={locked||!enabled||!settings||!text.trim()||!!preview||settings.busy===true||settings.editing_elsewhere===true} onClick={inspect}><LocalizedLabel text="预览发送内容"/></button></div>
    </div>
  </section>;
}
