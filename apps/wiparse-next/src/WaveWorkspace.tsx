import {latestTaskQueue} from './latestTaskQueue';
import {readWaveViewport,type ViewportRequest,type ViewportResult} from './waveViewport';
import {ProtocolFrameList} from './ProtocolFrameList';
import {ProtocolTimeline} from './ProtocolTimeline';
import {WaveMeasurements,type WaveMetrics} from './WaveMeasurements';
import {filterFrames,stepFrame,protocolGate,cursorFocusWindow,frameFocusWindow,zoomVertical,reconcileChannels,visibleExtent,type ProtocolFrame} from './waveAnalysis';
import {coordinateReadout} from './plotAxes';
import {useWorkspaceSummary} from './workspaceSummary';
import {OfflineBrowser,type OfflineFile} from './OfflineBrowser';
import {identityMapping,overlayCompatible,stackMappings,validMapping,displayValue,sourceValue,measurementWindow,type ChannelMapping} from './waveMapping';
import {useWorkspaceActive} from './WorkspaceSlot';
import {WavePlot,type View,type Point} from './WavePlot';
import {LocalizedLabel,localizeLabel} from './localization';
import { ResizableWorkspace } from './ResizableWorkspace';
import {useEffect,useRef,useState,useMemo} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {cursorAxisOnSpace,cursorWindow,nextCursorPair,waveWindow,zoomWaveWindow,plotExtent} from './waveCursor';

type Channel={index:number;name:string;points:number;start:number;end:number;x_unit:string;y_unit:string};
type Summary={filename:string;channels:Channel[];files:{id:string;name:string;channels:number[]}[]};
type Range={start:number;end:number};
type ViewSource={summary:Summary|null;channel:number;overlayIds:number[]};
type ViewJob={source:ViewSource;request:ViewportRequest};
type PlotState={view:View|null;overlays:View[];overlayError:string;viewError:string};
const emptyPlot:PlotState={view:null,overlays:[],overlayError:'',viewError:''};
type Metrics=WaveMetrics;
const format=(n:number)=>n.toLocaleString('zh-CN',{maximumSignificantDigits:7});
const desktopOnly=(reason:unknown)=>String(reason).includes("reading 'invoke'");
const DESKTOP_ONLY='打开、测量和解码波形需要在 WiParse 桌面窗口中进行。';


type Frame=ProtocolFrame;

type DecodeView={gate_key?:string;kind:string;info:string;error:string|null;truncated:boolean;frame_count:number;frames:Frame[]};
type Roles={uart_signal?:number;i2c_scl?:number;i2c_sda?:number;spi_clk?:number;spi_mosi?:number;spi_miso?:number;spi_cs?:number;spi_io2?:number;spi_io3?:number;i2s_bclk?:number;i2s_ws?:number;i2s_data?:number;ddsss_signal?:number;charging_signal?:number};

function ChannelField({label,channels,value,allowEmpty,onChange}:{label:string;channels:Channel[];value?:number;allowEmpty?:boolean;onChange:(value:number|undefined)=>void}){
  return <label>{label}<select aria-label={label} value={value??''} onChange={event=>onChange(event.target.value===''?undefined:Number(event.target.value))}><option value="" disabled={!allowEmpty}><LocalizedLabel text={allowEmpty?"无":"请选择通道"}/></option>{channels.map(item=><option key={item.index} value={item.index}>{item.name}</option>)}</select></label>;
}

function ChargingFields({kind,field,manual,threshold,ufcsBaud,scpUi,onManual,onThreshold,onUfcsBaud,onScpUi}:{kind:string;field:(label:string,key:keyof Roles,allowEmpty?:boolean)=>ReturnType<typeof ChannelField>;manual:boolean;threshold:string;ufcsBaud:number;scpUi:string;onManual:(enabled:boolean)=>void;onThreshold:(value:string)=>void;onUfcsBaud:(value:number)=>void;onScpUi:(value:string)=>void}){
  const signal=kind==='usb_pd'?'CC':kind==='scp'?'D-':'D+';
  return <>
    {field(signal,'charging_signal')}
    <label className="wave-bus-check"><input type="checkbox" checked={manual} onChange={event=>onManual(event.target.checked)}/><LocalizedLabel text={"手动阈值"}/></label>
    {manual&&<label><LocalizedLabel text={"阈值 V"}/><input aria-label={localizeLabel("阈值 V")} inputMode="decimal" value={threshold} onChange={event=>onThreshold(event.target.value)}/></label>}
    {kind==='ufcs'&&<label><LocalizedLabel text={"波特率"}/><select aria-label={localizeLabel("UFCS 波特率")} value={ufcsBaud} onChange={event=>onUfcsBaud(Number(event.target.value))}>{[38400,57600,115200,230400].map(value=><option key={value} value={value}>{value}</option>)}</select></label>}
    {kind==='scp'&&<label>UI µs<input aria-label="SCP UI" inputMode="decimal" value={scpUi} onChange={event=>onScpUi(event.target.value)}/></label>}
  </>;
}

type ProtocolLane={query:string;issues:boolean;id:number;revision:number;autoDecode:boolean;failure:string;kind:string;roles:Roles;baud:string;dataBits:number;parity:string;stopBits:number;inverted:boolean;gate:boolean;gateMode:'cursors'|'view';regBits:number;spiMode:number;spiWire:string;spiWord:number;spiMsb:boolean;spiCsLow:boolean;i2sBits:number;i2sFormat:string;ddsssSeq:string;ddsssExt:string;fop:string;ufcsBaud:number;scpUi:string;manualThreshold:boolean;threshold:string;decoded:DecodeView|null;frameIndex:number|null;detail:string[];};
const newLane=(id:number):ProtocolLane=>({id,query:'',issues:false,revision:0,autoDecode:false,failure:'',kind:'off',roles:{},baud:'',dataBits:8,parity:'none',stopBits:1,inverted:false,gate:true,gateMode:'cursors',regBits:8,spiMode:0,spiWire:'4',spiWord:8,spiMsb:true,spiCsLow:true,i2sBits:16,i2sFormat:'philips',ddsssSeq:'seqa',ddsssExt:'off',fop:'',ufcsBaud:115200,scpUi:'160',manualThreshold:false,threshold:'0.6',decoded:null,frameIndex:null,detail:[]});

