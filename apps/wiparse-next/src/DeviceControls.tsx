import {ProbeFirmwarePanel} from './ProbeFirmwarePanel';
import {LocalizedLabel,localizeLabel} from './localization';
import {useEffect,useRef,useState,type ReactNode} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {readLegacy} from './bridge';
import {object,string} from './model';
import {useDeviceInterlock} from './useDeviceInterlock';

type Field={key:string;label:string;type?:'number'|'boolean'|'optional_boolean';options?:string[];value?:string;optional?:boolean};
type Form={title:string;command:string;fields?:Field[];scalar?:boolean};
const number=(key:string,label:string,value?:string):Field=>({key,label,type:'number',value});
const text=(key:string,label:string,value?:string):Field=>({key,label,value});
const choice=(key:string,label:string,options:string[]):Field=>({key,label,options,value:options[0]});
const flag=(key:string,label:string):Field=>({key,label,type:'boolean',value:'false'});
const optionalFlag=(key:string,label:string):Field=>({key,label,type:'optional_boolean',value:'',optional:true});

function CommandForm({form,blocked,send}:{form:Form;blocked:boolean;send:(command:unknown,label:string)=>Promise<void>}) {
  const [values,setValues]=useState<Record<string,string>>(()=>Object.fromEntries((form.fields??[]).map(field=>[field.key,field.value??''])));
  const ready=(form.fields??[]).every(field=>field.optional||field.type==='boolean'||!!values[field.key]?.trim());
  const submit=()=>{
    const fields=(form.fields??[]).filter(field=>!field.optional||!!values[field.key]?.trim());
    const payload=Object.fromEntries(fields.map(field=>[field.key,field.type==='number'?Number(values[field.key]):field.type==='boolean'||field.type==='optional_boolean'?values[field.key]==='true':values[field.key]]));
    if(fields.some(field=>field.type==='number'&&!Number.isFinite(payload[field.key])))return;
    void send(!form.fields?.length?form.command:{[form.command]:form.scalar?payload[form.fields[0].key]:payload},form.title);
  };
  return <section className="manual-control-card"><h3>{localizeLabel(form.title)}</h3><div className="manual-control-fields">{form.fields?.map(field=><label key={field.key}>{localizeLabel(field.label)}{field.options?<select value={values[field.key]} disabled={blocked} onChange={event=>setValues({...values,[field.key]:event.target.value})}>{field.options.map(value=><option key={value}>{value}</option>)}</select>:field.type==='boolean'||field.type==='optional_boolean'?<select value={values[field.key]} disabled={blocked} onChange={event=>setValues({...values,[field.key]:event.target.value})}>{field.type==='optional_boolean'&&<option value=""><LocalizedLabel text={"保持 / 查询"}/></option>}<option value="false"><LocalizedLabel text={field.key==='dir'?'输入':field.key==='value'?'低电平':'关闭'}/></option><option value="true"><LocalizedLabel text={field.key==='dir'?'输出':field.key==='value'?'高电平':'开启'}/></option></select>:<input aria-label={`${form.title} ${field.label}`} inputMode={field.type==='number'?'decimal':undefined} value={values[field.key]} disabled={blocked} onChange={event=>setValues({...values,[field.key]:event.target.value})}/>}</label>)}</div><button type="button" className="button secondary" disabled={blocked||!ready} onClick={submit}>{form.fields?.length?localizeLabel("应用"):localizeLabel("执行")}</button></section>;
}

