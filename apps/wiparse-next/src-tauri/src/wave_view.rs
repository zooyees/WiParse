//! Bounded, offline waveform viewing. The user must select a local file; no
//! device session is opened and source samples are never altered.
#[cfg(test)]use wiparse_core::waveform_file::load_waveform_file_all;
use std::{io::{BufWriter,Write},path::{Path,PathBuf}, sync::{Arc,Mutex,atomic::{AtomicBool,AtomicU64,Ordering}}};
use serde::{Deserialize,Serialize};
use std::collections::BTreeMap;
use wiparse_core::waveform_protocol_html::{ProtocolReport,ProtocolReportWave,ProtocolReportLane,ProtocolReportFrame,ProtocolReportField,render_protocol_report,detail_windows,detail_columns};
use super::wave_extrema::ExtremaIndex;
use wiparse_core::bus_decode::{decode_bus,explain_frame,BusChannelMap,BusDecodeResult,BusDecodeSettings,BusFrameError,BusKind,DdsssExtension,DdsssSequence,I2sFormat,SpiMode,SpiWire,UartParity};
use wiparse_core::{instrument::WaveformTrace, waveform_file::{write_waveform_csv,measure_waveform_range,export_waveform_isf,export_waveform_wfm}};




const MAX_BUCKETS:usize=2048;

#[derive(Default)]
pub struct WaveState { data:Arc<Mutex<Option<Dataset>>>, busy:Arc<AtomicBool>, generation:Arc<AtomicU64>, measure_generation:Arc<AtomicU64>, measure_lock:Arc<Mutex<()>>, decode:Arc<Mutex<BTreeMap<u64,DecodeCache>>> }
#[derive(Clone)]
struct DecodeCache { time_gate:Option<(f64,f64)>,revision:u64,generation:u64,kind:BusKind,result:Arc<BusDecodeResult>,channels:Vec<usize> }
#[derive(Clone)]
struct Dataset { name:String, traces:Arc<Vec<WaveformTrace>>, extrema:Arc<Vec<ExtremaIndex>>, files:Vec<SourceFile> }
#[derive(Clone)]
struct SourceFile { id:String, path:PathBuf, name:String, channels:usize }
#[derive(Serialize)]
pub struct WaveFile { id:String, name:String, channels:Vec<usize> }
#[derive(Serialize)]
pub struct WaveChannel { index:usize, name:String, points:usize, start:f64, end:f64, x_unit:String, y_unit:String }
#[derive(Serialize)]
pub struct WaveSummary { filename:String, channels:Vec<WaveChannel>, files:Vec<WaveFile> }
#[derive(Serialize)]
pub struct Bucket { x0:f64, x1:f64, low:f64, high:f64 }
#[derive(Serialize)]
pub struct WaveView { channel:String, x_unit:String, y_unit:String, start:f64, end:f64, minimum:f64, maximum:f64, samples:usize, buckets:Vec<Bucket> }
#[derive(Serialize)]
pub struct WavePoint { index:usize, x:f64, y:f64, x_unit:String, y_unit:String }
#[derive(Serialize)]
pub struct WaveMetrics { count:usize, minimum:Option<f64>, maximum:Option<f64>, mean:Option<f64>, rms:Option<f64>, peak_to_peak:Option<f64>, sample_interval:Option<f64>, frequency_hz:Option<f64>, period_s:Option<f64>, x_unit:String, y_unit:String }

struct BusyGuard(Arc<AtomicBool>);
impl Drop for BusyGuard { fn drop(&mut self){self.0.store(false,Ordering::Release);} }
struct TempOutput(PathBuf);
impl Drop for TempOutput {fn drop(&mut self){let _=std::fs::remove_file(&self.0);}}

#[tauri::command]
pub async fn wave_open(state:tauri::State<'_,WaveState>)->Result<Option<WaveSummary>,String>{
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有波形文件正在打开".into());}
    let data=state.data.clone();let busy=state.busy.clone();let generation=state.generation.clone();let decode=state.decode.clone();
    let token=generation.load(Ordering::Acquire);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard=BusyGuard(busy);
        let Some(path)=rfd::FileDialog::new().add_filter("波形文件",&["csv","txt","isf","wfm"]).pick_file() else{return Ok(None)};
        let dataset=load_dataset(&path)?;
        commit_dataset(&data,&generation,&decode,token,Some(dataset))
    }).await.map_err(|_|"波形文件打开失败".to_string())?
}

/// Append local documents transactionally. Samples remain shared, with one global budget.
#[tauri::command]
pub async fn wave_add_files(state:tauri::State<'_,WaveState>,browser:tauri::State<'_,super::offline_browser::OfflineBrowserState>,browser_selection:Option<super::offline_browser::OfflineFile>,replace:Option<bool>)->Result<Option<WaveSummary>,String>{
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有波形操作正在进行".into());}
    let permit=BusyGuard(state.busy.clone());let data=state.data.clone();let generation=state.generation.clone();let decode=state.decode.clone();let browser=browser.data.clone();
    let token=generation.load(Ordering::Acquire);
    tauri::async_runtime::spawn_blocking(move||{
        let _permit=permit;
        let paths=if let Some(selection)=browser_selection{vec![super::offline_browser::resolve_file(&browser,"wave",&selection)?]}else{let Some(paths)=rfd::FileDialog::new().add_filter("波形文件",&["csv","txt","isf","wfm"]).pick_files()else{return Ok(None)};paths};
        let previous=data.lock().map_err(|_|"波形会话异常")?.clone();
        let next=load_selected(previous,paths,replace.unwrap_or(false),super::resource_policy::points(),super::resource_policy::source_limit())?;
        commit_dataset(&data,&generation,&decode,token,next)
    }).await.map_err(|_|"添加波形文件失败".to_string())?
}

/// Build the replacement/append off-session. Cancellation or failure preserves
/// the committed library; only remaining sample capacity is given to parsers.
fn load_selected(previous:Option<Dataset>,paths:Vec<PathBuf>,replace:bool,point_limit:usize,source_limit:usize)->Result<Option<Dataset>,String>{
    if paths.len()>source_limit{return Err(format!("所选文件数 {} 超过来源上限 {source_limit}，当前波形未更改",paths.len()));}
    let mut next=if replace{None}else{previous};
    for path in paths {
        let canonical=std::fs::canonicalize(&path).map_err(|error|format!("无法读取文件：{error}"))?;
        if next.as_ref().is_some_and(|value|value.files.iter().any(|file|same_source(&file.path,&canonical))){continue;}
        let used=next.as_ref().map(dataset_points).transpose()?.unwrap_or(0);
        let files=next.as_ref().map_or(0,|value|value.files.len());
        if files>=source_limit{return Err(format!("波形来源数已达上限 {source_limit}；请移除文件后添加，或使用打开替换。当前波形未更改"));}
        let remaining=point_limit.saturating_sub(used);
        if remaining==0{return Err(sample_budget_error(used,point_limit));}
        let added=load_dataset_bounded(&canonical,remaining).map_err(|error|format!("{error}；已有 {used} 点，本批剩余 {remaining} 点，总上限 {point_limit} 点。可用打开替换、移除文件或在设置中调整内存预算；当前波形未更改"))?;
        next=Some(merge_dataset_bounded(next,added,point_limit,source_limit)?);
    }
    Ok(next)
}
fn dataset_points(data:&Dataset)->Result<usize,String>{data.traces.iter().try_fold(0usize,|sum,trace|sum.checked_add(trace.x.len())).ok_or_else(||"波形采样点数过大".into())}
fn sample_budget_error(points:usize,limit:usize)->String{format!("波形库采样点数 {points} 点，当前上限 {limit} 点。请用打开替换、移除文件或在设置中调整内存预算；当前波形未更改")}

#[tauri::command]
pub fn wave_remove_file(state:tauri::State<'_,WaveState>,id:String)->Result<Option<WaveSummary>,String>{
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有波形操作正在进行".into());}
    let _permit=BusyGuard(state.busy.clone());
    let mut data=state.data.lock().map_err(|_|"波形会话异常")?;
    let previous=data.as_ref().ok_or("尚未打开波形")?;
    let next=remove_source(previous,&id)?;
    let result=next.as_ref().map(summary);
    state.generation.fetch_add(1,Ordering::AcqRel);*data=next;
    if let Ok(mut cached)=state.decode.lock(){cached.clear();}
    Ok(result)
}

fn commit_dataset(data:&Mutex<Option<Dataset>>,generation:&AtomicU64,decode:&Mutex<BTreeMap<u64,DecodeCache>>,expected:u64,next:Option<Dataset>)->Result<Option<WaveSummary>,String>{
    let mut guard=data.lock().map_err(|_|"波形会话异常")?;
    if generation.load(Ordering::Acquire)!=expected{return Err("波形会话已变化，未提交文件".into());}
    let result=next.as_ref().map(summary);
    generation.fetch_add(1,Ordering::AcqRel);*guard=next;
    if let Ok(mut cached)=decode.lock(){cached.clear();}
    Ok(result)
}

fn same_source(a:&Path,b:&Path)->bool{
    if cfg!(windows){a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())}else{a==b}
}

#[cfg(test)]
fn merge_dataset(previous:Option<Dataset>,added:Dataset)->Result<Dataset,String>{merge_dataset_bounded(previous,added,super::resource_policy::points(),super::resource_policy::source_limit())}
fn merge_dataset_bounded(previous:Option<Dataset>,added:Dataset,point_limit:usize,source_limit:usize)->Result<Dataset,String>{
    let Some(previous)=previous else{
        let points=dataset_points(&added)?;
        if points>point_limit{return Err(sample_budget_error(points,point_limit));}
        if added.files.len()>source_limit||added.traces.len()>source_limit{return Err(format!("波形来源或通道数超过上限 {source_limit}，当前波形未更改"));}
        return Ok(added)
    };
    if previous.files.iter().any(|old|added.files.iter().any(|new|same_source(&old.path,&new.path))){return Ok(previous);}
    let channels=previous.traces.len().checked_add(added.traces.len()).ok_or("波形通道数过大")?;
    let points=previous.traces.iter().chain(added.traces.iter()).try_fold(0usize,|sum,trace|sum.checked_add(trace.x.len())).ok_or("波形采样点数过大")?;
    if points>point_limit{return Err(sample_budget_error(points,point_limit));}
    if previous.files.len().saturating_add(added.files.len())>source_limit||channels>source_limit{return Err(format!("波形来源或通道数超过上限 {source_limit}（合计 {channels} 通道）；请移除部分文件或使用打开替换，当前波形未更改"));}
    let mut traces=Vec::with_capacity(channels);traces.extend(previous.traces.iter().cloned());traces.extend(added.traces.iter().cloned());
    let mut extrema=Vec::with_capacity(channels);extrema.extend(previous.extrema.iter().cloned());extrema.extend(added.extrema.iter().cloned());
    let mut files=previous.files;files.extend(added.files);
    Ok(Dataset{name:files.iter().map(|file|file.name.as_str()).collect::<Vec<_>>().join(" · "),traces:Arc::new(traces),extrema:Arc::new(extrema),files})
}

