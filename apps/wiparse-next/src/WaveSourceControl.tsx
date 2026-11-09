import {LocalizedLabel,localizeLabel} from './localization';
import {useEffect,useRef,useState} from 'react';
import {captureWaveformSource,readLegacy} from './bridge';
import {object,string} from './model';
import {useDeviceInterlock} from './useDeviceInterlock';

export function WaveSourceControl({device,supported,disabled}:{device:unknown;supported:boolean;disabled:boolean}) {
  const item=object(device),caps=object(item.capabilities),id=item.device_id,resource=string(item.resource);
  const [busy,setBusy]=useState(false),[job,setJob]=useState<{id:number;directory:string}|null>(null),[notice,setNotice]=useState(''),[error,setError]=useState('');
  const {uncertain,setUncertain,begin,finish,acknowledge}=useDeviceInterlock('waveform',resource),operation=useRef<string|null>(null);
  useEffect(()=>{
    if(!job||typeof id!=='number')return;
    let active=true,timer:ReturnType<typeof setTimeout>;const started=Date.now();
    const poll=async()=>{try{
      if(Date.now()-started>120000)throw Error('读取超时，请检查设备及保存目录；不要立即重复操作。');
      const result=object(await readLegacy('instrument.job.get',{job_id:job.id}));if(!active)return;
      if(result.state==='pending_or_unknown'){timer=setTimeout(poll,1000);return;}
      if(result.state!=='completed'||result.job_id!==job.id||result.device_id!==id)throw Error('设备任务状态不匹配。');
      if(result.ok!==true)throw Error(string(object(result.detail).error,'波形读取失败'));
      if(result.kind!=='waveform_source')throw Error('设备任务类型不匹配。');
      const files=object(result.detail).files;
      if(!Array.isArray(files)||!files.length||files.some(path=>typeof path!=='string'||!path))throw Error('任务未返回保存文件，请检查目录。');
      if(!operation.current||!finish(operation.current))throw Error('文件已保存，但设备恢复标记未能清除');
      setNotice(`源波形已保存到 ${job.directory}。`);setJob(null);
    }catch(reason){if(active){setUncertain(true);setError(`${String(reason)} 保存目录：${job.directory}`);setJob(null);}}};
    void poll();return()=>{active=false;clearTimeout(timer);};
  },[job,id]);
  const capture=async()=>{if(typeof id!=='number'||!resource)return;setBusy(true);setError('');setNotice('');try{
    operation.current=begin();
    const response=await captureWaveformSource(id,resource);if(response===null){if(operation.current)finish(operation.current);return;}
    const value=object(response),jobId=value.job_id,directory=string(value.directory);
    if(typeof jobId!=='number'||!Number.isSafeInteger(jobId)||jobId<1||value.device_id!==id||!directory)throw Error('请求已提交，但任务信息无效，请检查设备。');
    setJob({id:jobId,directory});setNotice(`正在读取显示通道，保存目录：${directory}`);
  }catch(reason){const message=String(reason),rejected=['INSTRUMENT.TEST_IN_PROGRESS','INSTRUMENT.SESSION_CHANGED','INSTRUMENT.BUSY','INSTRUMENT.WAVEFORM_UNSUPPORTED','INSTRUMENT.INVALID_DIRECTORY','INSTRUMENT.INVALID_REQUEST'].some(code=>message.includes(code));if(rejected&&operation.current){if(!finish(operation.current))setUncertain(true);}else setUncertain(true);setError(message.includes('INSTRUMENT.TEST_IN_PROGRESS')?localizeLabel("测试运行中不能手动读取波形。"):message.includes('INSTRUMENT.SESSION_CHANGED')?localizeLabel("设备连接已变化，请重新选择。"):message.includes('INSTRUMENT.BUSY')?localizeLabel("仪器正在处理其他操作。"):`${message}。请检查设备及保存目录后继续。`);}finally{setBusy(false);}};
  return <div className="source-controls"><div className="section-heading"><div><h2><LocalizedLabel text={"源波形"}/></h2><p><LocalizedLabel text={"读取示波器当前显示的通道，保存到新建目录"}/></p></div><button type="button" className="button secondary" disabled={disabled||!supported||caps.waveform!==true||busy||!!job||uncertain||item.disconnecting===true||typeof id!=='number'||!resource} onClick={()=>void capture()}>{busy?localizeLabel("选择目录…"):job?localizeLabel("读取中…"):localizeLabel("读取并保存")}</button></div>{notice&&<div className="detail-help" role="status">{notice}</div>}{error&&<div className="detail-warning" role="alert">{error}</div>}{uncertain&&<button type="button" className="button secondary" onClick={()=>{if(acknowledge())setError('');}}><LocalizedLabel text={"已检查设备及目录"}/></button>}</div>;
}