function ScopePanel({channels,blocked,send,preview}:{channels:number;blocked:boolean;send:(command:unknown,label:string)=>Promise<void>;preview:ReactNode}) {
  const [drafts,setDrafts]=useState<Record<string,string>>({});
  const set=(key:string,value:string)=>setDrafts(previous=>({...previous,[key]:value}));
  const numeric=(channel:number,key:string,label:string,command:string,field:string)=>{
    const id=`${channel}:${key}`,value=drafts[id]??'';
    return <label className="scope-channel-field">{localizeLabel(label)}<span><input aria-label={`CH${channel} ${label}`} inputMode="decimal" value={value} disabled={blocked} onChange={event=>set(id,event.target.value)}/><button type="button" className="button secondary" disabled={blocked||!value.trim()||!Number.isFinite(Number(value))} onClick={()=>void send({[command]:{channel,[field]:Number(value)}},`CH${channel} ${label}`)}><LocalizedLabel text={"设置"}/></button></span></label>;
  };
  return <div className="scope-controls-workbench"><div className="scope-control-column">
    <div className="scope-main-actions">{[['ScopeRun','运行'],['ScopeStop','停止'],['ScopeSingle','单次'],['ScopeAutoset','自动设置']].map(([command,label])=><button key={command} type="button" className="button secondary" disabled={blocked} onClick={()=>void send(command,label)}>{label}</button>)}</div>
    <div className="scope-channel-grid">{Array.from({length:channels},(_,index)=>{const channel=index+1,key=`${channel}:measure`,measure=drafts[key]??'Frequency';return <section className="manual-control-card scope-channel-card" key={channel}><h3>CH{channel}</h3><div className="assistant-buttons"><button type="button" className="button secondary" disabled={blocked} onClick={()=>void send({ScopeChannel:{channel,enabled:true}},`CH${channel} 显示`)}><LocalizedLabel text={"显示"}/></button><button type="button" className="button secondary" disabled={blocked} onClick={()=>void send({ScopeChannel:{channel,enabled:false}},`CH${channel} 隐藏`)}><LocalizedLabel text={"隐藏"}/></button></div>{numeric(channel,'scale','垂直刻度 V/div','ScopeScale','volts_per_div')}{numeric(channel,'position','垂直位置 div','ScopePosition','divisions')}{numeric(channel,'offset','偏移 V','ScopeOffset','volts')}<label><LocalizedLabel text={"测量"}/><select value={measure} disabled={blocked} onChange={event=>set(key,event.target.value)}>{['Frequency','Period','Pk2Pk','Amplitude','Maximum','Minimum','Mean','Rms','Rise','Fall','PosWidth','NegWidth','DutyCycle'].map(value=><option key={value}>{value}</option>)}</select></label><button type="button" className="button secondary" disabled={blocked} onClick={()=>void send({ScopeMeasure:{channel,meas_type:measure}},`CH${channel} ${measure}`)}><LocalizedLabel text={"读取测量"}/></button></section>;})}</div>
    <section className="manual-control-card"><h3><LocalizedLabel text={"时基与触发"}/></h3><div className="manual-control-grid"><CommandForm form={{title:'水平时基',command:'ScopeTimebase',scalar:true,fields:[number('value','秒 / div')]}} blocked={blocked} send={send}/><CommandForm form={{title:'边沿触发',command:'ScopeTrigger',fields:[choice('source','触发源',Array.from({length:channels},(_,i)=>`CH${i+1}`)),number('level','电平 V'),choice('slope','边沿',['POS','NEG'])]}} blocked={blocked} send={send}/></div></section>
  </div><section className="manual-control-card scope-preview"><h3><LocalizedLabel text={"屏幕预览与源波形"}/></h3>{preview}</section></div>;
}