fn remove_source(data:&Dataset,id:&str)->Result<Option<Dataset>,String>{
    if !data.files.iter().any(|file|file.id==id){return Err("波形文件已变化，请重新选择".into());}
    let mut start=0;let mut traces=Vec::new();let mut extrema=Vec::new();let mut files=Vec::new();
    for file in &data.files {
        let end=start+file.channels;
        if file.id!=id{traces.extend(data.traces[start..end].iter().cloned());extrema.extend(data.extrema[start..end].iter().cloned());files.push(file.clone());}
        start=end;
    }
    if files.is_empty(){return Ok(None);}
    Ok(Some(Dataset{name:files.iter().map(|file|file.name.as_str()).collect::<Vec<_>>().join(" · "),traces:Arc::new(traces),extrema:Arc::new(extrema),files}))
}

fn waveform_extension(path:&Path)->bool{
    matches!(path.extension().and_then(|value|value.to_str()).unwrap_or("").to_ascii_lowercase().as_str(),"csv"|"txt"|"isf"|"wfm")
}

#[tauri::command]
pub async fn wave_open_indexed(state:tauri::State<'_,WaveState>,project:String,path:String)->Result<WaveSummary,String>{
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有波形文件正在打开".into());}
    let data=state.data.clone();let busy=state.busy.clone();let generation=state.generation.clone();let decode=state.decode.clone();
    let token=generation.load(Ordering::Acquire);
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=BusyGuard(busy);
        let canonical=super::report_view::resolve_project_file(&project,&path)?;
        if !waveform_extension(&canonical){return Err("该工程文件不能作为波形打开".into());}
        let dataset=load_dataset(&canonical)?;
        commit_dataset(&data,&generation,&decode,token,Some(dataset))?.ok_or_else(||"波形为空".to_string())
    }).await.map_err(|_|"波形文件打开失败".to_string())?
}

#[tauri::command]
pub async fn wave_open_linked(state:tauri::State<'_,WaveState>,report:tauri::State<'_,super::report_view::ReportState>,session:u64,source:String)->Result<WaveSummary,String>{
    let path=super::report_view::resolve_report_link(&report,session,&source)?;
    if !waveform_extension(&path){return Err("该证据不是波形文件".into());}
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有波形操作正在进行".into());}
    let permit=BusyGuard(state.busy.clone());let data=state.data.clone();let generation=state.generation.clone();let decode=state.decode.clone();
    let token=generation.load(Ordering::Acquire);
    let dataset=tauri::async_runtime::spawn_blocking(move||load_dataset(&path)).await.map_err(|_|"波形文件打开失败".to_string())??;
    let _permit=permit;
    super::report_view::resolve_report_link(&report,session,&source)?;
    commit_dataset(&data,&generation,&decode,token,Some(dataset))?.ok_or_else(||"波形为空".to_string())
}

#[tauri::command]
pub fn wave_close(state:tauri::State<'_,WaveState>)->Result<(),String>{
    let mut data=state.data.lock().map_err(|_|"波形会话异常")?;
    state.generation.fetch_add(1,Ordering::AcqRel);
    *data=None;
    if let Ok(mut cached)=state.decode.lock() { cached.clear(); }
    Ok(())
}

#[tauri::command]
pub fn wave_status(state:tauri::State<'_,WaveState>)->Result<Option<WaveSummary>,String>{
    let data=state.data.lock().map_err(|_|"波形会话异常")?;
    Ok(data.as_ref().map(summary))
}

#[tauri::command]
pub fn wave_sample(state:tauri::State<'_,WaveState>,channel:usize,x:f64)->Result<WavePoint,String>{
    let guard=state.data.lock().map_err(|_|"波形会话异常")?;
    let trace=guard.as_ref().and_then(|data|data.traces.get(channel)).ok_or("波形通道不存在")?;
    sample(trace,x)
}

#[tauri::command]
pub async fn wave_export_csv(state:tauri::State<'_,WaveState>,channel:usize)->Result<Option<String>,String>{
    export_requested(state,channel,"csv").await
}

#[tauri::command]
pub async fn wave_export_file(state:tauri::State<'_,WaveState>,channel:usize,format:String)->Result<Option<String>,String>{
    let format=match format.as_str(){"csv"=>"csv","isf"=>"isf","wfm"=>"wfm",_=>return Err("不支持的波形导出格式".into())};
    export_requested(state,channel,format).await
}

async fn export_requested(state:tauri::State<'_,WaveState>,channel:usize,format:&'static str)->Result<Option<String>,String>{
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有波形文件操作正在进行".into());}
    let permit=BusyGuard(state.busy.clone());
    let generation=state.generation.clone();
    let data=state.data.clone();
    let (token,dataset)={let guard=state.data.lock().map_err(|_|"波形会话异常")?;let dataset=guard.as_ref().ok_or("请先打开波形文件")?;if channel>=dataset.traces.len(){return Err("波形通道不存在".into());}(generation.load(Ordering::Acquire),dataset.clone())};
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=permit;
        let trace=&dataset.traces[channel];
        let Some(path)=rfd::FileDialog::new().add_filter(format,&[format]).set_file_name(format!("waveform.{format}")).save_file() else{return Ok(None)};
        if generation.load(Ordering::Acquire)!=token{return Err("波形会话已改变，请重新导出".into());}
        export_format_new(trace,&path,format,|temp,path|{
            let _session=data.lock().map_err(|_|"波形会话异常")?;
            if generation.load(Ordering::Acquire)!=token{return Err("波形会话已改变，请重新导出".into());}
            commit_export(temp,path)
        })?;
        Ok(Some(path.file_name().and_then(|value|value.to_str()).unwrap_or("waveform").to_owned()))
    }).await.map_err(|_|"波形导出失败".to_string())?
}

#[cfg(test)]
fn export_new(trace:&WaveformTrace,path:&Path,commit:impl FnOnce(&Path,&Path)->Result<(),String>)->Result<(),String>{
    export_format_new(trace,path,"csv",commit)
}

fn export_format_new(trace:&WaveformTrace,path:&Path,format:&str,commit:impl FnOnce(&Path,&Path)->Result<(),String>)->Result<(),String>{
    if !["csv","isf","wfm"].contains(&format)||!path.extension().and_then(|ext|ext.to_str()).is_some_and(|ext|ext.eq_ignore_ascii_case(format)){return Err(format!("请选择 .{format} 文件名"));}
    validate_export_timebase(trace,format)?;
    let parent=path.parent().ok_or("导出目录无效")?;
    let temp=parent.join(format!(".wiparse-export-{}.{}",uuid::Uuid::new_v4(),format));
    let file=std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e|format!("无法创建导出临时文件：{e}"))?;
    let _cleanup=TempOutput(temp.clone());
    if format=="csv" {
        let mut writer=BufWriter::new(file);
        write_waveform_csv(&mut writer,trace).map_err(|e|format!("波形写入失败：{e}"))?;
        writer.flush().map_err(|e|format!("波形写入失败：{e}"))?;
        writer.get_ref().sync_all().map_err(|e|format!("波形保存失败：{e}"))?;
    } else {
        drop(file);
        // Use the established format writers directly: no full trace clone.
        if format=="isf" {export_waveform_isf(&temp,trace)} else {export_waveform_wfm(&temp,trace)}.map_err(|e|format!("波形写入失败：{e}"))?;
        std::fs::OpenOptions::new().write(true).open(&temp).and_then(|file|file.sync_all()).map_err(|e|format!("波形保存失败：{e}"))?;
    }
    commit(&temp,path)?;
    Ok(())
}

fn validate_export_timebase(trace:&WaveformTrace,format:&str)->Result<(),String>{
    if format=="csv"||trace.x.len()<2{return Ok(());}
    let start=trace.x[0];let end=trace.x[trace.x.len()-1];
    let interval=(end-start)/(trace.x.len()-1) as f64;
    let tolerance=(interval.abs()*1e-7).max(f64::EPSILON*start.abs().max(end.abs())*2.0);
    if !interval.is_finite()||interval<=0.0||trace.x.windows(2).any(|pair|!pair[0].is_finite()||!pair[1].is_finite()||(pair[1]-pair[0]-interval).abs()>tolerance){return Err("ISF/WFM 只能表示等间隔采样；请使用 CSV 保留原始时间戳".into());}
    // The legacy ISF preamble uses 13 significant digits. Do not silently lose
    // sub-sample timing when a large absolute origin exhausts that precision.
    if format=="isf" {
        let encoded_start=format!("{start:.12E}").parse::<f64>().map_err(|_|"ISF 时间原点不可表示")?;
        if (encoded_start-start).abs()>interval.abs()*0.001{return Err("ISF 时间原点精度不足；请使用 CSV 或 WFM".into());}
    }
    Ok(())
}

fn commit_export(temp:&Path,path:&Path)->Result<(),String>{
    super::commit_file::publish_new(temp,path).map_err(|e|if e.kind()==std::io::ErrorKind::AlreadyExists {"目标文件已存在，请选择其他文件名".to_string()} else {format!("无法安全创建导出文件：{e}")})
}

fn sample(trace:&WaveformTrace,x:f64)->Result<WavePoint,String>{
    if !x.is_finite()||x<trace.x[0]||x>trace.x[trace.x.len()-1]{return Err("游标超出波形范围".into());}
    let right=trace.x.partition_point(|value|*value<x);
    let index=if right==0 {0} else if right==trace.x.len(){right-1} else if x-trace.x[right-1]<=trace.x[right]-x {right-1} else {right};
    Ok(WavePoint{index,x:trace.x[index],y:trace.y[index],x_unit:trace.x_unit.clone(),y_unit:trace.y_unit.clone()})
}