export function WaveWorkspace(){
  const workspaceActive=useWorkspaceActive();
  const [summary,setSummary]=useState<Summary|null>(null),[channel,setChannel]=useState(0),[range,setRange]=useState<Range|null>(null),[yLimit,setYLimit]=useState<{minimum:number;maximum:number}|null>(null);
  const [overlayIds,setOverlayIds]=useState<number[]>([]);
  const [plot,setPlot]=useState<PlotState>(emptyPlot);const {view,overlays,overlayError}=plot;
  const [busy,setBusy]=useState(false),[error,setError]=useState('');
  const [notice,setNotice]=useState('');
  const [cursors,setCursors]=useState<Point[]>([]),[yCursors,setYCursors]=useState<number[]>([]),[cursorAxis,setCursorAxis]=useState<'x'|'y'>('x');
  const [metrics,setMetrics]=useState<Metrics|null>(null),[metricError,setMetricError]=useState('');
  const [inspectorTab,setInspectorTab]=useState<'channels'|'measure'|'protocol'>('channels');
  const [showInspector,setShowInspector]=useState(false),[showMetrics,setShowMetrics]=useState(false),[metricScope,setMetricScope]=useState<'auto'|'full'|'view'>('auto');
  const [mappings,setMappings]=useState<Record<number,ChannelMapping>>({});
  const [lanes,setLanes]=useState<ProtocolLane[]>(()=>[newLane(0)]),[laneId,setLaneId]=useState(0),[decoding,setDecoding]=useState(false);
  const autoAttempt=useRef(new Map<number,string>());
  const nextLane=useRef(1),previousSummary=useRef<Summary|null>(null);
  const lane=lanes.find(item=>item.id===laneId)??lanes[0];
  const laneSetter=<K extends keyof ProtocolLane>(key:K)=>(value:React.SetStateAction<ProtocolLane[K]>)=>{const parameter=!['decoded','frameIndex','detail','query','issues'].includes(key);if(parameter){decodeRequest.current++;frameRequest.current++;}setLanes(previous=>previous.map(item=>item.id===laneId?{...item,...(parameter?{revision:item.revision+1,decoded:null,frameIndex:null,detail:[],failure:''}:key==='decoded'&&value?{autoDecode:true,failure:''}:{}),[key]:typeof value==='function'?(value as (current:ProtocolLane[K])=>ProtocolLane[K])(item[key]):value}:item));};
  const {query,issues,gateMode,kind,roles,baud,dataBits,parity,stopBits,inverted,gate,regBits,spiMode,spiWire,spiWord,spiMsb,spiCsLow,i2sBits,i2sFormat,ddsssSeq,ddsssExt,fop,ufcsBaud,scpUi,manualThreshold,threshold,decoded,frameIndex,detail}=lane;
  const setQuery=laneSetter('query'),setIssues=laneSetter('issues');
  const openInspector=(tab:typeof inspectorTab)=>{setInspectorTab(tab);setShowInspector(true);if(tab==='measure')setShowMetrics(true);};
  const setKind=laneSetter('kind');
  const setRoles=laneSetter('roles');
  const setBaud=laneSetter('baud');
  const setDataBits=laneSetter('dataBits');
  const setParity=laneSetter('parity');
  const setStopBits=laneSetter('stopBits');
  const setInverted=laneSetter('inverted');
  const setGate=laneSetter('gate');
  const setGateMode=laneSetter('gateMode');
  const setRegBits=laneSetter('regBits');
  const setSpiMode=laneSetter('spiMode');
  const setSpiWire=laneSetter('spiWire');
  const setSpiWord=laneSetter('spiWord');
  const setSpiMsb=laneSetter('spiMsb');
  const setSpiCsLow=laneSetter('spiCsLow');
  const setI2sBits=laneSetter('i2sBits');
  const setI2sFormat=laneSetter('i2sFormat');
  const setDdsssSeq=laneSetter('ddsssSeq');
  const setDdsssExt=laneSetter('ddsssExt');
  const setFop=laneSetter('fop');
  const setUfcsBaud=laneSetter('ufcsBaud');
  const setScpUi=laneSetter('scpUi');
  const setManualThreshold=laneSetter('manualThreshold');
  const setThreshold=laneSetter('threshold');
  const setDecoded=laneSetter('decoded');
  const setFrameIndex=laneSetter('frameIndex');
  const setDetail=laneSetter('detail');
  const [exportFormat,setExportFormat]=useState('csv');
  const [interactionMode,setInteractionMode]=useState<'pan'|'cursor'>('pan');
  const cursorRequest=useRef(0),decodeRequest=useRef(0),frameRequest=useRef(0);
  const fail=(reason:unknown)=>{if(desktopOnly(reason)){setError('');setNotice(DESKTOP_ONLY);}else setError(String(reason));};
  useEffect(()=>{cursorRequest.current++;setCursors([]);setYCursors([]);setYLimit(null);},[summary]);
  useEffect(()=>{
    decodeRequest.current++;frameRequest.current++;
    const prior=previousSummary.current;previousSummary.current=summary;
    const channels=summary?.channels??[];
    const keys=(value:Summary|null)=>{const result=new Map<number,string>();for(const file of value?.files??[])file.channels.forEach((index,ordinal)=>result.set(index,`${file.id}:${ordinal}`));return result;};
    const before=keys(prior),after=keys(summary),byKey=new Map([...after].map(([index,key])=>[key,index]));
    if(prior){setMappings(previous=>Object.fromEntries(Object.entries(previous).flatMap(([index,mapping])=>{const next=byKey.get(before.get(Number(index))??'');return next===undefined?[]:[[next,mapping]];})));}
    if(summary){const visible=reconcileChannels(prior,summary,channel,overlayIds);setChannel(visible.primary);setOverlayIds(visible.overlays);}else setOverlayIds([]);
    const at=(offset:number)=>channels.length?channels[Math.min(offset,channels.length-1)].index:undefined;
    const defaults:Roles={uart_signal:at(0),i2c_scl:at(0),i2c_sda:at(1),spi_clk:at(0),spi_mosi:at(1),spi_miso:channels.length>2?at(2):undefined,spi_cs:channels.length>3?at(3):undefined,spi_io2:channels.length>4?at(4):undefined,spi_io3:channels.length>5?at(5):undefined,i2s_bclk:at(0),i2s_ws:at(1),i2s_data:at(2),ddsss_signal:at(0),charging_signal:at(0)};
    setLanes(previous=>previous.map(item=>({...item,roles:prior?Object.fromEntries(Object.entries(item.roles).map(([role,index])=>[role,index===undefined?undefined:byKey.get(before.get(index)??'')])):defaults,decoded:null,frameIndex:null,detail:[]})));
  },[summary]);
  useEffect(()=>{frameRequest.current++;decodeRequest.current++;},[laneId]);
  useEffect(()=>()=>{cursorRequest.current++;decodeRequest.current++;frameRequest.current++;},[]);
  useEffect(()=>{if(!workspaceActive)return;let active=true;void invoke<Summary|null>('wave_status').then(value=>{if(active)setSummary(previous=>JSON.stringify(previous)===JSON.stringify(value)?previous:value);}).catch(reason=>{if(!active)return;if(desktopOnly(reason)){setError('');setNotice(DESKTOP_ONLY);}else fail(reason);});return()=>{active=false;};},[workspaceActive]);
  const cursorPositions=useRef(cursors);cursorPositions.current=cursors;
  useEffect(()=>{const points=cursorPositions.current;if(!points.length)return;let active=true;const request=++cursorRequest.current;void Promise.allSettled(points.map(point=>invoke<Point>('wave_sample',{channel,x:point.x}))).then(results=>{if(active&&request===cursorRequest.current)setCursors(results.flatMap(result=>result.status==='fulfilled'?[result.value]:[]));});return()=>{active=false;};},[channel]);
  const navigation=useMemo(()=>summary?visibleExtent(summary.channels,[channel,...overlayIds]):null,[summary,channel,overlayIds]);
  const viewSource=useMemo(()=>({summary,channel,overlayIds}),[summary,channel,overlayIds]);
  const activeViewSource=useRef<ViewSource|null>(null);activeViewSource.current=workspaceActive?viewSource:null;
  const viewportQueue=useRef<ReturnType<typeof latestTaskQueue<ViewJob,ViewportResult>>|null>(null);
  useEffect(()=>{
    const queue=latestTaskQueue<ViewJob,ViewportResult>((job,alive)=>readWaveViewport(job.request,input=>invoke<View>('wave_view',input),()=>alive()&&activeViewSource.current===job.source),(result,job)=>{
      if(activeViewSource.current!==job.source)return;
      const warning=result.failures.map(failure=>`${job.source.summary?.channels.find(item=>item.index===failure.channel)?.name??`CH${failure.channel+1}`}（${failure.reason}）`).join('；');
      setPlot({view:result.view,overlays:result.overlays,overlayError:warning?`${localizeLabel('叠加通道读取失败：')}${warning}`:'',viewError:''});
    },(reason,job)=>{if(activeViewSource.current===job.source)setPlot(current=>({...current,viewError:String(reason)}));});
    viewportQueue.current=queue;return()=>{queue.dispose();viewportQueue.current=null;};
  },[]);
  const lastViewSource=useRef<{summary:Summary|null;channel:number}|null>(null);
  useEffect(()=>{
    const queue=viewportQueue.current;queue?.invalidate();
    const previous=lastViewSource.current;lastViewSource.current={summary,channel};
    if(!summary||previous?.summary!==summary||previous?.channel!==channel)setPlot(emptyPlot);
    else setPlot(current=>({...current,overlays:current.overlays.filter(item=>item.channelIndex!==undefined&&overlayIds.includes(item.channelIndex)),overlayError:'',viewError:''}));
    return()=>queue?.invalidate();
  },[viewSource,workspaceActive]);
  useEffect(()=>{
    if(!workspaceActive||!summary||!navigation)return;
    viewportQueue.current?.submit({source:viewSource,request:{channel,overlays:overlayIds,start:range?.start??navigation.start,end:range?.end??navigation.end}});
  },[workspaceActive,viewSource,range,navigation?.start,navigation?.end]);
  const selected=summary?.channels.find(item=>item.index===channel);
  const metricBounds=selected&&view?measurementWindow(metricScope,selected,view,cursors):null;
  useEffect(()=>{if(!workspaceActive||!metricBounds||!showMetrics){if(!showMetrics){setMetrics(null);setMetricError('');}return;}let active=true;setMetrics(null);setMetricError('');const timer=setTimeout(()=>{void invoke<Metrics>('wave_measure',{channel,start:metricBounds.start,end:metricBounds.end}).then(value=>{if(active)setMetrics(value);}).catch(reason=>{if(active)setMetricError(String(reason));});},120);return()=>{active=false;clearTimeout(timer);};},[workspaceActive,metricBounds?.start,metricBounds?.end,channel,showMetrics]);
  const open=async(browserSelection?:OfflineFile)=>{setBusy(true);setError('');setNotice('');try{const value=await invoke<Summary|null>('wave_add_files',browserSelection?{browserSelection,replace:true}:{replace:true});if(value){setRange(null);setYLimit(null);setSummary(value);}}catch(reason){fail(reason);}finally{setBusy(false);}};
  const changeFiles=async(id?:string)=>{setBusy(true);setError('');setNotice('');try{const value=await invoke<Summary|null>(id?'wave_remove_file':'wave_add_files',id?{id}:{});if(value||id){setRange(null);setPlot(emptyPlot);setYLimit(null);setSummary(value);}}catch(reason){fail(reason);}finally{setBusy(false);}};
  const exportCsv=async()=>{setBusy(true);setError('');setNotice('');try{const name=await invoke<string|null>('wave_export_file',{channel,format:exportFormat});if(name)setNotice(`已导出原始采样点：${name}`);}catch(reason){fail(reason);}finally{setBusy(false);}};
  const close=async()=>{setBusy(true);try{await invoke('wave_close');setSummary(null);setPlot(emptyPlot);setRange(null);setYLimit(null);setError('');}catch(reason){fail(reason);}finally{setBusy(false);}};

  const pick=async(x:number)=>{const request=++cursorRequest.current;setError('');try{const point=await invoke<Point>('wave_sample',{channel,x});if(request===cursorRequest.current)setCursors(current=>nextCursorPair(current,point));}catch(reason){if(request===cursorRequest.current)fail(reason);}};
  const moveXCursor=async(index:number,x:number)=>{const request=++cursorRequest.current;try{const point=await invoke<Point>('wave_sample',{channel,x});if(request===cursorRequest.current)setCursors(current=>current.map((existing,i)=>i===index?point:existing));}catch(reason){if(request===cursorRequest.current)fail(reason);}};
  const shiftWindow=(fraction:number)=>{if(!selected||!view)return;setRange(previous=>{const current=previous??{start:view.start,end:view.end},offset=(current.end-current.start)*fraction;return waveWindow(current.start+offset,current.end+offset,(navigation??selected).start,(navigation??selected).end,(navigation??selected).points)??previous;});};
  const boxZoom=(range:Range|null,extent:{minimum:number;maximum:number}|null)=>{if(!selected)return;if(range){const next=waveWindow(range.start,range.end,(navigation??selected).start,(navigation??selected).end,(navigation??selected).points);if(next)setRange(next);}if(extent&&Number.isFinite(extent.minimum)&&Number.isFinite(extent.maximum)&&extent.maximum>extent.minimum)setYLimit(extent);};
  const zoomAt=(fraction:number,factor:number)=>{if(!selected||!view)return;setRange(previous=>zoomWaveWindow(previous??{start:view.start,end:view.end},fraction,factor,navigation??selected)??previous);};
  const zoom=(factor:number)=>zoomAt(.5,factor);
  const zoomY=(fraction:number,factor:number)=>{if(!drawing)return;setYLimit(previous=>{const extent=previous??plotExtent(drawing,drawingOverlays);return extent?zoomVertical(extent,fraction,factor)??previous:previous;});};
  const panY=(minimum:number,maximum:number)=>{if(Number.isFinite(minimum)&&Number.isFinite(maximum)&&Number.isFinite(maximum-minimum)&&maximum>minimum)setYLimit({minimum,maximum});};
  const setXValue=async(index:number,x:number)=>{if(index>cursors.length){setError('请先设置 X1');return;}const request=++cursorRequest.current;setError('');try{const point=await invoke<Point>('wave_sample',{channel,x});if(request===cursorRequest.current)setCursors(current=>{const next=[...current];next[index]=point;return next;});}catch(reason){if(request===cursorRequest.current)fail(reason);}};
  const setYValue=(index:number,value:number)=>{if(index>yCursors.length){setError('请先设置 Y1');return;}const display=displayValue(value,mappingAt(channel));if(!Number.isFinite(display)){setError('光标超出显示数值范围');return;}setYCursors(current=>{const next=[...current];next[index]=display;return next;});};

  const pan=(start:number,end:number)=>{if(!selected)return;const next=waveWindow(start,end,(navigation??selected).start,(navigation??selected).end,(navigation??selected).points);if(next)setRange(next);};
  const fit=()=>{setRange(null);setYLimit(null);};
  const zoomCursors=()=>{if(cursorAxis==='x'){if(!selected)return;const next=cursorFocusWindow(cursors,navigation??selected);if(!next)return;setYLimit(null);setRange(next);return;}if(yCursors.length<2)return;const next=cursorWindow(yCursors[0],yCursors[1]);if(!next)return;setYLimit({minimum:next.start,maximum:next.end});};
  const rangeFor=(item:ProtocolLane)=>protocolGate(item.kind,item.gate,item.gateMode,cursors,view);
  const gateKey=(item:ProtocolLane)=>JSON.stringify(rangeFor(item));
  const validDecoded=decoded?.gate_key===gateKey(lane)?decoded:null;
  const activeFrame=validDecoded?.frames.find(frame=>frame.index===frameIndex)??null;
  useEffect(()=>{if(!view)return;const onKey=(event:KeyboardEvent)=>{const target=event.target;const typing=target instanceof HTMLElement&&(target.isContentEditable||target.tagName==='INPUT'||target.tagName==='TEXTAREA'||target.tagName==='SELECT');if(!typing&&workspaceActive&&event.key==='Escape'){event.preventDefault();setFrameIndex(null);setDetail([]);return;}if(!typing&&workspaceActive&&['ArrowDown','ArrowUp'].includes(event.key)&&validDecoded?.frames.length){event.preventDefault();const next=stepFrame(filterFrames(validDecoded.frames,query,issues),frameIndex,event.key==='ArrowDown'?1:-1);if(next!==null)focusFrame(next);return;}if(!workspaceActive||!cursorAxisOnSpace(event.key,typing,event.repeat))return;event.preventDefault();setCursorAxis(current=>current==='x'?'y':'x');};window.addEventListener('keydown',onKey);return()=>window.removeEventListener('keydown',onKey);},[view,workspaceActive,validDecoded,frameIndex,laneId,query,issues]);
  const setRole=(key:keyof Roles,value?:number)=>setRoles(current=>({...current,[key]:value}));
  const channels=summary?.channels??[];
  const decodeInput=(item:ProtocolLane)=>{const bounds=rangeFor(item);return {lane:item.id,revision:item.revision,kind:item.kind,...item.roles,baud:item.baud.trim()?Number(item.baud):null,data_bits:item.dataBits,parity:item.parity,stop_bits:item.stopBits,inverted:item.inverted,i2c_reg_bits:item.regBits,spi_mode:item.spiMode,spi_wire:item.spiWire,spi_word_bits:item.spiWord,spi_msb_first:item.spiMsb,spi_cs_active_low:item.spiCsLow,i2s_bits:item.i2sBits,i2s_format:item.i2sFormat,ddsss_sequence:item.ddsssSeq,ddsss_extension:item.ddsssExt,fop_hz:item.kind==='ddsss'&&item.fop.trim()?Number(item.fop):null,ufcs_baud:item.ufcsBaud,scp_ui_us:item.kind==='scp'?Number(item.scpUi):null,threshold:item.manualThreshold&&['usb_pd','scp','ufcs'].includes(item.kind)?Number(item.threshold):null,time_start:bounds?.start??null,time_end:bounds?.end??null};};
  const activeAttempt=JSON.stringify([lane.revision,rangeFor(lane)]);
  useEffect(()=>{if(!workspaceActive||!view||decoding||kind==='off'||!lane.autoDecode||decoded?.gate_key===gateKey(lane)||autoAttempt.current.get(laneId)===activeAttempt)return;const timer=setTimeout(()=>{autoAttempt.current.set(laneId,activeAttempt);void runDecode();},250);return()=>clearTimeout(timer);},[workspaceActive,!!view,decoding,kind,lane.autoDecode,decoded?.gate_key,laneId,activeAttempt]);
  const showFrame=async(index:number)=>{const request=++frameRequest.current;setFrameIndex(index);setDetail([]);try{const lines=await invoke<string[]>('wave_explain',{index,lane:laneId});if(request===frameRequest.current)setDetail(lines);}catch(reason){if(request===frameRequest.current)setDetail([String(reason)]);}};
  const runDecode=async()=>{
    if(kind==='off'||!summary)return;
    openInspector('protocol');
    if(baud.trim()&&!Number.isFinite(Number(baud))){setError(localizeLabel("波特率无效"));return;}
    if(gate&&!view){setError(localizeLabel("当前没有可解码的显示范围"));return;}
    const charging=kind==='usb_pd'||kind==='scp'||kind==='ufcs';
    
    if(charging&&manualThreshold){const value=Number(threshold);if(!Number.isFinite(value)||value<-20||value>20){setError(localizeLabel("阈值需在 -20–20 V"));return;}}
    if(kind==='ddsss'&&fop.trim()){const value=Number(fop);if(!Number.isFinite(value)||value<80000||value>2000000){setError(localizeLabel("FOP 需在 80000–2000000 Hz，留空则自动估计"));return;}}
    if(kind==='scp'){const value=Number(scpUi);if(!Number.isFinite(value)||value<120||value>200){setError(localizeLabel("SCP UI 需在 120–200 µs"));return;}}
    const request=++decodeRequest.current;frameRequest.current++;
    setDecoding(true);setError('');setDecoded(null);setFrameIndex(null);setDetail([]);
    try{
      const value=await invoke<DecodeView>('wave_decode',{request:decodeInput(lane)});
      if(request!==decodeRequest.current)return;
      setDecoded({...value,gate_key:gateKey(lane)});autoAttempt.current.set(laneId,activeAttempt);
      if(value.frames[0])await showFrame(value.frames[0].index);
    }catch(reason){if(request===decodeRequest.current)fail(reason);}
    finally{setDecoding(false);}
  };
  const removeLane=async()=>{if(busy||decoding||lanes.length<=1)return;setBusy(true);try{await invoke('wave_lane_remove',{lane:laneId});const remaining=lanes.filter(item=>item.id!==laneId);setLanes(remaining);setLaneId(remaining[0].id);}catch(reason){fail(reason);}finally{setBusy(false);}};
  const protocolReport=async()=>{setBusy(true);setError('');try{const name=await invoke<string|null>('wave_protocol_report',{open:true,focusLane:laneId,focusFrame:frameIndex,lanes:lanes.filter(item=>item.kind!=='off').map(item=>({id:item.id,revision:item.revision,time_start:rangeFor(item)?.start??null,time_end:rangeFor(item)?.end??null}))});if(name)setNotice(`协议报告已保存：${name}`);}catch(reason){fail(reason);}finally{setBusy(false);}};
  const decodeBatch=async(items:ProtocolLane[],requestId:number)=>{for(let position=0;position<items.length;position++){autoAttempt.current.set(items[position].id,JSON.stringify([items[position].revision,rangeFor(items[position])]));
    const item=items[position];if(requestId!==decodeRequest.current)return;setNotice(`正在解析通道 ${position+1} / ${items.length}`);
    try{const result=await invoke<DecodeView>('wave_decode',{request:decodeInput(item)});if(requestId!==decodeRequest.current)return;setLanes(previous=>previous.map(entry=>entry.id===item.id&&entry.revision===item.revision?{...entry,decoded:{...result,gate_key:gateKey(item)},frameIndex:null,detail:[],autoDecode:true,failure:''}:entry));}
    catch(reason){if(requestId!==decodeRequest.current)return;setLanes(previous=>previous.map(entry=>entry.id===item.id?{...entry,decoded:null,failure:String(reason)}:entry));}
  }if(requestId===decodeRequest.current)setNotice('协议解析批次完成；各路结果及错误分别保留。');};
  const decodeAll=async()=>{if(!summary||!view||busy||decoding)return;const items=lanes.filter(item=>item.kind!=='off');if(!items.length)return;setDecoding(true);const request=++decodeRequest.current;frameRequest.current++;try{await decodeBatch(items,request);}finally{setDecoding(false);}};
  const decodeRemaining=async()=>{
    if(!summary||!view||busy||decoding||!['uart','ddsss','usb_pd','scp','ufcs'].includes(kind))return;
    const role:keyof Roles=kind==='uart'?'uart_signal':kind==='ddsss'?'ddsss_signal':'charging_signal';
    const used=new Set(lanes.filter(item=>item.kind===kind).map(item=>item.roles[role]));const remaining=channels.filter(item=>!used.has(item.index)).slice(0,16-lanes.length);
    if(!remaining.length){setNotice('没有未分配的通道。');return;}
    const additions=remaining.map(item=>({...lane,id:nextLane.current++,roles:{...lane.roles,[role]:item.index},decoded:null,frameIndex:null,detail:[]}));
    setLanes(previous=>[...previous,...additions]);setDecoding(true);setError('');const requestId=++decodeRequest.current;
    try{await decodeBatch(additions,requestId);}finally{setDecoding(false);}

  };
  const focusFrame=(index:number)=>{const frame=validDecoded?.frames.find(frame=>frame.index===index);if(!frame)return;void showFrame(index);locateFrame(frame);};
  const locate=()=>{if(activeFrame)locateFrame(activeFrame);};
  const locateFrame=(activeFrame:Frame)=>{if(!selected)return;const bounds=frameFocusWindow(activeFrame,navigation??selected);if(bounds){setRange(bounds);setYLimit(null);}};
  const mappingAt=(index:number)=>mappings[index]??identityMapping;
  const mappedView=(raw:View,index:number):View=>{const mapping=mappingAt(index);return {...raw,y_unit:(Object.values(mappings).some(mapping=>mapping.scale!==1||mapping.offset!==0)||overlays.some(other=>other.y_unit!==view?.y_unit))?'显示坐标':raw.y_unit,minimum:displayValue(raw.minimum,mapping),maximum:displayValue(raw.maximum,mapping),buckets:raw.buckets.map(bucket=>({...bucket,low:displayValue(bucket.low,mapping),high:displayValue(bucket.high,mapping)}))};};
  const plotHandles=useMemo(()=>[channel,...overlayIds].map(index=>({channel:index,label:summary?.channels.find(item=>item.index===index)?.name??`CH${index+1}`,...(mappings[index]??identityMapping)})),[channel,overlayIds,summary,mappings]);
  const drawing=useMemo(()=>view?mappedView(view,channel):null,[view,channel,mappings,overlays]);
  const drawingOverlays=useMemo(()=>overlays.map((raw,index)=>mappedView(raw,raw.channelIndex??overlayIds[index])),[overlays,overlayIds,mappings]);
  const adjustMapping=(index:number,patch:Partial<ChannelMapping>)=>{const next={...mappingAt(index),...patch};if(!validMapping(next)){setError('幅度必须为 1e-12–1e12 的正数，偏移必须是有限数。');return;}const raw=index===channel?view:overlays.find(raw=>raw.channelIndex===index);if(raw&&(!Number.isFinite(displayValue(raw.minimum,next))||!Number.isFinite(displayValue(raw.maximum,next)))){setError('显示映射超出数值范围');return;}if(drawing)setYLimit(previous=>previous??plotExtent(drawing,drawingOverlays));setMappings(previous=>({...previous,[index]:next}));};
  const arrangeDisplays=()=>{if(!view||overlays.length!==overlayIds.length)return;const sources=[{...view,index:channel},...overlays.map((raw,index)=>({...raw,index:raw.channelIndex??overlayIds[index]}))];const next=stackMappings(sources);if(!next){setError('当前范围无法自动排列显示');return;}setMappings(previous=>({...previous,...next}));setYLimit({minimum:-3*(sources.length-1)-1.2,maximum:1.2});};
  useWorkspaceSummary('wave',async()=>{if(!summary||!selected||!metricBounds)throw Error('尚未打开波形。');return invoke('wave_analysis_context',{channel,start:metricBounds.start,end:metricBounds.end,expectedFiles:summary.files.map(file=>file.id),cursors:cursors.map(point=>({x:point.x,y:point.y})),lanes:lanes.filter(item=>item.decoded?.gate_key===gateKey(item)).map(item=>({id:item.id,revision:item.revision,time_start:rangeFor(item)?.start??null,time_end:rangeFor(item)?.end??null}))});});
  const field=(label:string,key:keyof Roles,allowEmpty=false)=><ChannelField label={label} channels={channels} value={roles[key]} allowEmpty={allowEmpty} onChange={value=>setRole(key,value)}/>;
  return <ResizableWorkspace id="wave-layout" className="wave-layout"><section className="card library"><OfflineBrowser scope="wave" disabled={busy||decoding} onOpen={file=>void open(file)}/><div className="section-heading"><h2><LocalizedLabel text={"波形文件"}/></h2></div><div className="wave-actions"><button className="button secondary" type="button" disabled={busy} onClick={()=>void open()}>{busy?localizeLabel("处理中…"):localizeLabel("打开文件")}</button><button className="button secondary" type="button" disabled={busy} onClick={()=>void changeFiles()}><LocalizedLabel text="添加文件"/></button>{summary&&<button className="button secondary" type="button" disabled={busy} onClick={()=>void close()}><LocalizedLabel text="全部关闭"/></button>}</div>{summary&&<><div className="wave-library-note"><LocalizedLabel text="打开替换当前波形库，添加保留已有文件；移除仅关闭内存波形。"/></div><div className="wave-file-list">{summary.files.map((file,fileIndex)=><section className="wave-source-file" key={file.id}><header><strong title={file.name}>{summary.files.length>1?`${fileIndex+1} · ${file.name}`:file.name}</strong><button type="button" className="icon-button" aria-label={`${localizeLabel('移除文件')} ${file.name}`} disabled={busy} onClick={()=>void changeFiles(file.id)}>×</button></header><div className="plugin-list">{summary.channels.filter(item=>file.channels.includes(item.index)).map(item=><div className="wave-channel-row" key={item.index}><button type="button" className={`plugin ${channel===item.index?'selected':''}`} disabled={busy} onClick={()=>{setOverlayIds(previous=>[...new Set([channel,...previous])].filter(index=>index!==item.index&&summary.channels.find(candidate=>candidate.index===index)?.x_unit===item.x_unit));setChannel(item.index);setRange(null);}}><span className="plugin-icon">∿</span><span><strong>{item.name}</strong><small>{item.points.toLocaleString('zh-CN')}<LocalizedLabel text={"点"}/></small></span></button>{channel!==item.index&&<label title={!overlayCompatible(item,selected)?localizeLabel("仅支持相同横轴单位的通道叠加"):localizeLabel("叠加到当前通道")}><input type="checkbox" aria-label={`${localizeLabel("叠加")} ${file.name} · ${item.name}`} checked={overlayIds.includes(item.index)} disabled={busy||!overlayCompatible(item,selected)} onChange={e=>setOverlayIds(previous=>e.target.checked?[...previous,item.index]:previous.filter(id=>id!==item.index))}/><LocalizedLabel text={"叠加"}/></label>}</div>)}</div></section>)}</div></>}</section>
    <section className="card wave-panel"><div className="section-heading wave-toolbar"><div><h2>{selected?.name||localizeLabel("波形分析")}</h2><p>{view?`${view.samples.toLocaleString('zh-CN')} 个采样点 · 横轴 ${view.x_unit||'未标注'} · 纵轴 ${view.y_unit||'未标注'}`:localizeLabel("打开 CSV、TXT、ISF 或 WFM 文件查看原始波形")}</p></div><div className="wave-actions">{(['channels','measure','protocol'] as const).map(tab=><button key={tab} type="button" className="button secondary" aria-pressed={showInspector&&inspectorTab===tab} aria-controls="wave-inspector" onClick={()=>showInspector&&inspectorTab===tab?setShowInspector(false):openInspector(tab)}>{localizeLabel(tab==='channels'?'通道':tab==='measure'?'测量与光标':'协议分析')}</button>)}<div className="wave-mode-switch" role="group" aria-label={localizeLabel('波形交互模式')}><button type="button" className="button secondary" aria-pressed={interactionMode==='pan'} onClick={()=>setInteractionMode('pan')}><LocalizedLabel text="平移"/></button><button type="button" className="button secondary" aria-pressed={interactionMode==='cursor'} onClick={()=>setInteractionMode('cursor')}><LocalizedLabel text="光标"/></button></div><button type="button" className="button secondary" disabled={!selected||busy} onClick={()=>void exportCsv()}><LocalizedLabel text="导出波形"/></button><select aria-label={localizeLabel("波形导出格式")} value={exportFormat} disabled={busy} onChange={event=>setExportFormat(event.target.value)}><option value="csv">CSV</option><option value="isf">ISF · 8 bit</option><option value="wfm">WFM · 16 bit</option></select><button type="button" className="button secondary" disabled={!view||view.start===view.end} onClick={()=>zoom(.5)}>X+</button><button type="button" className="button secondary" disabled={!view||view.start===view.end} onClick={()=>zoom(2)}>X−</button><button type="button" className="button secondary" disabled={!view} title="纵向放大" onClick={()=>zoomY(.5,.5)}>Y+</button><button type="button" className="button secondary" disabled={!view} title="纵向缩小" onClick={()=>zoomY(.5,2)}>Y−</button><button type="button" className="button secondary" disabled={!view} onClick={fit}><LocalizedLabel text={"适应"}/></button><button type="button" className="button secondary" disabled={!view} title={localizeLabel("空格键切换横轴和纵轴游标")} onClick={()=>setCursorAxis(current=>current==='x'?'y':'x')}>{cursorAxis==='x'?localizeLabel("纵轴游标"):localizeLabel("横轴游标")}</button><button type="button" className="button secondary" disabled={!view||(cursorAxis==='x'?cursors.length<2:yCursors.length<2)} onClick={zoomCursors}><LocalizedLabel text={"缩放到光标"}/></button><button type="button" className="button secondary" disabled={cursorAxis==='x'?!cursors.length:!yCursors.length} onClick={()=>{if(cursorAxis==='x'){cursorRequest.current++;setCursors([]);}else setYCursors([]);}}><LocalizedLabel text={"清除游标"}/></button></div></div><div className={`wave-stage ${showInspector?"with-inspector":""}`}><div className="wave-viewport">{(error||plot.viewError)&&<div className="alert" role="alert"><strong><LocalizedLabel text={"波形操作失败"}/></strong><span>{error||plot.viewError}</span></div>}{notice&&<div role="status">{notice}</div>}{overlayError&&<div className="detail-warning" role="alert">{overlayError}</div>}{view&&overlays.length>0&&<div className="wave-legend">{[view,...overlays].map((item,index)=><span key={`${item.channel}-${index}`} title={item.channel}><i style={{background:`var(--wave-series-${index%4})`}}/>{item.channel}</span>)}</div>}{view?<><div className="wave-plot"><WavePlot axisUnit={view.y_unit} handles={plotHandles} onMapping={adjustMapping} onZoomY={zoomY} onPanY={panY} mode={interactionMode} onMoveX={moveXCursor} onMoveY={(index,value)=>setYCursors(current=>current.map((existing,i)=>i===index?value:existing))} onShiftWindow={shiftWindow} onBox={boxZoom} view={drawing??view} overlays={drawingOverlays} cursors={cursors} yCursors={yCursors} axis={cursorAxis} band={activeFrame?{start:activeFrame.t_start,end:activeFrame.t_end}:null} yLimit={yLimit} onZoomAt={zoomAt} onPan={pan} onFit={fit} onPickX={x=>void pick(x)} onPickY={value=>setYCursors(current=>nextCursorPair(current,value))}/></div>{validDecoded&&view.end>view.start&&<ProtocolTimeline lane={laneId} revision={lane.revision} start={view.start} end={view.end} gateStart={rangeFor(lane)?.start??null} gateEnd={rangeFor(lane)?.end??null} selected={frameIndex} onSelect={focusFrame} onLocate={focusFrame}/>}<div className="wave-meta"><span>{metrics?.minimum!=null?localizeLabel("最小值"):localizeLabel("主通道最小值")} <b>{coordinateReadout(metrics?.minimum??view.minimum,(metrics?.maximum??view.maximum)-(metrics?.minimum??view.minimum))} {metrics?.y_unit||view.y_unit}</b></span><span>{metrics?.maximum!=null?localizeLabel("最大值"):localizeLabel("主通道最大值")} <b>{coordinateReadout(metrics?.maximum??view.maximum,(metrics?.maximum??view.maximum)-(metrics?.minimum??view.minimum))} {metrics?.y_unit||view.y_unit}</b></span>{metrics&&<span><LocalizedLabel text={"采样点数"}/><b>{metrics.count.toLocaleString('zh-CN')}</b></span>}{metrics?.sample_interval!=null&&<span><LocalizedLabel text={"采样间隔"}/><b>{format(metrics.sample_interval)} {metrics.x_unit}</b></span>}{metrics?.mean!=null&&<span><LocalizedLabel text={"平均值"}/><b>{format(metrics.mean)} {metrics.y_unit}</b></span>}{metrics?.rms!=null&&<span><LocalizedLabel text={"均方根"}/><b>{format(metrics.rms)} {metrics.y_unit}</b></span>}{metrics?.peak_to_peak!=null&&<span><LocalizedLabel text={"峰峰值"}/><b>{format(metrics.peak_to_peak)} {metrics.y_unit}</b></span>}{metrics?.frequency_hz!=null&&<span><LocalizedLabel text={"频率（估算）"}/><b>{format(metrics.frequency_hz)} Hz</b></span>}{metrics?.period_s!=null&&<span><LocalizedLabel text={"周期（估算）"}/><b>{format(metrics.period_s)} s</b></span>}{metricError&&<span><LocalizedLabel text={"测量不可用："}/>{metricError}</span>}<span><LocalizedLabel text={"显示范围"}/><b>{coordinateReadout(view.start,view.end-view.start)} – {coordinateReadout(view.end,view.end-view.start)} {view.x_unit}</b></span><span>{interactionMode==='pan'?localizeLabel('拖动平移，右键框选缩放；切换光标模式进行测量'):cursorAxis==='y'?localizeLabel("纵轴游标点击图中位置设置 Y1、Y2"):localizeLabel("点击波形设置横轴游标，读数取最近原始采样点")}</span>{cursorAxis==='y'?<>{yCursors.map((value,index)=><span key={index}>{index?'Y2':'Y1'} <b>{coordinateReadout(sourceValue(value,mappingAt(channel)),view.maximum-view.minimum)} {view.y_unit}</b></span>)}{yCursors.length===1&&<span><LocalizedLabel text={"再点击设置 Y2"}/></span>}{yCursors.length===2&&<span>ΔY <b>{format((yCursors[1]-yCursors[0])/mappingAt(channel).scale)} {view.y_unit}</b></span>}{yCursors.length===2&&<span>|ΔY| <b>{format(Math.abs(yCursors[1]-yCursors[0])/mappingAt(channel).scale)} {view.y_unit}</b></span>}</>:<>{cursors.map((point,index)=><span key={index}><LocalizedLabel text={"游标"}/>{index?'B':'A'} <b>{coordinateReadout(point.x,view.end-view.start)} {point.x_unit} / {coordinateReadout(point.y,view.maximum-view.minimum)} {point.y_unit}</b></span>)}{cursors.length===2&&<span>Δ <b>{format(cursors[1].x-cursors[0].x)} {view.x_unit} / {format(cursors[1].y-cursors[0].y)} {view.y_unit}</b></span>}</>}</div></>:<div className="empty"><div className="empty-icon">∿</div><strong>{busy?localizeLabel("正在处理波形"):localizeLabel("尚未打开波形")}</strong><span><LocalizedLabel text={"选择本地波形文件后显示。不会连接仪器。"}/></span></div>}
      </div><aside id="wave-inspector" className="wave-inspector" hidden={!showInspector}><div className="section-heading"><h3><LocalizedLabel text={inspectorTab==='channels'?'通道显示':inspectorTab==='measure'?'测量与光标':'协议分析'}/></h3><button type="button" className="icon-button" aria-label={localizeLabel("收起分析设置")} onClick={()=>setShowInspector(false)}>×</button></div>{view&&<div className="wave-display-controls" hidden={inspectorTab!=='channels'}><button type="button" className="button secondary" disabled={busy||overlays.length!==overlayIds.length} onClick={arrangeDisplays}>自动排列显示</button><button type="button" className="button secondary" onClick={()=>{setMappings({});setYLimit(null);}}>复位全部通道</button><div className="wave-mapping-grid">{[channel,...overlayIds].map(index=>{const mapping=mappingAt(index);return <fieldset key={index}><legend>{summary?.channels.find(item=>item.index===index)?.name} · {summary?.channels.find(item=>item.index===index)?.y_unit||'未标注'}</legend><label>幅度 ×<input type="number" step="any" min="1e-12" max="1e12" value={mapping.scale} onChange={event=>adjustMapping(index,{scale:Number(event.target.value)})}/></label><label>偏移<input type="number" step="any" value={mapping.offset} onChange={event=>adjustMapping(index,{offset:Number(event.target.value)})}/></label><button type="button" className="button secondary" onClick={()=>adjustMapping(index,identityMapping)}>复位显示</button></fieldset>;})}</div><small>仅改变显示；游标、解码、测量和导出使用原始值。</small></div>}
      {view&&<div hidden={inspectorTab!=='measure'}><WaveMeasurements metrics={metrics} error={metricError} scope={metricScope} onScope={setMetricScope} cursors={cursors} yCursors={yCursors} unit={view.x_unit} channels={[channel,...overlayIds].flatMap(index=>{const item=channels.find(item=>item.index===index);return item?[{...item,mapping:mappingAt(index)}]:[];})} onSetX={(index,value)=>void setXValue(index,value)} onSetY={setYValue}/></div>}
      <section className="wave-bus" hidden={inspectorTab!=='protocol'} aria-label={localizeLabel("协议分析")}><div className="protocol-lanes">{lane.failure&&<p className="detail-warning" role="alert">{lane.failure}</p>}<div role="tablist" aria-label="协议通道">{lanes.map(item=><button type="button" key={item.id} role="tab" aria-selected={item.id===laneId} disabled={decoding} className={`button secondary ${item.id===laneId?'active':''}`} onClick={()=>setLaneId(item.id)}>{item.id+1} · {item.kind.toUpperCase()}{item.decoded?` · ${item.decoded.frame_count} 帧`:''}</button>)}</div><div className="assistant-buttons"><button className="button secondary" disabled={!summary||!view||busy||decoding||lanes.length>=16} onClick={()=>{const id=nextLane.current++;setLanes(previous=>[...previous,{...lane,id,roles:{...lane.roles},decoded:null,frameIndex:null,detail:[]}]);setLaneId(id);}}>添加协议通道</button><button className="button secondary" disabled={busy||decoding||lanes.length<=1} onClick={()=>void removeLane()}>移除当前</button><button className="button secondary" disabled={!summary||!view||busy||decoding||!['uart','ddsss','usb_pd','scp','ufcs'].includes(kind)||lanes.length>=16} onClick={()=>void decodeRemaining()}>其余通道同时解析</button><button className="button secondary" disabled={!summary||!view||busy||decoding||!lanes.some(item=>item.kind!=='off')} onClick={()=>void decodeAll()}>解析全部通道</button><button className="button secondary" disabled={busy||decoding||!lanes.some(item=>item.kind!=='off')||lanes.some(item=>item.kind!=='off'&&(!item.decoded||item.decoded.gate_key!==gateKey(item)))} onClick={()=>void protocolReport()}>协议分析报告…</button></div></div>
        <fieldset className="wave-bus-bar" disabled={decoding||busy}>
          <label><LocalizedLabel text={"协议"}/><select aria-label={localizeLabel("协议")} value={kind} onChange={event=>{setKind(event.target.value);setDecoded(null);setFrameIndex(null);setDetail([]);}}><option value="off"><LocalizedLabel text={"关闭"}/></option><option value="uart">UART</option><option value="i2c">I2C</option><option value="spi">SPI</option><option value="i2s">I2S</option><option value="ddsss">DDSSS</option><option value="usb_pd">USB PD</option><option value="scp">SCP</option><option value="ufcs">UFCS</option></select></label>
          {kind==='uart'&&<>{field('信号','uart_signal')}<label><LocalizedLabel text={"波特率"}/><input aria-label={localizeLabel("波特率")} inputMode="decimal" placeholder={localizeLabel("自动")} value={baud} onChange={event=>setBaud(event.target.value)}/></label><label><LocalizedLabel text={"数据位"}/><select aria-label={localizeLabel("数据位")} value={dataBits} onChange={event=>setDataBits(Number(event.target.value))}>{[5,6,7,8,9].map(bits=><option key={bits} value={bits}>{bits}</option>)}</select></label><label><LocalizedLabel text={"校验"}/><select aria-label={localizeLabel("校验")} value={parity} onChange={event=>setParity(event.target.value)}><option value="none"><LocalizedLabel text={"无"}/></option><option value="even"><LocalizedLabel text={"偶"}/></option><option value="odd"><LocalizedLabel text={"奇"}/></option><option value="mark">Mark</option><option value="space">Space</option></select></label><label><LocalizedLabel text={"停止位"}/><select aria-label={localizeLabel("停止位")} value={stopBits} onChange={event=>setStopBits(Number(event.target.value))}><option value={1}>1</option><option value={2}>2</option></select></label><label className="wave-bus-check"><input type="checkbox" checked={inverted} onChange={event=>setInverted(event.target.checked)}/><LocalizedLabel text={"反相"}/></label></>}
          {kind==='i2c'&&<>{field('SCL','i2c_scl')}{field('SDA','i2c_sda')}<label><LocalizedLabel text={"寄存器"}/><select aria-label={localizeLabel("寄存器地址宽度")} value={regBits} onChange={event=>setRegBits(Number(event.target.value))}><option value={8}><LocalizedLabel text={"8 位"}/></option><option value={16}><LocalizedLabel text={"16 位"}/></option></select></label></>}
          {kind==='spi'&&<>
            <label><LocalizedLabel text={"线制"}/><select aria-label={localizeLabel("SPI 线制")} value={spiWire} onChange={event=>setSpiWire(event.target.value)}><option value="2"><LocalizedLabel text={"2 线"}/></option><option value="4"><LocalizedLabel text={"4 线"}/></option><option value="dual">Dual</option><option value="quad">Quad</option></select></label>
            {field('CLK','spi_clk')}{field(spiWire==='dual'||spiWire==='quad'?'IO0':'MOSI','spi_mosi')}
            {spiWire!=='2'&&field(spiWire==='4'?'MISO':'IO1','spi_miso',spiWire==='4')}
            {spiWire==='quad'&&<>{field('IO2','spi_io2')}{field('IO3','spi_io3')}</>}
            {field('CS','spi_cs',true)}
            <label><LocalizedLabel text={"模式"}/><select aria-label={localizeLabel("SPI 模式")} value={spiMode} onChange={event=>setSpiMode(Number(event.target.value))}>{[0,1,2,3].map(mode=><option key={mode} value={mode}>{mode}</option>)}</select></label>
            <label><LocalizedLabel text={"位宽"}/><select aria-label={localizeLabel("SPI 位宽")} value={spiWord} onChange={event=>setSpiWord(Number(event.target.value))}>{[8,16,24,32].map(bits=><option key={bits} value={bits}>{bits}</option>)}</select></label>
            <label><LocalizedLabel text={"位序"}/><select aria-label={localizeLabel("SPI 位序")} value={spiMsb?'msb':'lsb'} onChange={event=>setSpiMsb(event.target.value==='msb')}><option value="msb">MSB</option><option value="lsb">LSB</option></select></label>
            <label><LocalizedLabel text={"片选"}/><select aria-label={localizeLabel("片选极性")} value={spiCsLow?'low':'high'} onChange={event=>setSpiCsLow(event.target.value==='low')}><option value="low"><LocalizedLabel text={"低有效"}/></option><option value="high"><LocalizedLabel text={"高有效"}/></option></select></label>
          </>}
          {kind==='i2s'&&<>{field('BCLK','i2s_bclk')}{field('WS','i2s_ws')}{field('DATA','i2s_data')}<label><LocalizedLabel text={"位宽"}/><select aria-label={localizeLabel("I2S 位宽")} value={i2sBits} onChange={event=>setI2sBits(Number(event.target.value))}><option value={16}>16</option><option value={24}>24</option><option value={32}>32</option></select></label><label><LocalizedLabel text={"格式"}/><select aria-label={localizeLabel("I2S 格式")} value={i2sFormat} onChange={event=>setI2sFormat(event.target.value)}><option value="philips"><LocalizedLabel text={"飞利浦"}/></option><option value="left"><LocalizedLabel text={"左对齐"}/></option></select></label></>}
          {kind==='ddsss'&&<>{field('信号','ddsss_signal')}<label><LocalizedLabel text={"序列"}/><select aria-label={localizeLabel("DDSSS 序列")} value={ddsssSeq} onChange={event=>setDdsssSeq(event.target.value)}><option value="auto"><LocalizedLabel text={"自动"}/></option><option value="seqa">SEQA</option><option value="seqb">SEQB</option><option value="seqc">SEQC</option><option value="seqd">SEQD</option></select></label><label><LocalizedLabel text={"扩展码"}/><select aria-label={localizeLabel("DDSSS 扩展码")} value={ddsssExt} onChange={event=>setDdsssExt(event.target.value)}><option value="auto"><LocalizedLabel text={"自动"}/></option><option value="off"><LocalizedLabel text={"关"}/></option><option value="on"><LocalizedLabel text={"开"}/></option></select></label><label>FOP Hz<input aria-label="FOP Hz" inputMode="decimal" placeholder={localizeLabel("自动")} value={fop} onChange={event=>setFop(event.target.value)}/></label></>}
          {(kind==='usb_pd'||kind==='scp'||kind==='ufcs')&&<ChargingFields kind={kind} field={field} manual={manualThreshold} threshold={threshold} ufcsBaud={ufcsBaud} scpUi={scpUi} onManual={enabled=>{setManualThreshold(enabled);if(enabled)setThreshold(kind==='scp'?'1.1':'0.6');}} onThreshold={setThreshold} onUfcsBaud={setUfcsBaud} onScpUi={setScpUi}/>}
          {kind!=='off'&&<label className="wave-bus-check"><input type="checkbox" checked={gate} onChange={event=>setGate(event.target.checked)}/>解码门控{gate&&<select value={gateMode} onChange={event=>setGateMode(event.target.value as 'cursors'|'view')}><option value="cursors">{localizeLabel(kind==='ddsss'?'全波形（光标仅测量）':'X1–X2（无双游标时全波形）')}</option><option value="view">当前视口</option></select>}</label>}
          {kind==='ddsss'&&gate&&gateMode==='cursors'&&<p className="wave-bus-empty" style={{gridColumn:'1 / -1',margin:'4px 0'}}><LocalizedLabel text="DDSSS 保留完整信号用于相关锁定；需要截取解码时请选择当前视口。"/></p>}
          <button type="button" className="button primary" disabled={kind==='off'||!summary||busy||decoding} onClick={()=>void runDecode()}>{decoding?localizeLabel("解码中…"):localizeLabel("解码")}</button>
        </fieldset>
        {kind==='off'?<div className="wave-bus-empty"><LocalizedLabel text={"选择协议后，用原始采样点解码。结果出现在下方，不会改动波形文件。"}/></div>:validDecoded?<div className="wave-bus-body">{validDecoded.frames.length?<ProtocolFrameList frames={validDecoded.frames} selected={frameIndex} onSelect={focusFrame} onLocate={focusFrame} query={query} issues={issues} onQuery={setQuery} onIssues={setIssues}/>:<div className="wave-bus-empty">{validDecoded.error||localizeLabel("这个范围里没有解出事务。")}</div>}
<div className="wave-frame-detail">{validDecoded.info&&<p>{validDecoded.info}</p>}{validDecoded.truncated&&<p><LocalizedLabel text={"结果已达到解码上限，后面的数据没有继续解析。"}/></p>}{validDecoded.error&&<p>{validDecoded.error}</p>}{detail.map((line,index)=><p key={index}>{line}</p>)}{activeFrame&&<button type="button" className="button secondary" onClick={locate}><LocalizedLabel text={"定位到波形"}/></button>}</div></div>:<div className="wave-bus-empty">{summary?localizeLabel("指定通道后解码。"):localizeLabel("打开波形文件后才能解码。")}</div>}
      </section>
    </aside></div></section></ResizableWorkspace>;
}