function formsFor(kind:string,caps:Record<string,unknown>):Form[] {
  if(kind==='multimeter'){
    const functions=['DcVoltage','AcVoltage','DcCurrent','AcCurrent','Resistance','Frequency'];
    const forms:Form[]=[{title:'测量功能',command:'DmmFunction',scalar:true,fields:[choice('function','功能',functions)]}];
    if(caps.range_control===true)forms.push({title:'自动量程',command:'DmmAutoRange',fields:[choice('function','功能',functions),flag('enabled','自动量程')]},{title:'手动量程',command:'DmmRange',fields:[choice('function','功能',functions),number('value','量程')]});
    if(caps.nplc_control===true)forms.push({title:'积分周期',command:'DmmNplc',fields:[choice('function','功能',functions),number('value','NPLC')]});
    return forms;
  }
  if(kind==='debug_probe')return [
    {title:'探针状态',command:'ProbeStatus'},{title:'连接目标',command:'ProbeAttach',fields:[{...text('target','目标芯片（可选）'),optional:true}]},{title:'读取寄存器',command:'ProbeRegs'},
    {title:'暂停目标',command:'ProbeHalt'},{title:'运行目标',command:'ProbeRun'},{title:'复位目标',command:'ProbeReset',fields:[flag('hardware','硬件复位')]},
    {title:'读取内存',command:'ProbeMemRead',fields:[text('address','地址'),number('len','字节数','64')]},
    {title:'写入内存',command:'ProbeMemWrite',fields:[text('address','地址'),text('data_hex','十六进制数据')]},
    {title:'调试时钟',command:'ProbeSpeed',fields:[number('khz','kHz')]},
    {title:'启动 RTT',command:'ProbeRttStart',fields:[number('up_channel','上行通道','0')]},{title:'读取 RTT',command:'ProbeRttRead'},{title:'停止 RTT',command:'ProbeRttStop'},
  ];
  if(kind==='usb_bridge')return [
    {title:'桥接器信息',command:'BridgeInfo'},
    {title:'I²C 传输',command:'BridgeI2c',fields:[text('addr','地址'),{...text('write_hex','写数据（十六进制）'),optional:true},number('read_len','读取字节数','1'),number('clock_hz','时钟 Hz','100000')]},
    {title:'SPI 传输',command:'BridgeSpi',fields:[number('mode','模式 0–3','0'),number('clock_hz','时钟 Hz','1000000'),number('cs','片选','0'),{...text('write_hex','写数据（十六进制）'),optional:true},number('read_len','读取字节数','1')]},
    {title:'GPIO',command:'BridgeGpio',fields:[number('pin','引脚 0–3'),optionalFlag('dir','方向（保持时不配置）'),optionalFlag('value','电平（保持时只读）')]},
  ];
  if(kind==='wiparse_dongle')return [
    {title:'Dongle 信息',command:'DongleInfo'},
    {title:'网络端口',command:'DongleSetLink',fields:[text('host','主机'),number('uart_port','UART 端口'),number('i2c_port','I²C 端口')]},
    {title:'I²C 读取',command:'DongleI2cRead',fields:[text('addr','地址'),number('len','字节数','1')]},
    {title:'I²C 写入',command:'DongleI2cWrite',fields:[text('addr','地址'),text('data_hex','十六进制数据')]},
    {title:'I²C 写后读',command:'DongleI2cWriteRead',fields:[text('addr','地址'),text('data_hex','十六进制数据'),number('len','字节数','1')]},
    {title:'I²C 扫描',command:'DongleI2cScan'},
    {title:'I²C 文本命令',command:'DongleI2cLine',fields:[text('line','命令')]},
    {title:'I²C 探测地址',command:'DongleI2cPing',fields:[text('addr','地址')]},
    {title:'I²C 时钟',command:'DongleI2cSpeed',fields:[{...number('hz','Hz（留空查询）'),optional:true}]},{title:'I²C 超时',command:'DongleI2cTimeout',fields:[{...number('ms','毫秒（留空查询）'),optional:true}]},
    {title:'I²C 总线恢复',command:'DongleI2cRecover'},
    {title:'打开 UART',command:'DongleUartOpen',fields:[{...text('host','主机（可选）'),optional:true},{...number('port','TCP 端口（可选）'),optional:true},number('baud','波特率','115200'),{...text('com','COM 端口（可选）'),optional:true},{...flag('send_baud_cmd','同步波特率'),value:'true'}]},
    {title:'UART 发送',command:'DongleUartWrite',fields:[text('data','数据'),flag('hex','十六进制')]},{title:'读取 UART',command:'DongleUartRead'},{title:'关闭 UART',command:'DongleUartClose'},
  ];
  return [];
}