#[tauri::command]
pub async fn wave_view(state:tauri::State<'_,WaveState>,channel:usize,start:Option<f64>,end:Option<f64>,buckets:usize)->Result<WaveView,String>{
    let generation=state.generation.clone();
    let (token,dataset)={
        let guard=state.data.lock().map_err(|_|"波形会话异常")?;
        (generation.load(Ordering::Acquire),guard.clone().ok_or("请先打开波形文件")?)
    };
    tauri::async_runtime::spawn_blocking(move || {
        let trace=dataset.traces.get(channel).ok_or("波形通道不存在")?;
        let extrema=dataset.extrema.get(channel).ok_or("波形索引不存在")?;
        let mut view=render_indexed(trace,extrema,start,end,buckets)?;
        view.channel=view_channel_label(&dataset,channel,&trace.channel);
        if generation.load(Ordering::Acquire)!=token{return Err("波形会话已改变，请重新查看".into());}
        Ok(view)
    }).await.map_err(|_|"波形读取失败".to_string())?
}

#[tauri::command]
pub async fn wave_measure(state:tauri::State<'_,WaveState>,channel:usize,start:f64,end:f64)->Result<WaveMetrics,String>{
    let generation=state.generation.clone();
    let measure_generation=state.measure_generation.clone();let measure_lock=state.measure_lock.clone();
    let request=measure_generation.fetch_add(1,Ordering::AcqRel)+1;
    let (token,dataset)={let guard=state.data.lock().map_err(|_|"波形会话异常")?;let dataset=guard.as_ref().ok_or("请先打开波形文件")?;if channel>=dataset.traces.len(){return Err("波形通道不存在".into());}(generation.load(Ordering::Acquire),dataset.clone())};
    tauri::async_runtime::spawn_blocking(move||guarded_metrics(&dataset.traces[channel],start,end,&generation,token,&measure_generation,request,&measure_lock)).await.map_err(|_|"波形测量失败".to_string())?
}

fn guarded_metrics(trace:&WaveformTrace,start:f64,end:f64,generation:&AtomicU64,token:u64,measure_generation:&AtomicU64,request:u64,lock:&Mutex<()>)->Result<WaveMetrics,String>{
    let _guard=lock.lock().map_err(|_|"波形测量队列异常")?;
    if generation.load(Ordering::Acquire)!=token||measure_generation.load(Ordering::Acquire)!=request{return Err("波形测量请求已过期".into());}
    let result=metrics(trace,start,end)?;
    if generation.load(Ordering::Acquire)!=token||measure_generation.load(Ordering::Acquire)!=request{return Err("波形测量请求已过期".into());}
    Ok(result)
}

fn metrics(trace:&WaveformTrace,start:f64,end:f64)->Result<WaveMetrics,String>{
    if !start.is_finite()||!end.is_finite()||start>end{return Err("波形测量范围无效".into());}
    let m=measure_waveform_range(trace,start,end);if m.count==0{return Err("选择范围没有采样点".into());}
    let finite=|value:f64|value.is_finite().then_some(value);
    let seconds=trace.x_unit.trim().eq_ignore_ascii_case("s");
    Ok(WaveMetrics{count:m.count,minimum:finite(m.min),maximum:finite(m.max),mean:finite(m.mean),rms:finite(m.rms),peak_to_peak:finite(m.pp),sample_interval:if m.count>=2 {finite(m.dt)} else {None},frequency_hz:if seconds {m.freq_hz.and_then(finite)}else{None},period_s:if seconds {m.period.and_then(finite)}else{None},x_unit:trace.x_unit.clone(),y_unit:trace.y_unit.clone()})
}

#[tauri::command]
pub async fn wave_analysis_context(state:tauri::State<'_,WaveState>,channel:usize,start:f64,end:f64,expected_files:Vec<String>,cursors:Vec<ContextCursor>,lanes:Vec<ReportLaneSelection>)->Result<serde_json::Value,String>{
    if cursors.len()>2||cursors.iter().any(|point|!point.x.is_finite()||!point.y.is_finite())||lanes.len()>16{return Err("波形摘要参数无效".into());}
    let generation=state.generation.clone();let lock=state.measure_lock.clone();
    let (token,dataset)={let data=state.data.lock().map_err(|_|"波形会话异常")?;let dataset=data.clone().ok_or("尚未打开波形")?;if channel>=dataset.traces.len()||expected_files!=dataset.files.iter().map(|file|file.id.clone()).collect::<Vec<_>>(){return Err("波形来源已变化，请返回工具页刷新".into());}(generation.load(Ordering::Acquire),dataset)};
    let decoded={let cache=state.decode.lock().map_err(|_|"协议会话异常")?;let mut remaining=20usize;
        lanes.iter().map(|selection|{let Some(result)=cache.get(&selection.id).filter(|result|result.generation==token&&result.revision==selection.revision&&result.time_gate==selection.time_start.zip(selection.time_end))else{return serde_json::json!({"lane":selection.id,"stale":true})};
            let frames=result.result.frames.iter().take(remaining).enumerate().map(|(index,frame)|serde_json::json!({"index":index,"start":frame.t_start,"end":frame.t_end,"summary":frame.summary.chars().take(200).collect::<String>(),"error":format!("{:?}",frame.error),"fields":explain_frame(result.kind,&result.result.frames,index,true).into_iter().take(6).map(|text|text.chars().take(180).collect::<String>()).collect::<Vec<_>>()})).collect::<Vec<_>>();remaining=remaining.saturating_sub(frames.len());
            serde_json::json!({"lane":selection.id,"protocol":result.kind.label(),"frames_total":result.result.frames.len(),"truncated":result.result.truncated,"error":result.result.error,"frames":frames})
        }).collect::<Vec<_>>()
    };
    tauri::async_runtime::spawn_blocking(move||{
        let trace=&dataset.traces[channel];let metrics=guarded_metrics(trace,start,end,&generation,token,&AtomicU64::new(0),0,&lock)?;
        let cursors=cursors.iter().map(|point|{let index=trace.x.partition_point(|x|*x<point.x).min(trace.x.len()-1);let index=if index>0&&(trace.x[index-1]-point.x).abs()<(trace.x[index]-point.x).abs(){index-1}else{index};serde_json::json!({"index":index,"x":trace.x[index],"y":trace.y[index]})}).collect::<Vec<_>>();
        Ok(serde_json::json!({"files":dataset.files.iter().map(|file|file.name.chars().take(120).collect::<String>()).collect::<Vec<_>>(),"channel":trace.channel,"measurement_range":{"start":start,"end":end},"metrics":metrics,"cursors":cursors,"decoded":decoded,"raw_samples_attached":false}))
    }).await.map_err(|_|"波形摘要生成失败".to_string())?
}
#[derive(Deserialize)]
pub struct ContextCursor{pub x:f64,pub y:f64}

fn load_dataset(path:&Path)->Result<Dataset,String>{load_dataset_bounded(path,super::resource_policy::points())}
fn load_dataset_bounded(path:&Path,point_limit:usize)->Result<Dataset,String>{
    let metadata=std::fs::metadata(path).map_err(|e|format!("无法读取文件：{e}"))?;
    if !metadata.is_file()||metadata.len()>super::resource_policy::bytes() as u64 {return Err("文件超出内存预算，请在设置中调整预算或缩小输入".into());}
    let traces=wiparse_core::waveform_file::load_waveform_file_bounded(path,point_limit).map_err(|e|format!("波形解析失败：{e}"))?;
    if traces.is_empty()||traces.len()>super::resource_policy::source_limit() || traces.iter().map(|t|t.x.len()).sum::<usize>()>point_limit {return Err("波形为空或超过当前预览的通道或采样点上限".into());}
    for trace in &traces {
        if trace.x.is_empty() || trace.x.len()!=trace.y.len() || trace.x.iter().chain(trace.y.iter()).any(|v|!v.is_finite()) || trace.x.windows(2).any(|v|v[0]>v[1]) {return Err("波形包含空、无效或未按时间排序的数据".into());}
    }
    let name=path.file_name().and_then(|v|v.to_str()).unwrap_or("波形文件").to_owned();
    let canonical=std::fs::canonicalize(path).map_err(|error|format!("无法确认波形来源：{error}"))?;
    let files=vec![SourceFile{id:uuid::Uuid::new_v4().to_string(),path:canonical,name:name.clone(),channels:traces.len()}];
    let extrema=traces.iter().map(|trace|ExtremaIndex::new(&trace.y)).collect::<Result<Vec<_>,_>>()?;
    Ok(Dataset{name,traces:Arc::new(traces),extrema:Arc::new(extrema),files})
}

fn summary(data:&Dataset)->WaveSummary{
    let mut start=0;
    let files=data.files.iter().map(|file|{let channels=(start..start+file.channels).collect();start+=file.channels;WaveFile{id:file.id.clone(),name:file.name.clone(),channels}}).collect();
    WaveSummary{filename:data.name.clone(),files,channels:data.traces.iter().enumerate().map(|(index,t)|WaveChannel{index,name:view_channel_label(data,index,&t.channel),points:t.x.len(),start:t.x[0],end:t.x[t.x.len()-1],x_unit:t.x_unit.clone(),y_unit:t.y_unit.clone()}).collect()}
}

fn view_channel_label(data:&Dataset,index:usize,channel:&str)->String{
    if data.files.len()>1 {
        let mut start=0;
        for (ordinal,file) in data.files.iter().enumerate(){
            if index>=start&&index<start+file.channels{return format!("{} · {} / {channel}",ordinal+1,file.name);}
            start+=file.channels;
        }
    }
    channel.to_owned()
}

#[cfg(test)]
fn render(trace:&WaveformTrace,start:Option<f64>,end:Option<f64>,requested:usize)->Result<WaveView,String>{
    render_impl(trace,None,start,end,requested)
}

fn render_indexed(trace:&WaveformTrace,index:&ExtremaIndex,start:Option<f64>,end:Option<f64>,requested:usize)->Result<WaveView,String>{
    render_impl(trace,Some(index),start,end,requested)
}

