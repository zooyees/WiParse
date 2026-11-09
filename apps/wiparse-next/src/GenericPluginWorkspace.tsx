import {pluginExecutionStatus} from './pluginExecutionStatus';
import {ExecutionLogPanel} from './ExecutionLogPanel';
import {LocalizedLabel,localizeLabel} from './localization';
import {useEffect,useId,useRef,useState} from 'react';
import type {Dispatch,SetStateAction} from 'react';
import {genericPluginRun,genericPluginStop,genericSelectPath,readLegacy} from './bridge';
import {object,string} from './model';

type Field={name:string;type:string;label:string;help:string;group:string;advanced:boolean;filterKind:string;filterFrom:string;options:{value:string;label:string}[]};
type Device={id:string;kind:string;label:string};
function kindName(value:string){const key=value.toLowerCase().replaceAll('-','_');return ({scope:'oscilloscope',osc:'oscilloscope',psu:'dc_source',dcsource:'dc_source',source:'dc_source',power:'dc_source',load:'electronic_load',eload:'electronic_load',dmm:'multimeter',meter:'multimeter',probe:'debug_probe',jlink:'debug_probe',stlink:'debug_probe',cmsis_dap:'debug_probe',dap:'debug_probe',bridge:'usb_bridge',ft4222:'usb_bridge',dongle:'wiparse_dongle',wiparse:'wiparse_dongle'} as Record<string,string>)[key]||key;}