export function DeviceControls({device,disabled,supported,preview}:{device:unknown;disabled:boolean;supported:boolean;preview?:ReactNode}) {
  const item=object(device),caps=object(item.capabilities),id=item.device_id,resource=string(item.resource),kind=string(item.kind);
  const [busy,setBusy]=useState(false),[job,setJob]=useState<number|null>(null),[feedback,setFeedback]=useState(''),[response,setResponse]=useState(''),[raw,setRaw]=useState(''),[resetPending,setResetPending]=useState(false);
  const {uncertain,setUncertain,begin,finish,acknowledge}=useDeviceInterlock('manual',resource);
  const operation=useRef<string|null>(null),label=useRef('');
  useEffect(()=>{if(job===null)return;let active=true,timer:ReturnType<typeof setTimeout>;const started=Date.now();const poll=async()=>{
    try{
      if(Date.now()-started>60000)throw Error('任务超过等待时限，设备状态未确认');
      const result=object(await readLegacy('instrument.job.get',{job_id:job}));if(!active)return;
      if(result.state==='pending_or_unknown'){timer=setTimeout(poll,500);return;}
      if(result.state!=='completed'||result.job_id!==job||result.device_id!==id)throw Error('任务与当前设备不匹配');
      if(result.ok!==true)throw Error(string(object(result.detail).error,'设备执行失败'));
      setResponse(string(object(result.detail).response));setFeedback(`${label.current}已完成`);
      if(!operation.current||!finish(operation.current))throw Error('设备已完成，但恢复标记未能清除');
      setJob(null);
    }catch(reason){if(active){setUncertain(true);setFeedback(`结果未确认：${String(reason)}。请核对设备后继续。`);setJob(null);}}
  };void poll();return()=>{active=false;clearTimeout(timer);};},[job,id]);
  const send=async(command:unknown,title:string)=>{
    if(typeof id!=='number'||disabled||busy||job!==null||uncertain)return;
    setBusy(true);setFeedback('');setResponse('');let token:string|null=null;
    try{token=begin();operation.current=token;label.current=title;const result=object(await invoke('instrument_control_safe',{deviceId:id,resource,command}));if(typeof result.job_id!=='number')throw Error('任务编号未确认');setJob(result.job_id);setFeedback(`${title}已提交，等待设备完成`);}
    catch(reason){const message=String(reason);if(token&&/INSTRUMENT\.(?:BUSY|SESSION_CHANGED|INVALID_REQUEST|INVALID_VALUE|INVALID_CHANNEL|UNSUPPORTED|TEST_IN_PROGRESS)/.test(message)){finish(token);setFeedback(message);}else{setUncertain(true);setFeedback(`结果未确认：${message}`);}}
    finally{setBusy(false);}
  };
  const blocked=disabled||!supported||busy||job!==null||uncertain||item.disconnecting===true||item.source_action_pending===true||item.scope_action_pending===true;
  return <section className="typed-device-controls">{kind==='debug_probe'&&<ProbeFirmwarePanel device={device} blocked={blocked} status={response}/>} 
    {kind==='oscilloscope'?<ScopePanel channels={Math.min(8,Math.max(1,Number(caps.channels)||1))} blocked={blocked} send={send} preview={preview}/>:<div className="manual-control-grid">{formsFor(kind,caps).map(form=><CommandForm key={form.command} form={form} blocked={blocked} send={send}/>)}</div>}
    {['oscilloscope','multimeter','dc_source','electronic_load','generic'].includes(kind)&&<section className="manual-control-card manual-console"><h3><LocalizedLabel text={"SCPI 控制台"}/></h3><label><LocalizedLabel text={"命令"}/><input aria-label={localizeLabel("SCPI 命令")} maxLength={4096} value={raw} disabled={blocked} onChange={event=>setRaw(event.target.value)}/></label><div className="assistant-buttons"><button type="button" className="button secondary" disabled={blocked||!raw.trim()} onClick={()=>void send({RawQuery:raw},'SCPI 查询')}><LocalizedLabel text={"查询"}/></button><button type="button" className="button secondary" disabled={blocked||!raw.trim()} onClick={()=>void send({RawWrite:raw},'SCPI 发送')}><LocalizedLabel text={"发送"}/></button><button type="button" className="button secondary" disabled={blocked} onClick={()=>void send('Clear','清除设备状态')}><LocalizedLabel text={"清除状态"}/></button><button type="button" className="button secondary" disabled={blocked} onClick={()=>setResetPending(value=>!value)}><LocalizedLabel text={"设备复位…"}/></button></div>{resetPending&&<div className="detail-warning"><p><LocalizedLabel text={"复位会改变设备当前设置，输出行为由设备型号决定。请确认当前工况允许复位。"}/></p><div className="assistant-buttons"><button type="button" className="button secondary" disabled={blocked} onClick={()=>{setResetPending(false);void send('Reset','设备复位');}}><LocalizedLabel text={"确认复位"}/></button><button type="button" className="button secondary" onClick={()=>setResetPending(false)}><LocalizedLabel text={"取消"}/></button></div></div>}</section>}
    {feedback&&<div className={uncertain?'detail-warning':'detail-help'} role="status">{feedback}</div>}
    {response&&<pre className="instrument-response">{response}</pre>}
    {uncertain&&<button type="button" className="button secondary" disabled={busy||job!==null||item.source_action_pending===true} onClick={()=>{if(acknowledge())setFeedback(localizeLabel("已核对设备状态，可继续操作。"));}}><LocalizedLabel text={"已核对设备状态"}/></button>}
  </section>;
}