fn render_impl(trace:&WaveformTrace,extrema:Option<&ExtremaIndex>,start:Option<f64>,end:Option<f64>,requested:usize)->Result<WaveView,String>{
    if requested==0||requested>MAX_BUCKETS{return Err("显示分辨率无效".into());}
    let first=trace.x[0];let last=trace.x[trace.x.len()-1];
    let lo=start.unwrap_or(first);let hi=end.unwrap_or(last);
    if !lo.is_finite()||!hi.is_finite()||lo>hi||!(hi-lo).is_finite(){return Err("波形显示范围无效".into());}
    let a=trace.x.partition_point(|x|*x<lo);let b=trace.x.partition_point(|x|*x<=hi);
    if a>=b{
        // A shared document viewport can contain another channel but not this one.
        // Keep the axis range and an empty curve instead of losing the whole plot.
        let (minimum,maximum)=if let Some(index)=extrema{index.range(&trace.y,0,trace.y.len()).ok_or("波形索引与采样点不一致")?}else{trace.y.iter().fold((f64::INFINITY,f64::NEG_INFINITY),|(lo,hi),&y|(lo.min(y),hi.max(y)))};
        return Ok(WaveView{channel:trace.channel.clone(),x_unit:trace.x_unit.clone(),y_unit:trace.y_unit.clone(),start:lo,end:hi,minimum,maximum,samples:0,buckets:Vec::new()});
    }
    let count=b-a;let count_buckets=requested.min(count);let mut output=Vec::with_capacity(count_buckets);
    let mut minimum=f64::INFINITY;let mut maximum=f64::NEG_INFINITY;
    for index in 0..count_buckets {
        let left=a+index*count/count_buckets;let right=a+(index+1)*count/count_buckets;
        let (low,high)=if let Some(index)=extrema{
            index.range(&trace.y,left,right).ok_or("波形索引与采样点不一致")?
        }else{
            trace.y[left..right].iter().fold((f64::INFINITY,f64::NEG_INFINITY),|(low,high),&value|(low.min(value),high.max(value)))
        };
        minimum=minimum.min(low);maximum=maximum.max(high);
        output.push(Bucket{x0:trace.x[left],x1:trace.x[right-1],low,high});
    }
    if !(maximum-minimum).is_finite(){return Err("波形幅度超出显示范围".into());}
    Ok(WaveView{channel:trace.channel.clone(),x_unit:trace.x_unit.clone(),y_unit:trace.y_unit.clone(),start:lo,end:hi,minimum,maximum,samples:count,buckets:output})
}

#[derive(Deserialize, Default, Clone)]
pub struct WaveDecodeRequest {
    #[serde(default)] pub lane:Option<u64>,
    #[serde(default)] pub revision:u64,
    pub kind:String,
    #[serde(default)] pub uart_signal:Option<usize>,
    #[serde(default)] pub i2c_scl:Option<usize>,
    #[serde(default)] pub i2c_sda:Option<usize>,
    #[serde(default)] pub spi_clk:Option<usize>,
    #[serde(default)] pub spi_mosi:Option<usize>,
    #[serde(default)] pub spi_miso:Option<usize>,
    #[serde(default)] pub spi_cs:Option<usize>,
    #[serde(default)] pub spi_io2:Option<usize>,
    #[serde(default)] pub spi_io3:Option<usize>,
    #[serde(default)] pub i2s_bclk:Option<usize>,
    #[serde(default)] pub i2s_ws:Option<usize>,
    #[serde(default)] pub i2s_data:Option<usize>,
    #[serde(default)] pub ddsss_signal:Option<usize>,
    #[serde(default)] pub charging_signal:Option<usize>,
    #[serde(default)] pub baud:Option<f64>,
    #[serde(default)] pub data_bits:Option<u8>,
    #[serde(default)] pub parity:Option<String>,
    #[serde(default)] pub stop_bits:Option<u8>,
    #[serde(default)] pub inverted:Option<bool>,
    #[serde(default)] pub idle_high:Option<bool>,
    #[serde(default)] pub threshold:Option<f64>,
    #[serde(default)] pub time_start:Option<f64>,
    #[serde(default)] pub time_end:Option<f64>,
    #[serde(default)] pub i2c_reg_bits:Option<u8>,
    #[serde(default)] pub spi_mode:Option<u8>,
    #[serde(default)] pub spi_wire:Option<String>,
    #[serde(default)] pub spi_word_bits:Option<u8>,
    #[serde(default)] pub spi_msb_first:Option<bool>,
    #[serde(default)] pub spi_cs_active_low:Option<bool>,
    #[serde(default)] pub i2s_bits:Option<u8>,
    #[serde(default)] pub i2s_format:Option<String>,
    #[serde(default)] pub ddsss_sequence:Option<String>,
    #[serde(default)] pub ddsss_extension:Option<String>,
    #[serde(default)] pub fop_hz:Option<f64>,
    #[serde(default)] pub ufcs_baud:Option<u32>,
    #[serde(default)] pub scp_ui_us:Option<f64>,
}
#[derive(Serialize)]
pub struct WaveFrameRow { pub index:usize, pub t_start:f64, pub t_end:f64, pub summary:String, pub error:String, pub bytes:usize, pub preview:String, pub hex:String }
#[derive(Serialize)]
pub struct WaveDecodeView { pub kind:String, pub info:String, pub error:Option<String>, pub truncated:bool, pub frame_count:usize, pub frames:Vec<WaveFrameRow> }

#[tauri::command]
pub async fn wave_decode(state:tauri::State<'_,WaveState>,request:WaveDecodeRequest)->Result<WaveDecodeView,String>{
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有波形文件操作正在进行".into());}
    let permit=BusyGuard(state.busy.clone());
    let generation=state.generation.clone();
    let decode=state.decode.clone();
    let settings=decode_settings(&request)?;
    let kind=settings.kind;let time_gate=settings.time_gate;
    let lane=request.lane.unwrap_or(0);let revision=request.revision;let channels=request_channels(&request);
    {let cached=decode.lock().map_err(|_|"波形会话异常")?;if !cached.contains_key(&lane)&&cached.len()>=16{return Err("最多保留 16 路协议解析".into());}}
    let (token,traces)={
        let guard=state.data.lock().map_err(|_|"波形会话异常")?;
        (generation.load(Ordering::Acquire),guard.as_ref().ok_or("请先打开波形文件")?.traces.clone())
    };
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=permit;
        let result=decode_bus(&traces,&settings);
        if generation.load(Ordering::Acquire)!=token{return Err("波形会话已改变，请重新解码".into());}
        let view=present_decode(kind,&result);
        let mut cached=decode.lock().map_err(|_|"波形会话异常")?;
        cached.insert(lane,DecodeCache{time_gate,revision,generation:token,kind,result:Arc::new(result),channels});
        Ok(view)
    }).await.map_err(|_|"协议解码失败".to_string())?
}

#[derive(Deserialize)]pub struct OverlayRequest {pub lane:u64,pub revision:u64,pub start:f64,pub end:f64,pub selected:Option<usize>,pub gate_start:Option<f64>,pub gate_end:Option<f64>,pub budget:usize}
#[tauri::command]
pub async fn wave_overlay(state:tauri::State<'_,WaveState>,request:OverlayRequest)->Result<super::protocol_overlay::Overlay,String>{
    let generation=state.generation.clone();let token=generation.load(Ordering::Acquire);
    let result={let guard=state.decode.lock().map_err(|_|"波形会话异常")?;overlay_result(&guard,token,&request)?};
    tauri::async_runtime::spawn_blocking(move||{let overlay=super::protocol_overlay::render(&result,request.start,request.end,request.budget,request.selected)?;if generation.load(Ordering::Acquire)!=token{return Err("波形会话已变化".into());}Ok(overlay)}).await.map_err(|_|"协议标注读取失败".to_string())?
}

fn overlay_result(cache:&BTreeMap<u64,DecodeCache>,generation:u64,request:&OverlayRequest)->Result<Arc<BusDecodeResult>,String>{
    let cached=cache.get(&request.lane).ok_or("请先解码当前协议通道")?;
    if cached.generation!=generation||cached.revision!=request.revision||request.gate_start.is_some()!=request.gate_end.is_some()||cached.time_gate!=request.gate_start.zip(request.gate_end){return Err("协议结果已变化，请重新解码".into());}
    Ok(cached.result.clone())
}

#[tauri::command]
pub fn wave_explain(state:tauri::State<'_,WaveState>,index:usize,lane:Option<u64>)->Result<Vec<String>,String>{
    let generation=state.generation.load(Ordering::Acquire);
    let guard=state.decode.lock().map_err(|_|"波形会话异常")?;
    let cached=guard.get(&lane.unwrap_or(0)).ok_or("请先解码当前协议通道")?;
    if cached.generation!=generation {return Err("波形已更换，请重新解码".into());}
    if index>=cached.result.frames.len() {return Err("所选事务不存在".into());}
    Ok(explain_frame(cached.kind,&cached.result.frames,index,true))
}