export function GenericPluginWorkspace({snapshot,plugin,connected,setOwner,onUpdated}:{snapshot:unknown;plugin:string;connected:boolean;setOwner:Dispatch<SetStateAction<string|null>>;onUpdated:()=>void}){
  const state=object(snapshot),params=object(state.params),running=state.running===true,active=string(state.selected)===plugin;
  const fields:Field[]=(Array.isArray(state.generic_params)?state.generic_params:[]).map(value=>{
    const item=object(value);
    return {name:string(item.name),type:string(item.type).toLowerCase(),label:string(item.label)||string(item.name),help:string(item.help),group:string(item.group),advanced:item.advanced===true,filterKind:string(item.filter_kind),filterFrom:string(item.filter_from),options:(Array.isArray(item.options)?item.options:[]).map(value=>{const option=object(value);return {value:string(option.value),label:string(option.label)||string(option.value)};})};
  }).filter(field=>!!field.name);
  const [baseline,setBaseline]=useState(()=>({...params}));
  const [draft,setDraft]=useState<Record<string,string>>(()=>Object.fromEntries(fields.map(field=>[field.name,string(params[field.name])])));
  const editId=useId(),dirty=fields.some(field=>(draft[field.name]??'')!==string(baseline[field.name]));
  useEffect(()=>{if(!dirty)return;setOwner(current=>current??editId);return()=>setOwner(current=>current===editId?null:current);},[dirty,editId,setOwner]);
  const [busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState(''),[acceptedHere,setAcceptedHere]=useState(false);
  const [devices,setDevices]=useState<Device[]>([]),[ports,setPorts]=useState<string[]>([]),[inventoryError,setInventoryError]=useState(''),[inventoryVersion,setInventoryVersion]=useState(0);
  const mounted=useRef(true);
  useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;};},[]);
  const needsDevices=fields.some(field=>field.type==='device'),needsPorts=fields.some(field=>field.type==='serial'||field.type==='serial_port');
  useEffect(()=>{
    if(!active||(!needsDevices&&!needsPorts))return;
    let current=true;setInventoryError('');
    if(needsDevices)void readLegacy('instrument.list').then(value=>{if(!current)return;const list=object(value).devices;setDevices((Array.isArray(list)?list:[]).flatMap(raw=>{const row=object(raw),id=row.device_id,resource=string(row.resource),model=string(object(row.identity).model);return typeof id==='number'&&Number.isSafeInteger(id)?[{id:String(id),kind:string(row.kind),label:`${model||'仪器'} · ${resource||id}`}]:[];}));}).catch(()=>{if(current)setInventoryError('读取仪器列表失败，请刷新设备。');});
    if(needsPorts)void readLegacy('serial.ports').then(value=>{if(current)setPorts((Array.isArray(value)?value:[]).map(row=>string(object(row).device)).filter(Boolean));}).catch(()=>{if(current)setInventoryError('读取串口列表失败，请刷新设备。');});
    return()=>{current=false;};
  },[active,needsDevices,needsPorts,inventoryVersion]);
  const available=connected&&active&&state.generic_workspace_version===1&&!!string(state.project)&&!!string(state.data_root);
  const execute=async(lifecycle:'preflight'|'run')=>{
    setBusy(true);setError('');setNotice('');
    try{
      const result=object(await genericPluginRun({project:string(state.project),plugin,dataRoot:string(state.data_root),expected:baseline,values:draft,lifecycle}));
      if(!mounted.current)return;
      setAcceptedHere(true);const saved=object(result.params);setBaseline({...saved});setDraft(Object.fromEntries(fields.map(field=>[field.name,string(saved[field.name])])));setNotice(lifecycle==='preflight'?localizeLabel('预检请求已接受，执行记录见下方日志。'):localizeLabel('运行请求已接受，执行记录见下方日志。'));onUpdated();
    }catch(reason){if(mounted.current){const message=String(reason);setError(message.includes('CONFIG.STALE')?localizeLabel("插件参数已变化，请先放弃修改，再刷新并重新填写。"):message.includes('CONFIG.CONTEXT_CHANGED')?localizeLabel("工程或插件已变化，请重新选择。"):message.includes('INSTRUMENT.SESSION_CHANGED')?localizeLabel("所选仪器连接已变化，请刷新设备后重新选择。"):message.includes('RUN.STATION_BUSY')?localizeLabel("其他测试正在运行。"):/timeout|timed out|未确认/i.test(message)?localizeLabel("操作结果未确认，请刷新状态后检查，不会自动重试。"):message);}}finally{if(mounted.current)setBusy(false);}
  };
  const stop=async()=>{setBusy(true);setError('');try{await genericPluginStop(plugin,string(state.running_token));if(mounted.current){setNotice(localizeLabel("停止请求已提交，请等待任务退出。"));onUpdated();}}catch(reason){if(mounted.current)setError(`停止结果未确认，请刷新状态：${String(reason)}`);}finally{if(mounted.current)setBusy(false);}};
  const choosePath=async(field:Field)=>{setBusy(true);setError('');try{const directory=['dir','directory','folder'].includes(field.type)||field.name.endsWith('_dir')||field.name.endsWith('_root');const path=await genericSelectPath(directory);if(mounted.current&&path)setDraft(previous=>({...previous,[field.name]:path}));}catch(reason){if(mounted.current)setError(`无法选择路径：${String(reason)}`);}finally{if(mounted.current)setBusy(false);}};
  const renderField=(field:Field)=>{
    const value=draft[field.name]??'',disabled=busy||running||!available;
    const filter=(draft[field.filterFrom]||field.filterKind).split(/[,|]/).map(item=>kindName(item.trim())).filter(Boolean),choices=devices.filter(device=>!filter.length||filter.includes(kindName(device.kind)));
    return <label key={field.name} className="generic-field" title={field.help||undefined}><span>{field.label}</span>{field.type==='boolean'||field.type==='bool'?<select value={value} disabled={disabled} onChange={event=>setDraft({...draft,[field.name]:event.target.value})}><option value="true"><LocalizedLabel text={"是"}/></option><option value="false"><LocalizedLabel text={"否"}/></option></select>:field.type==='enum'?<select value={value} disabled={disabled} onChange={event=>setDraft({...draft,[field.name]:event.target.value})}>{field.options.map(option=><option key={option.value} value={option.value}>{option.label}</option>)}</select>:field.type==='device'?<select value={value} disabled={disabled} onChange={event=>setDraft({...draft,[field.name]:event.target.value})}><option value=""><LocalizedLabel text={"自动选择"}/></option>{value&&!choices.some(device=>device.id===value)&&<option value={value}><LocalizedLabel text={"当前设备不可用（"}/>{value}）</option>}{choices.map(device=><option key={device.id} value={device.id}>{device.label}</option>)}</select>:field.type==='serial'||field.type==='serial_port'?<><input list={`generic-port-${field.name}`} value={value} disabled={disabled} onChange={event=>setDraft({...draft,[field.name]:event.target.value})}/><datalist id={`generic-port-${field.name}`}>{ports.map(port=><option key={port} value={port}/>)}</datalist></>:field.type==='json'||field.type==='text'?<textarea value={value} disabled={disabled} onChange={event=>setDraft({...draft,[field.name]:event.target.value})}/>:['path','dir','directory','folder','file'].includes(field.type)||field.name.endsWith('_dir')||field.name.endsWith('_path')?<div className="generic-path"><input value={value} disabled={disabled} onChange={event=>setDraft({...draft,[field.name]:event.target.value})}/><button type="button" className="button secondary" disabled={disabled} onClick={()=>void choosePath(field)}><LocalizedLabel text={"选择"}/></button></div>:<input value={value} disabled={disabled} onChange={event=>setDraft({...draft,[field.name]:event.target.value})}/>} {/[\u3400-\u9fff]/.test(field.help)&&<small>{field.help}</small>}</label>;
  };
  const renderGroups=(items:Field[])=>{
    const groups=new Map<string,Field[]>();
    for(const field of items){const group=groups.get(field.group)||[];group.push(field);groups.set(field.group,group);}
    return [...groups].map(([name,group])=><div className="generic-group" key={name}>{name&&<h3>{name}</h3>}<div className="generic-fields">{group.map(renderField)}</div></div>);
  };
  return <section className="card matrix generic-plugin"><div className="section-heading"><div><h2>{string((Array.isArray(state.plugins)?state.plugins:[]).map(object).find(item=>item.id===plugin)?.name_zh)||plugin}</h2><p><LocalizedLabel text={"插件配置与任务"}/></p></div><div className="item-actions">{dirty&&<button type="button" className="button secondary" disabled={busy||running} onClick={()=>{setDraft(Object.fromEntries(fields.map(field=>[field.name,string(baseline[field.name])])));setError('');}}><LocalizedLabel text={"放弃修改"}/></button>}<button type="button" className="button secondary" disabled={!available||busy||running} onClick={()=>void execute('preflight')}><LocalizedLabel text={"预检"}/></button><button type="button" className="calculator-submit" disabled={!available||busy||running} onClick={()=>void execute('run')}><LocalizedLabel text={"运行"}/></button>{running&&state.running_plugin===plugin&&<button type="button" className="button secondary" disabled={!connected||busy||!string(state.running_token)} onClick={()=>void stop()}><LocalizedLabel text={"停止"}/></button>}</div></div>
    {!active?<div className="detail-help"><LocalizedLabel text={"请选择并激活此插件。"}/></div>:<>{(needsDevices||needsPorts)&&<div className="item-actions"><button type="button" className="button secondary" disabled={!connected||busy||running} onClick={()=>setInventoryVersion(version=>version+1)}><LocalizedLabel text={"刷新设备"}/></button>{inventoryError&&<span className="detail-warning">{inventoryError}</span>}</div>}{renderGroups(fields.filter(field=>!field.advanced))}{fields.some(field=>field.advanced)&&<details className="generic-advanced"><summary><LocalizedLabel text={"高级设置"}/></summary>{renderGroups(fields.filter(field=>field.advanced))}</details>}{!fields.length&&<div className="detail-help"><LocalizedLabel text={"此插件没有可编辑参数。"}/></div>}<div className="detail-help">{localizeLabel(pluginExecutionStatus(state,string(state.project),plugin,acceptedHere))}<LocalizedLabel text={"。预检和运行由插件执行。"}/></div>{notice&&<div className="detail-help" role="status">{notice}</div>}{error&&<div className="detail-warning" role="alert">{error}</div>}<ExecutionLogPanel key={`${string(state.project)}:${plugin}`} project={string(state.project)} plugin={plugin}/></>}
  </section>;
}