fn request_channels(request:&WaveDecodeRequest)->Vec<usize>{
    let values=match request.kind.as_str(){
        "uart"=>vec![request.uart_signal],"i2c"=>vec![request.i2c_scl,request.i2c_sda],
        "spi"=>vec![request.spi_clk,request.spi_mosi,request.spi_miso,request.spi_cs,request.spi_io2,request.spi_io3],
        "i2s"=>vec![request.i2s_bclk,request.i2s_ws,request.i2s_data],"ddsss"=>vec![request.ddsss_signal],_=>vec![request.charging_signal]
    };
    let mut channels=Vec::new();for index in values.into_iter().flatten(){if !channels.contains(&index){channels.push(index);}}channels
}
#[tauri::command]
pub fn wave_lane_remove(state:tauri::State<'_,WaveState>,lane:u64)->Result<(),String>{state.decode.lock().map_err(|_|"协议会话异常")?.remove(&lane);Ok(())}

#[derive(Deserialize)]
pub struct ReportLaneSelection{pub id:u64,pub revision:u64,#[serde(default)] pub time_start:Option<f64>,#[serde(default)] pub time_end:Option<f64>}

#[tauri::command]
pub async fn wave_protocol_report(state:tauri::State<'_,WaveState>,open:bool,focus_lane:Option<u64>,focus_frame:Option<usize>,lanes:Vec<ReportLaneSelection>)->Result<Option<String>,String>{
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("波形操作进行中".into());}
    let permit=BusyGuard(state.busy.clone());let generation=state.generation.clone();
    let (token,dataset)={let data=state.data.lock().map_err(|_|"波形会话异常")?;(generation.load(Ordering::Acquire),data.clone().ok_or("请先打开波形")?)};
    if lanes.is_empty()||lanes.len()>16||lanes.iter().map(|lane|lane.id).collect::<std::collections::HashSet<_>>().len()!=lanes.len(){return Err("协议报告通道选择无效".into());}
    let cached={let cache=state.decode.lock().map_err(|_|"协议会话异常")?;let mut selected=BTreeMap::new();for lane in lanes{let gate=match (lane.time_start,lane.time_end){(None,None)=>None,(Some(start),Some(end)) if start.is_finite()&&end.is_finite()&&start<=end=>Some((start,end)),_=>return Err("协议报告时间门控无效".into())};let result=cache.get(&lane.id).filter(|result|result.generation==token&&result.revision==lane.revision&&result.time_gate==gate).ok_or("协议参数已变化，请重新解析后生成报告")?;selected.insert(lane.id,result.clone());}selected};
    tauri::async_runtime::spawn_blocking(move||{
        let _permit=permit;
        let lanes:Vec<_>=cached.iter().filter(|(_,item)|item.generation==token).map(|(id,item)|{
            let frames=item.result.frames.iter().enumerate().map(|(index,frame)|ProtocolReportFrame{index,t0:frame.t_start,t1:frame.t_end,summary:frame.summary.clone(),error:frame.error!=BusFrameError::None,fields:explain_frame(item.kind,&item.result.frames,index,true).into_iter().map(|text|{let (name,value)=text.split_once('：').or_else(||text.split_once(':')).map_or(("字段",text.as_str()),|(name,value)|(name,value));ProtocolReportField{name:name.into(),value:value.into(),code:String::new(),raw:false,note:String::new()}}).collect()}).collect();
            ProtocolReportLane{id:*id,title:format!("{} · {}",id,item.kind.label()),channels:item.channels.iter().filter_map(|index|dataset.traces.get(*index).map(|trace|view_channel_label(&dataset,*index,&trace.channel))).collect::<Vec<_>>().join(" / "),wave:item.channels.first().copied(),ready:true,on_plot:true,brief:item.kind.label().into(),info:item.result.info.clone(),error:item.result.error.clone(),truncated:item.result.truncated,frames}
        }).collect();
        if lanes.is_empty(){return Err("请先完成至少一路协议解析".into());}
        let windows=detail_windows(&lanes);let budget=(600_000/dataset.traces.len().max(1)).clamp(16_000,300_000);
        let mut waves=Vec::new();for (index,trace) in dataset.traces.iter().enumerate(){let view=render_indexed(trace,&dataset.extrema[index],None,None,2048)?;waves.push(ProtocolReportWave::new(view_channel_label(&dataset,index,&trace.channel),trace.y_unit.clone(),view.buckets.into_iter().map(|b|[b.x0,b.x1,b.low,b.high]).collect(),detail_columns(&trace.x,&trace.y,&windows,budget)));}
        let html=render_protocol_report(&ProtocolReport{zh:true,sources:dataset.files.iter().map(|file|file.name.clone()).collect(),byte_limit:wiparse_core::bus_decode::MAX_DECODE_BYTES,focus:focus_lane.zip(focus_frame),waves,lanes});
        let Some(path)=rfd::FileDialog::new().add_filter("协议分析报告",&["html"]).set_file_name("protocol-analysis.html").save_file()else{return Ok(None)};
        if path.exists()||!path.extension().is_some_and(|extension|extension.eq_ignore_ascii_case("html")){return Err("请选择新的 .html 文件名".into());}
        let temp=path.with_file_name(format!(".wiparse-{}.html.tmp",uuid::Uuid::new_v4()));let _cleanup=TempOutput(temp.clone());
        let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e|e.to_string())?;file.write_all(html.as_bytes()).map_err(|e|e.to_string())?;file.sync_all().map_err(|e|e.to_string())?;drop(file);
        if generation.load(Ordering::Acquire)!=token{return Err("波形已变化，未发布报告".into());}
        super::commit_file::publish_new(&temp,&path).map_err(|e|e.to_string())?;
        let warning=if open{super::report_view::show_indexed_report(&path,"open").err()}else{None};
        Ok(Some(format!("{}{}",path.file_name().unwrap_or_default().to_string_lossy(),warning.map(|message|format!(" · 已保存，{message}")).unwrap_or_default())))
    }).await.map_err(|_|"协议报告生成失败".to_string())?
}

fn decode_settings(request:&WaveDecodeRequest)->Result<BusDecodeSettings,String>{
    let kind=match request.kind.as_str(){
        "off"=>BusKind::Off,"uart"=>BusKind::Uart,"i2c"=>BusKind::I2c,"spi"=>BusKind::Spi,"i2s"=>BusKind::I2s,
        "ddsss"=>BusKind::Ddsss,"usb_pd"=>BusKind::UsbPd,"scp"=>BusKind::Scp,"ufcs"=>BusKind::Ufcs,
        _=>return Err("不支持的协议".into()),
    };
    let baud=match request.baud{
        None=>None,
        Some(value) if value.is_finite()&&(1.0..=12_000_000.0).contains(&value)=>Some(value),
        Some(_)=>return Err("波特率无效".into()),
    };
    let data_bits=request.data_bits.unwrap_or(8);
    if !(5..=9).contains(&data_bits){return Err("数据位无效".into());}
    let stop_bits=request.stop_bits.unwrap_or(1);
    if !matches!(stop_bits,1|2){return Err("停止位无效".into());}
    let parity=match request.parity.as_deref().unwrap_or("none"){
        ""|"none"=>UartParity::None,"even"=>UartParity::Even,"odd"=>UartParity::Odd,"mark"=>UartParity::Mark,"space"=>UartParity::Space,
        _=>return Err("校验位无效".into()),
    };
    let reg_bits=request.i2c_reg_bits.unwrap_or(8);
    if !matches!(reg_bits,8|16){return Err("I2C 寄存器地址宽度无效".into());}
    let spi_mode=match request.spi_mode.unwrap_or(0){0=>SpiMode::Mode0,1=>SpiMode::Mode1,2=>SpiMode::Mode2,3=>SpiMode::Mode3,_=>return Err("SPI 模式无效".into())};
    let spi_wire=match request.spi_wire.as_deref().unwrap_or("4"){
        "2"|"two"=>SpiWire::TwoWire,"4"|"four"|""=>SpiWire::FourWire,"dual"=>SpiWire::Dual,"quad"=>SpiWire::Quad,_=>return Err("SPI 线制无效".into()),
    };
    let spi_word_bits=request.spi_word_bits.unwrap_or(8);
    if !matches!(spi_word_bits,8|16|24|32){return Err("SPI 位宽无效".into());}
    let i2s_bits=request.i2s_bits.unwrap_or(16);
    if !matches!(i2s_bits,16|24|32){return Err("I2S 位宽无效".into());}
    let i2s_format=match request.i2s_format.as_deref().unwrap_or("philips"){
        ""|"philips"=>I2sFormat::Philips,"left"=>I2sFormat::LeftJustified,_=>return Err("I2S 格式无效".into()),
    };
    let ddsss_sequence=match request.ddsss_sequence.as_deref().unwrap_or("seqa"){
        "auto"=>DdsssSequence::Auto,""|"seqa"|"a"=>DdsssSequence::SeqA,"seqb"|"b"=>DdsssSequence::SeqB,"seqc"|"c"=>DdsssSequence::SeqC,"seqd"|"d"=>DdsssSequence::SeqD,_=>return Err("DDSSS 序列无效".into()),
    };
    let ddsss_extension=match request.ddsss_extension.as_deref().unwrap_or("off"){
        "auto"=>DdsssExtension::Auto,""|"off"=>DdsssExtension::Off,"on"=>DdsssExtension::On,_=>return Err("DDSSS 扩展码无效".into()),
    };
    let fop_hz=match request.fop_hz{
        None=>None,
        Some(value) if value.is_finite()&&(80_000.0..=2_000_000.0).contains(&value)=>Some(value),
        Some(_)=>return Err("FOP 需在 80000–2000000 Hz，留空则自动估计".into()),
    };
    let ufcs_baud=request.ufcs_baud.unwrap_or(115_200);
    if !matches!(ufcs_baud,38_400|57_600|115_200|230_400){return Err("UFCS 波特率只能是 38400、57600、115200 或 230400".into());}
    let scp_ui_us=request.scp_ui_us.unwrap_or(160.0);
    if !scp_ui_us.is_finite()||!(120.0..=200.0).contains(&scp_ui_us){return Err("SCP UI 需在 120–200 µs".into());}
    let threshold=match request.threshold{None=>None,Some(value) if value.is_finite()&&(-20.0..=20.0).contains(&value)=>Some(value),Some(_)=>return Err("阈值需在 -20–20 V".into())};
    let time_gate=match (request.time_start,request.time_end){
        (None,None)=>None,
        (Some(start),Some(end)) if start.is_finite()&&end.is_finite()&&start<=end=>Some((start,end)),
        _=>return Err("解码时间范围无效".into()),
    };
    let mut settings=BusDecodeSettings::default();
    settings.kind=kind;
    settings.channels=BusChannelMap{
        uart_signal:request.uart_signal,i2c_scl:request.i2c_scl,i2c_sda:request.i2c_sda,
        spi_clk:request.spi_clk,spi_mosi:request.spi_mosi,spi_miso:request.spi_miso,spi_cs:request.spi_cs,
        spi_io2:request.spi_io2,spi_io3:request.spi_io3,i2s_bclk:request.i2s_bclk,i2s_ws:request.i2s_ws,i2s_data:request.i2s_data,
        ddsss_signal:request.ddsss_signal,charging_signal:request.charging_signal,
    };
    settings.uart.baud=baud;
    settings.uart.data_bits=data_bits;
    settings.uart.parity=parity;
    settings.uart.stop_bits=stop_bits;
    settings.uart.inverted=request.inverted.unwrap_or(false);
    settings.i2c.reg_addr_bits=reg_bits;
    settings.spi.mode=spi_mode;
    settings.spi.wire=spi_wire;
    settings.spi.word_bits=spi_word_bits;
    settings.spi.msb_first=request.spi_msb_first.unwrap_or(true);
    settings.spi.cs_active_low=request.spi_cs_active_low.unwrap_or(true);
    settings.i2s.bits_per_sample=i2s_bits;
    settings.i2s.format=i2s_format;
    settings.ddsss.sequence=ddsss_sequence;
    settings.ddsss.extension=ddsss_extension;
    settings.ddsss.fop_hz=fop_hz;
    settings.ufcs.baud=ufcs_baud;
    settings.scp.ui_us=scp_ui_us;
    settings.threshold=threshold;
    settings.idle_high=request.idle_high.unwrap_or(true);
    settings.time_gate=time_gate;
    Ok(settings)
}

fn present_decode(kind:BusKind,result:&BusDecodeResult)->WaveDecodeView{
    let frames=result.frames.iter().enumerate().map(|(index,frame)|WaveFrameRow{
        index,t_start:frame.t_start,t_end:frame.t_end,summary:frame.summary.clone(),
        error:match frame.error{BusFrameError::None=>"none",BusFrameError::Checksum=>"checksum",BusFrameError::Parity=>"parity",BusFrameError::Framing=>"framing"}.into(),
        bytes:frame.bytes.len(),
        preview:preview_bytes(&frame.bytes),hex:frame.bytes.iter().map(|byte|format!("{byte:02X}")).collect::<Vec<_>>().join(" "),
    }).collect();
    WaveDecodeView{kind:kind.label().into(),info:result.info.clone(),error:localize_decode_error(result.error.clone()),truncated:result.truncated,frame_count:result.frames.len(),frames}
}

fn preview_bytes(bytes:&[u8])->String{
    const LIMIT:usize=16;
    let shown=bytes.iter().take(LIMIT).map(|byte|format!("{byte:02X}")).collect::<Vec<_>>().join(" ");
    if bytes.len()>LIMIT {format!("{shown} …")} else {shown}
}

fn localize_decode_error(error:Option<String>)->Option<String>{
    error.map(|text|{
        if let Some(what)=text.strip_prefix("Assign channel(s): ") {format!("请指定通道：{what}")}
        else if text=="Selected channel index out of range" {"所选通道不存在".into()}
        else {text}
    })
}

#[cfg(test)]
mod tests{
    #[test]fn empty_primary_view_keeps_shared_document_axes(){let trace=wiparse_core::instrument::WaveformTrace{channel:"CH1".into(),x:vec![0.,1.].into(),y:vec![0.,5.].into(),x_unit:"s".into(),y_unit:"V".into()};let view=super::render(&trace,Some(10.),Some(20.),32).unwrap();assert_eq!(view.samples,0);assert!(view.buckets.is_empty());assert_eq!((view.start,view.end),(10.,20.));}
    #[test]fn protocol_overlay_requires_current_generation_revision_and_gate_and_shares_samples(){
        use super::*;let shared=Arc::new(BusDecodeResult::default());let mut cache=BTreeMap::new();cache.insert(2,DecodeCache{time_gate:Some((1.,2.)),revision:7,generation:5,kind:BusKind::Uart,result:shared.clone(),channels:vec![0]});
        let mut request=OverlayRequest{lane:2,revision:7,start:1.,end:2.,selected:None,gate_start:Some(1.),gate_end:Some(2.),budget:1200};assert!(Arc::ptr_eq(&shared,&overlay_result(&cache,5,&request).unwrap()));assert!(overlay_result(&cache,6,&request).is_err());request.revision=8;assert!(overlay_result(&cache,5,&request).is_err());request.revision=7;request.gate_end=None;assert!(overlay_result(&cache,5,&request).is_err());request.gate_end=Some(3.);assert!(overlay_result(&cache,5,&request).is_err());
    }

    use super::*;
    #[test]
    fn open_replaces_append_respects_remaining_budget_and_failure_preserves_library(){
        let dir=std::env::temp_dir().join(format!("wiparse_open_{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(&dir).unwrap();
        let a=dir.join("a.csv");let b=dir.join("b.csv");let c=dir.join("c.csv");
        for path in [&a,&b,&c]{std::fs::write(path,"TIME,CH1\n0,1\n1,2\n2,3\n").unwrap();}
        let current=load_selected(None,vec![a.clone()],false,5,10).unwrap().unwrap();
        assert_eq!(dataset_points(&current).unwrap(),3);
        let sample=current.traces[0].y.clone();
        let error=match load_selected(Some(current.clone()),vec![b.clone()],false,5,10){Err(e)=>e,Ok(_)=>panic!("append exceeded remaining capacity")};
        assert!(error.contains("已有 3 点")&&error.contains("剩余 2 点"));
        assert!(Arc::ptr_eq(&current.traces[0].y,&sample));assert_eq!(current.files.len(),1);
        let replaced=load_selected(Some(current.clone()),vec![b.clone()],true,5,10).unwrap().unwrap();
        assert_eq!(replaced.files.len(),1);assert_eq!(replaced.files[0].name,"b.csv");
        let duplicate=load_selected(Some(current.clone()),vec![a.clone()],false,3,10).unwrap().unwrap();assert_eq!(duplicate.files.len(),1);
        let exact=load_selected(Some(current.clone()),vec![b.clone()],false,6,10).unwrap().unwrap();assert_eq!(dataset_points(&exact).unwrap(),6);
        assert!(load_selected(Some(current.clone()),vec![b.clone(),c.clone()],true,5,10).is_err());
        assert!(load_selected(Some(current.clone()),vec![dir.join("missing.csv")],true,5,10).is_err());
        assert_eq!(current.files[0].name,"a.csv");assert_eq!(current.traces[0].y.len(),3);
        assert!(load_selected(Some(current.clone()),vec![b],false,10,1).is_err());
        let _=std::fs::remove_dir_all(dir);
    }

    #[test]
    fn waveform_library_groups_sources_shares_samples_and_removes_transactionally(){
        let dir=std::env::temp_dir().join(format!("wiparse_library_{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(&dir).unwrap();
        let first=dir.join("first.csv");let second=dir.join("second.csv");
        std::fs::write(&first,b"TIME,CH1\n0,1\n0.000001,2\n").unwrap();
        std::fs::write(&second,b"TIME,CH1\n0,3\n0.000001,4\n").unwrap();
        let a=load_dataset(&first).unwrap();let b=load_dataset(&second).unwrap();
        let original_a=a.traces[0].y.clone();let original_b=b.traces[0].y.clone();
        let a_id=a.files[0].id.clone();let b_id=b.files[0].id.clone();
        let combined=merge_dataset(Some(a),b).unwrap();let listed=summary(&combined);
        assert_eq!(view_channel_label(&combined,0,"CH1"),"1 · first.csv / CH1");assert_eq!(view_channel_label(&combined,1,"CH1"),"2 · second.csv / CH1");
        assert_eq!(listed.files.len(),2);assert_eq!(listed.files[0].channels,vec![0]);assert_eq!(listed.files[1].channels,vec![1]);
        assert!(Arc::ptr_eq(&original_a,&combined.traces[0].y));assert!(Arc::ptr_eq(&original_b,&combined.traces[1].y));
        let mut same_names=combined.clone();for file in &mut same_names.files{file.name="same.csv".into();}
        assert_eq!(view_channel_label(&same_names,0,"CH1"),"1 · same.csv / CH1");assert_eq!(view_channel_label(&same_names,1,"CH1"),"2 · same.csv / CH1");
        let deduplicated=merge_dataset(Some(combined.clone()),load_dataset(&first).unwrap()).unwrap();assert_eq!(deduplicated.files.len(),2);
        assert!(remove_source(&combined,"stale-id").is_err());assert_eq!(combined.traces.len(),2);
        let remaining=remove_source(&combined,&a_id).unwrap().unwrap();assert_eq!(summary(&remaining).files[0].channels,vec![0]);assert_eq!(remaining.files[0].id,b_id);
        assert!(Arc::ptr_eq(&original_b,&remaining.traces[0].y));assert_eq!(remaining.traces[0].y[0],3.0);
        let remaining_view=render_indexed(&remaining.traces[0],&remaining.extrema[0],None,None,2).unwrap();
        assert_eq!((remaining_view.minimum,remaining_view.maximum),(3.0,4.0));
        assert!(remove_source(&remaining,&b_id).unwrap().is_none());assert!(first.is_file()&&second.is_file());
        let storage=Mutex::new(Some(combined.clone()));let epoch=AtomicU64::new(9);let cache=Mutex::new(BTreeMap::new());
        assert!(commit_dataset(&storage,&epoch,&cache,9,Some(combined.clone())).unwrap().is_some());assert_eq!(epoch.load(Ordering::Acquire),10);
        // A Close while file I/O is pending must prevent a late load from resurrecting it.
        *storage.lock().unwrap()=None;epoch.fetch_add(1,Ordering::AcqRel);
        assert!(commit_dataset(&storage,&epoch,&cache,10,Some(combined.clone())).is_err());assert!(storage.lock().unwrap().is_none());
        let mut full=combined.clone();
        for index in 2..40 {
            let path=dir.join(format!("source-{index}.csv"));std::fs::write(&path,b"TIME,CH1\n0,5\n1,6\n").unwrap();
            full=merge_dataset(Some(full),load_dataset(&path).unwrap()).unwrap();
        }
        assert_eq!(full.files.len(),40);assert_eq!(full.traces.len(),40);
        assert_ne!(summary(&full).channels[0].name,summary(&full).channels[1].name);
        let third=dir.join("extra.csv");std::fs::write(&third,b"TIME,CH1\n0,5\n1,6\n").unwrap();
        assert!(merge_dataset(Some(full.clone()),load_dataset(&third).unwrap()).is_ok());
        full.files.resize(crate::resource_policy::source_limit(),full.files[0].clone());
        assert!(merge_dataset(Some(full),load_dataset(&third).unwrap()).is_err());
        let _=std::fs::remove_dir_all(dir);
    }

    #[test]
    #[ignore = "requires WIPARSE_WAVE_SAMPLE pointing at the supplied SCP record"]
    fn supplied_scp_record_uses_core_decode_and_field_explanations(){
        let path=PathBuf::from(std::env::var_os("WIPARSE_WAVE_SAMPLE").expect("Set WIPARSE_WAVE_SAMPLE"));let data=load_dataset(&path).unwrap();
        let mut results=Vec::new();let mut clean_data=0;
        for channel in 0..data.traces.len(){
            let request:WaveDecodeRequest=serde_json::from_value(serde_json::json!({"kind":"scp","charging_signal":channel})).unwrap();
            let settings=decode_settings(&request).unwrap();let started=std::time::Instant::now();let result=decode_bus(&data.traces,&settings);
            let clean=result.frames.iter().filter(|frame|frame.error==BusFrameError::None&&!frame.bytes.is_empty()).count();clean_data+=clean;
            if let Some((index,_))=result.frames.iter().enumerate().find(|(_,frame)|!frame.bytes.is_empty()) {assert!(!explain_frame(BusKind::Scp,&result.frames,index,true).is_empty());}
            results.push(serde_json::json!({"channel":data.traces[channel].channel,"frames":result.frames.len(),"clean_data_frames":clean,"error":result.error,"decode_ms":started.elapsed().as_millis()}));
        }
        println!("REAL_SCP_RESULT {}",serde_json::json!({"file":path.file_name().unwrap().to_string_lossy(),"channels":results}));assert!(clean_data>0,"No clean SCP data frame found");
    }

    #[test]
    #[ignore = "requires WIPARSE_WAVE_SAMPLE; large real scope fixture, run explicitly"]
    fn supplied_scope_record_loads_and_views_without_copying_source_arrays(){
        let path=PathBuf::from(std::env::var_os("WIPARSE_WAVE_SAMPLE").expect("Set WIPARSE_WAVE_SAMPLE to the real waveform"));
        let started=std::time::Instant::now();let data=load_dataset(&path).unwrap();let load_ms=started.elapsed().as_millis();
        let clone=data.clone();assert!(Arc::ptr_eq(&data.traces,&clone.traces));assert!(Arc::ptr_eq(&data.extrema,&clone.extrema));
        let total:usize=data.traces.iter().map(|trace|trace.x.len()).sum();assert!(total>0);
        let mut views=Vec::new();
        for (channel,trace) in data.traces.iter().enumerate(){
            let started=std::time::Instant::now();let reference=render(trace,None,None,1024).unwrap();let scan_us=started.elapsed().as_micros();
            let started=std::time::Instant::now();let view=render_indexed(trace,&data.extrema[channel],None,None,1024).unwrap();let indexed_us=started.elapsed().as_micros();
            assert!(view.buckets.len()<=1024);assert_eq!(serde_json::to_value(&view).unwrap(),serde_json::to_value(&reference).unwrap());
            views.push(serde_json::json!({"channel":trace.channel,"points":trace.x.len(),"render_ms":indexed_us/1000,"indexed_us":indexed_us,"scan_us":scan_us}));
        }
        let index_bytes:usize=data.extrema.iter().map(ExtremaIndex::allocated_bytes).sum();
        println!("REAL_WAVE_RESULT {}",serde_json::json!({"file":path.file_name().unwrap().to_string_lossy(),"total_points":total,"load_ms":load_ms,"extrema_index_bytes":index_bytes,"views":views}));
    }

    #[test]
    fn indexed_views_preserve_every_bucket_for_nonuniform_times_spikes_and_zoom(){
        let count=100_003;
        let x:Vec<f64>=(0..count).map(|index|((index/2) as f64)*1e-6).collect();
        let mut y:Vec<f64>=(0..count).map(|index|((index*71%1021) as f64-510.)/93.).collect();
        for index in [0,255,256,257,511,512,513,count-1]{y[index]=if index%2==0{2000.}else{-3000.};}
        let extrema=ExtremaIndex::new(&y).unwrap();
        let trace=WaveformTrace{channel:"CH1".into(),x:x.into(),y:y.into(),x_unit:"s".into(),y_unit:"V".into()};
        for buckets in [1,2,17,511,1500,2048]{
            for (start,end) in [(None,None),(Some(trace.x[255]),Some(trace.x[count-257])),(Some(trace.x[50_000]),Some(trace.x[50_000]+0.00001)),(Some(trace.x[256]),Some(trace.x[256]))]{
                let original=render(&trace,start,end,buckets).unwrap();
                let indexed=render_indexed(&trace,&extrema,start,end,buckets).unwrap();
                assert_eq!(serde_json::to_value(original).unwrap(),serde_json::to_value(indexed).unwrap(),"buckets={buckets}, start={start:?}, end={end:?}");
            }
        }
    }

    #[test]
    #[ignore = "requires WIPARSE_WAVE_SAMPLE; records real-file viewport pressure evidence"]
    fn supplied_scope_viewport_stress_uses_exact_index_and_bounded_output(){
        let path=PathBuf::from(std::env::var_os("WIPARSE_WAVE_SAMPLE").expect("Set WIPARSE_WAVE_SAMPLE"));
        let data=load_dataset(&path).unwrap();let original=data.clone();
        let total:usize=data.traces.iter().map(|trace|trace.x.len()).sum();
        let index_bytes:usize=data.extrema.iter().map(ExtremaIndex::allocated_bytes).sum();
        let mut timings=Vec::new();let mut largest_output=0;
        for step in 0..200{
            let channel=step%data.traces.len();let trace=&data.traces[channel];
            let first=trace.x[0];let span=trace.x[trace.x.len()-1]-first;assert!(span>0.0);
            let width=span*(0.05+(step%11) as f64*0.075);
            let start=first+(span-width)*(step%101) as f64/100.0;let end=start+width;
            let buckets=[512,1024,1500,2048][step%4];
            let started=std::time::Instant::now();let view=render_indexed(trace,&data.extrema[channel],Some(start),Some(end),buckets).unwrap();
            timings.push(started.elapsed().as_micros());largest_output=largest_output.max(view.buckets.len());
            assert!(view.samples<=trace.x.len());assert!(view.buckets.len()<=buckets);
            if step%25==0{
                let reference=render(trace,Some(start),Some(end),buckets).unwrap();
                assert_eq!(serde_json::to_value(&view).unwrap(),serde_json::to_value(reference).unwrap());
            }
        }
        assert!(Arc::ptr_eq(&original.traces,&data.traces));assert!(Arc::ptr_eq(&original.extrema,&data.extrema));
        assert!(index_bytes<total*std::mem::size_of::<f64>()/10+data.traces.len()*64);
        timings.sort_unstable();let elapsed:u128=timings.iter().sum();
        println!("REAL_VIEWPORT_STRESS_RESULT {}",serde_json::json!({"file":path.file_name().unwrap().to_string_lossy(),"requests":timings.len(),"points":total,"extrema_index_bytes":index_bytes,"total_us":elapsed,"p50_us":timings[99],"p95_us":timings[189],"max_us":timings[199],"largest_output_buckets":largest_output,"source_arrays_shared":true}));
    }

    #[test]fn rendering_preserves_extrema_and_rejects_bad_ranges(){
        let trace=WaveformTrace{channel:"CH1".into(),x:vec![0.,1.,2.,3.].into(),y:vec![0.,9.,-4.,2.].into(),x_unit:"s".into(),y_unit:"V".into()};
        let view=render(&trace,None,None,2).unwrap();assert_eq!(view.samples,4);assert_eq!(view.minimum,-4.);assert_eq!(view.maximum,9.);assert_eq!(view.buckets.len(),2);assert_eq!(view.buckets[0].high,9.);assert_eq!(view.buckets[1].low,-4.);
        let empty=render(&trace,Some(4.),Some(5.),2).unwrap();assert_eq!(empty.samples,0);assert!(empty.buckets.is_empty());assert!(render(&trace,None,None,0).is_err());
        let point=sample(&trace,1.4).unwrap();assert_eq!(point.index,1);assert_eq!(point.y,9.);
        assert!(sample(&trace,4.).is_err());
        let stats=metrics(&trace,1.,2.).unwrap();assert_eq!(stats.count,2);assert_eq!(stats.minimum,Some(-4.));assert_eq!(stats.maximum,Some(9.));assert_eq!(stats.mean,Some(2.5));assert_eq!(stats.peak_to_peak,Some(13.));assert_eq!(stats.sample_interval,Some(1.));
        assert!(metrics(&trace,4.,5.).is_err());assert!(metrics(&trace,2.,1.).is_err());
    }
    #[test]fn actual_scope_fixtures_load_and_produce_bounded_views(){
        let root=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sample_waveforms");
        for file in ["DS1307Z/tek0000CH1.isf","Rigol_WFM_4ch/DHO824-ch1234.wfm"] {
            let data=load_dataset(&root.join(file)).unwrap();
            let channels=summary(&data);assert!(!channels.channels.is_empty());
            let view=render(&data.traces[0],None,None,512).unwrap();
            assert!(view.samples>0);assert!(view.buckets.len()<=512);assert!(view.minimum<=view.maximum);
        }
    }
    #[test]fn frequency_is_not_labelled_hertz_when_time_unit_is_not_seconds(){
        let mut trace=WaveformTrace{channel:"CH1".into(),x:(0..9).map(|i|i as f64).collect::<Vec<_>>().into(),y:vec![0.,1.,0.,-1.,0.,1.,0.,-1.,0.].into(),x_unit:"s".into(),y_unit:"V".into()};
        assert!(metrics(&trace,0.,8.).unwrap().frequency_hz.is_some());
        trace.x_unit="ms".into();assert!(metrics(&trace,0.,8.).unwrap().frequency_hz.is_none());
    }
    #[test]fn superseded_measurements_are_not_returned(){
        let trace=WaveformTrace{channel:"CH1".into(),x:vec![0.,1.,2.].into(),y:vec![1.,2.,3.].into(),x_unit:"s".into(),y_unit:"V".into()};
        let generation=AtomicU64::new(4);let request=AtomicU64::new(2);let lock=Mutex::new(());
        assert!(guarded_metrics(&trace,0.,2.,&generation,4,&request,1,&lock).is_err());
        assert_eq!(guarded_metrics(&trace,0.,2.,&generation,4,&request,2,&lock).unwrap().mean,Some(2.));
        generation.store(5,Ordering::Release);
        assert!(guarded_metrics(&trace,0.,2.,&generation,4,&request,2,&lock).is_err());
    }
    #[test]fn multichannel_fixture_renders_a_shared_range_with_bounded_output(){
        let root=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../sample_waveforms/Rigol_WFM_4ch/DHO824-ch1234.wfm");
        let data=load_dataset(&root).unwrap();assert!(data.traces.len()>=2);
        let first=render(&data.traces[0],None,None,512).unwrap();
        let second=render(&data.traces[1],Some(first.start),Some(first.end),512).unwrap();
        assert_eq!(first.x_unit,second.x_unit);assert!(first.samples>0&&second.samples>0);
        assert!(first.buckets.len()<=512&&second.buckets.len()<=512);
    }
    #[test]fn large_constant_signal_keeps_sample_count_and_value(){
        let count=250_000;
        let trace=WaveformTrace{channel:"CH1".into(),x:(0..count).map(|i|i as f64*1e-6).collect::<Vec<_>>().into(),y:vec![2.5;count].into(),x_unit:"s".into(),y_unit:"V".into()};
        let view=render(&trace,None,None,1024).unwrap();
        assert_eq!(view.samples,count);assert_eq!(view.buckets.len(),1024);
        assert!(view.buckets.iter().all(|bucket|bucket.low==2.5&&bucket.high==2.5));
    }
    #[test]fn export_never_overwrites_and_preserves_original_samples(){
        let root=std::env::temp_dir().join(format!("wiparse-wave-export-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(&root).unwrap();
        let trace=WaveformTrace{channel:"CH1".into(),x:vec![0.,1.,2.].into(),y:vec![1.,-2.,3.].into(),x_unit:"s".into(),y_unit:"V".into()};
        let target=root.join("wave.csv");export_new(&trace,&target,commit_export).unwrap();let original=std::fs::read(&target).unwrap();
        assert!(export_new(&trace,&target,commit_export).is_err());assert_eq!(std::fs::read(&target).unwrap(),original);
        let cancelled=root.join("cancelled.csv");
        assert!(export_new(&trace,&cancelled,|_,_|Err("会话已改变".into())).is_err());
        assert!(!cancelled.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(),1);
        let loaded=load_waveform_file_all(&target).unwrap();assert_eq!(loaded[0].y.as_ref(),trace.y.as_ref());
        let parent=std::env::temp_dir().canonicalize().unwrap();let canonical=root.canonicalize().unwrap();assert!(canonical.starts_with(parent));std::fs::remove_dir_all(canonical).unwrap();
    }
    #[test]fn legacy_export_formats_roundtrip_without_overwriting_or_changing_samples(){
        let root=std::env::temp_dir().join(format!("wiparse-wave-formats-{}",uuid::Uuid::new_v4()));std::fs::create_dir_all(&root).unwrap();
        let x:Vec<f64>=(0..128).map(|index|index as f64*1e-6).collect();
        let y:Vec<f64>=(0..128).map(|index|(index as f64*0.1).sin()*3.3).collect();
        let trace=WaveformTrace{channel:"CH1".into(),x:x.clone().into(),y:y.clone().into(),x_unit:"s".into(),y_unit:"V".into()};
        for format in ["csv","isf","wfm"] {
            let path=root.join(format!("wave.{format}"));
            export_format_new(&trace,&path,format,commit_export).unwrap();
            let bytes=std::fs::read(&path).unwrap();
            assert!(export_format_new(&trace,&path,format,commit_export).is_err());
            assert_eq!(std::fs::read(&path).unwrap(),bytes);
            let loaded=load_waveform_file_all(&path).unwrap();assert_eq!(loaded[0].y.len(),y.len());
            let low=y.iter().copied().fold(f64::INFINITY,f64::min);let high=y.iter().copied().fold(f64::NEG_INFINITY,f64::max);
            let quantization=match format {"isf"=>low.abs().max(high.abs())/127.0/2.0,"wfm"=>(high-low)/65534.0/2.0,_=>0.0};
            for ((want_x,want_y),(actual_x,actual_y)) in x.iter().zip(&y).zip(loaded[0].x.iter().zip(loaded[0].y.iter())) {assert!((actual_x-want_x).abs()<1e-10,"{format} x");assert!((actual_y-want_y).abs()<=quantization+1e-9,"{format} y exceeds half an encoded step");}
            let cancelled=root.join(format!("cancelled.{format}"));
            assert!(export_format_new(&trace,&cancelled,format,|_,_|Err("session changed".into())).is_err());assert!(!cancelled.exists());
        }
        assert!(export_format_new(&trace,&root.join("wrong.csv"),"isf",commit_export).is_err());
        assert_eq!(trace.x.as_ref(),x.as_slice());assert_eq!(trace.y.as_ref(),y.as_slice());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(),3);
        let canonical=root.canonicalize().unwrap();assert!(canonical.starts_with(std::env::temp_dir().canonicalize().unwrap()));std::fs::remove_dir_all(canonical).unwrap();
    }
    #[test]fn native_export_rejects_timebase_loss_before_creating_a_file(){
        let trace=WaveformTrace{channel:"CH1".into(),x:vec![0.0,1.0,3.0].into(),y:vec![0.0,1.0,0.0].into(),x_unit:"s".into(),y_unit:"V".into()};
        assert!(validate_export_timebase(&trace,"csv").is_ok());assert!(validate_export_timebase(&trace,"isf").is_err());assert!(validate_export_timebase(&trace,"wfm").is_err());
        let start=1e9+0.0000123;let mut high_origin=trace.clone();high_origin.x=vec![start,start+1e-6,start+2e-6].into();
        assert!(validate_export_timebase(&high_origin,"isf").is_err());assert!(validate_export_timebase(&high_origin,"wfm").is_ok());
    }
    #[test]fn decode_settings_keep_core_defaults_and_reject_bad_input(){
        let request=WaveDecodeRequest{kind:"uart".into(),uart_signal:Some(1),baud:Some(115200.),data_bits:Some(8),parity:Some("even".into()),stop_bits:Some(1),time_start:Some(0.),time_end:Some(1.),idle_high:Some(false),..Default::default()};
        let settings=decode_settings(&request).unwrap();
        assert_eq!(settings.kind,BusKind::Uart);assert_eq!(settings.channels.uart_signal,Some(1));assert_eq!(settings.uart.baud,Some(115200.));assert_eq!(settings.uart.parity,UartParity::Even);assert!(!settings.idle_high);assert_eq!(settings.time_gate,Some((0.,1.)));
        assert!(decode_settings(&WaveDecodeRequest{kind:"other".into(),..Default::default()}).is_err());
        assert!(decode_settings(&WaveDecodeRequest{kind:"uart".into(),baud:Some(0.),..Default::default()}).is_err());
        assert!(decode_settings(&WaveDecodeRequest{kind:"uart".into(),time_start:Some(1.),..Default::default()}).is_err());
        let spi=decode_settings(&WaveDecodeRequest{kind:"spi".into(),spi_wire:Some("quad".into()),spi_mode:Some(3),spi_word_bits:Some(16),spi_msb_first:Some(false),spi_cs_active_low:Some(false),spi_io2:Some(4),spi_io3:Some(5),..Default::default()}).unwrap();
        assert_eq!(spi.spi.wire,SpiWire::Quad);assert_eq!(spi.spi.mode,SpiMode::Mode3);assert_eq!(spi.spi.word_bits,16);assert!(!spi.spi.msb_first);assert!(!spi.spi.cs_active_low);assert_eq!(spi.channels.spi_io2,Some(4));
        let i2s=decode_settings(&WaveDecodeRequest{kind:"i2s".into(),i2s_bits:Some(24),i2s_format:Some("left".into()),..Default::default()}).unwrap();
        assert_eq!(i2s.i2s.bits_per_sample,24);assert_eq!(i2s.i2s.format,I2sFormat::LeftJustified);
        let ddsss=decode_settings(&WaveDecodeRequest{kind:"ddsss".into(),ddsss_sequence:Some("auto".into()),ddsss_extension:Some("on".into()),fop_hz:Some(128_000.),..Default::default()}).unwrap();
        assert_eq!(ddsss.ddsss.sequence,DdsssSequence::Auto);assert_eq!(ddsss.ddsss.extension,DdsssExtension::On);assert_eq!(ddsss.ddsss.fop_hz,Some(128_000.));
        assert!(decode_settings(&WaveDecodeRequest{kind:"ddsss".into(),fop_hz:Some(1_000.),..Default::default()}).is_err());
        let ufcs=decode_settings(&WaveDecodeRequest{kind:"ufcs".into(),ufcs_baud:Some(57_600),threshold:Some(0.6),..Default::default()}).unwrap();
        assert_eq!(ufcs.ufcs.baud,57_600);assert_eq!(ufcs.threshold,Some(0.6));
        assert!(decode_settings(&WaveDecodeRequest{kind:"ufcs".into(),ufcs_baud:Some(9600),..Default::default()}).is_err());
        let scp=decode_settings(&WaveDecodeRequest{kind:"scp".into(),scp_ui_us:Some(150.),..Default::default()}).unwrap();
        assert_eq!(scp.scp.ui_us,150.);
        assert!(decode_settings(&WaveDecodeRequest{kind:"scp".into(),scp_ui_us:Some(80.),..Default::default()}).is_err());
        assert!(decode_settings(&WaveDecodeRequest{kind:"usb_pd".into(),threshold:Some(30.),..Default::default()}).is_err());
        let empty=present_decode(BusKind::Uart,&BusDecodeResult{error:Some("Assign channel(s): UART signal".into()),..Default::default()});
        assert_eq!(empty.error.as_deref(),Some("请指定通道：UART signal"));assert_eq!(empty.frame_count,0);
    }
    #[test]fn indexed_wave_accepts_only_waveform_extensions(){
        assert!(waveform_extension(Path::new("capture.CSV")));
        assert!(waveform_extension(Path::new(r"D:\runs\scope.wfm")));
        assert!(waveform_extension(Path::new("note.txt")));
        assert!(!waveform_extension(Path::new("report.md")));
        assert!(!waveform_extension(Path::new("shot.png")));
        assert!(!waveform_extension(Path::new("trace")));
    }
}
