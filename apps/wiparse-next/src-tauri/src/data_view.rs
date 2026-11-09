//! Offline data extraction using the same token parser as the original GUI.
use std::{io::{BufWriter,Read,Write},path::{Path,PathBuf},sync::{Arc,Mutex,atomic::{AtomicBool,AtomicU64,Ordering}}};
use serde::Serialize;
use wiparse_core::data_extract::extract_first_filter_value;
use wiparse_core::data_extract_cache::{self as cache,WidaPayload,WidaSeriesData,WidaHeader};



const MAX_BUCKETS:usize=1500;

#[derive(Default)]
pub struct DataState { data:Arc<Mutex<Option<Dataset>>>,busy:Arc<AtomicBool>,generation:Arc<AtomicU64>,live:Arc<Mutex<LiveSession>>,live_generation:Arc<AtomicU64>,cache_busy:Arc<AtomicBool> }
#[derive(Clone)]
struct Dataset {name:String,text:Arc<String>,fingerprint:String,bytes:u64,wida:Option<Arc<WidaPayload>>,path:PathBuf,frame_cache:Arc<Mutex<Option<FrameIndex>>>}
struct FrameIndex {header:String,starts:Arc<Vec<usize>>}
#[derive(Serialize)]
pub struct DataSummary {filename:String,bytes:u64,meta:Option<WidaHeader>,preview:Option<DataResult>}
fn dataset_summary(data:&Dataset)->DataSummary{DataSummary{filename:data.name.clone(),bytes:data.bytes,meta:data.wida.as_ref().map(|p|p.meta.clone()),preview:data.wida.as_ref().and_then(|p|present_wida(p,None).ok())}}
#[derive(Serialize)]
pub struct DataBucket {frame:usize,low:f64,high:f64}
#[derive(Serialize)]
pub struct DataSeries {filter:String,count:usize,minimum:Option<f64>,maximum:Option<f64>,buckets:Vec<DataBucket>}
#[derive(Serialize)]
pub struct DataResult {frames:usize,range_start:usize,range_end:usize,series:Vec<DataSeries>}
struct BusyGuard(Arc<AtomicBool>);
impl Drop for BusyGuard {fn drop(&mut self){self.0.store(false,Ordering::Release);}}
struct TempOutput(PathBuf);
impl Drop for TempOutput {fn drop(&mut self){let _=std::fs::remove_file(&self.0);}}

fn present_wida(payload:&WidaPayload,range:Option<(usize,usize)>)->Result<DataResult,String>{
    let frames=payload.meta.frame_count as usize;
    let first=payload.series.iter().filter_map(|series|series.points.first().map(|p|p[0] as usize)).min().unwrap_or(0);
    let (lo,hi)=range.unwrap_or((first,frames));
    if lo>hi||hi>frames||(lo==hi&&frames>0){return Err("显示帧范围超出提取文件".into());}
    let size=(hi-lo).div_ceil(MAX_BUCKETS).max(1);
    let series=payload.series.iter().map(|source|{
        let mut item=DataSeries{filter:source.label.clone(),count:0,minimum:None,maximum:None,buckets:Vec::new()};
        let left=source.points.partition_point(|p|p[0]<(lo as f64));let right=source.points.partition_point(|p|p[0]<(hi as f64));
        for point in &source.points[left..right]{let index=point[0] as usize;let value=point[1];item.count+=1;item.minimum=Some(item.minimum.map_or(value,|v|v.min(value)));item.maximum=Some(item.maximum.map_or(value,|v|v.max(value)));
            if let Some(last)=item.buckets.last_mut().filter(|last|(last.frame-lo)/size==(index-lo)/size){last.low=last.low.min(value);last.high=last.high.max(value);}else{item.buckets.push(DataBucket{frame:index,low:value,high:value});}
        }item
    }).collect();
    Ok(DataResult{frames,range_start:lo,range_end:hi,series})
}

fn series_payload(labels:&[String],kinds:&[String],columns:Vec<Vec<[f64;2]>>,header:&str,frames:usize,source:String,fingerprint:String)->WidaPayload{
    let series=labels.iter().enumerate().zip(columns).map(|((index,label),points)|{
        let minimum=points.iter().map(|p|p[1]).reduce(f64::min).unwrap_or(0.0);let maximum=points.iter().map(|p|p[1]).reduce(f64::max).unwrap_or(0.0);
        WidaSeriesData{label:label.clone(),kind:kinds.get(index).cloned().unwrap_or("voltage".into()),y_scale:1.0,y_offset:0.0,y_min:minimum,y_max:maximum,points}
    }).collect();
    WidaPayload{meta:WidaHeader{source_key:source,content_fp:fingerprint,header:header.into(),filters:labels.to_vec(),kinds:kinds.to_vec(),frame_count:frames as u32,series:Vec::new()},series}
}

fn kinds_valid(kinds:&[String],count:usize)->bool{kinds.len()==count&&kinds.iter().all(|kind|matches!(kind.as_str(),"voltage"|"current"|"temperature"))}

#[tauri::command]
pub async fn data_save_wida(state:tauri::State<'_,DataState>,header:String,filters:Vec<String>,kinds:Vec<String>,live_session:Option<u64>,automatic:bool)->Result<Option<String>,String>{
    if !kinds_valid(&kinds,filters.len()){return Err("数据类型无效".into());}
    if automatic&&live_session.is_none(){return Err("自动缓存只用于实时会话".into());}
    if state.cache_busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有缓存保存任务".into());}
    let permit=BusyGuard(state.cache_busy.clone());
    let generation=state.generation.clone();let token=generation.load(Ordering::Acquire);
    let dataset=if live_session.is_none(){Some(state.data.lock().map_err(|_|"数据会话异常")?.clone().ok_or("请先打开文件")?)}else{None};
    let live_generation=state.live_generation.clone();
    let live=state.live.clone();let expected_live=state.live_generation.load(Ordering::Acquire);
    if live_session.is_some_and(|session|session!=expected_live){return Err("DATA.SESSION_CHANGED".into());}
    tauri::async_runtime::spawn_blocking(move||{
        let _permit=permit;
        if live_session.is_some()&&live_generation.load(Ordering::Acquire)!=expected_live{return Err("DATA.SESSION_CHANGED".into());}
        let payload=if let Some(dataset)=dataset{
            if let Some(payload)=dataset.wida{payload}else{
                validate(&header,&filters)?;let starts=frame_starts(&dataset.text,&header)?;if starts.len().saturating_mul(filters.len()).saturating_mul(16).saturating_add(starts.len().saturating_mul(8)).saturating_add(dataset.bytes as usize)>super::resource_policy::bytes(){return Err("保存完整提取数据超出当前内存预算，请调整设置或缩小输入".into())}
                let columns=filters.iter().map(|filter|starts.iter().enumerate().filter_map(|(index,start)|extract_first_filter_value(&dataset.text[*start..starts.get(index+1).copied().unwrap_or(dataset.text.len())],filter).map(|value|[index as f64,value])).collect()).collect();
                Arc::new(series_payload(&filters,&kinds,columns,&header,starts.len(),dataset.path.to_string_lossy().into_owned(),dataset.fingerprint.clone()))
            }
        }else{
            let guard=live.lock().map_err(|_|"实时数据异常")?;
            if guard.header!=header||guard.filters!=filters{return Err("筛选条件已变化".into());}
            let frames=guard.columns.first().map_or(0,Vec::len);
            let columns=guard.columns.iter().map(|column|column.iter().enumerate().filter_map(|(index,value)|value.map(|v|[index as f64,v])).collect()).collect();
            Arc::new(series_payload(&filters,&kinds,columns,&header,frames,"serial:live".into(),format!("next-live-{expected_live}")))
        };
        let path=if automatic{cache::ensure_cache_dir().map_err(|e|e.to_string())?;static RUN_ID:std::sync::OnceLock<String>=std::sync::OnceLock::new();cache::live_cache_path(&format!("next-{}-{expected_live}",RUN_ID.get_or_init(||uuid::Uuid::new_v4().to_string())))}else{let Some(path)=rfd::FileDialog::new().add_filter("WiParse 提取数据",&["wida"]).set_file_name("data-analysis.wida").save_file()else{return Ok(None)};if path.exists(){return Err("目标文件已存在，请使用新文件名".into());}path};
        let temp=path.with_file_name(format!(".wiparse-{}.wida.tmp",uuid::Uuid::new_v4()));let _cleanup=TempOutput(temp.clone());
        cache::save_wida(&temp,&payload)?;
        std::fs::File::open(&temp).and_then(|file|file.sync_all()).map_err(|e|e.to_string())?;
        if live_session.is_none()&&generation.load(Ordering::Acquire)!=token{return Err("文件已切换，未发布缓存".into());}
        if live_session.is_some()&&live_generation.load(Ordering::Acquire)!=expected_live{return Err("DATA.SESSION_CHANGED".into());}
        if automatic{std::fs::rename(&temp,&path).map_err(|e|e.to_string())?;}else{super::commit_file::publish_new(&temp,&path).map_err(|e|e.to_string())?;}
        Ok(Some(path.file_name().unwrap_or_default().to_string_lossy().into_owned()))
    }).await.map_err(|_|"提取数据保存失败".to_string())?
}

#[tauri::command]
pub async fn data_open(state:tauri::State<'_,DataState>,browser:tauri::State<'_,super::offline_browser::OfflineBrowserState>,browser_selection:Option<super::offline_browser::OfflineFile>)->Result<Option<DataSummary>,String>{
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有文件操作正在进行".into());}
    let _token=state.generation.fetch_add(1,Ordering::AcqRel)+1;let generation=state.generation.clone();let data=state.data.clone();let busy=state.busy.clone();let browser=browser.data.clone();
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=BusyGuard(busy);
        let path=if let Some(selection)=browser_selection{super::offline_browser::resolve_file(&browser,"data",&selection)?}else{let Some(path)=rfd::FileDialog::new().add_filter("日志 / 提取数据",&["txt","log","csv","wida"]).pick_file()else{return Ok(None)};path};
        let dataset=load_file(&path)?;let summary=dataset_summary(&dataset);
        let mut guard=data.lock().map_err(|_|"数据会话异常")?;
        if generation.load(Ordering::Acquire)!=_token{return Err("文件打开已取消".into());}
        *guard=Some(dataset);Ok(Some(summary))
    }).await.map_err(|_|"文件打开任务失败".to_string())?
}

#[tauri::command]
pub fn data_close(state:tauri::State<'_,DataState>)->Result<(),String>{
    let mut data=state.data.lock().map_err(|_|"数据会话异常")?;state.generation.fetch_add(1,Ordering::AcqRel);*data=None;Ok(())
}

#[tauri::command]
pub fn data_status(state:tauri::State<'_,DataState>)->Result<Option<DataSummary>,String>{
    let guard=state.data.lock().map_err(|_|"数据会话异常")?;Ok(guard.as_ref().map(|data|dataset_summary(data)))
}

#[tauri::command]
pub async fn data_extract(state:tauri::State<'_,DataState>,header:String,filters:Vec<String>)->Result<DataResult,String>{
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有数据操作正在进行".into());}
    let permit=BusyGuard(state.busy.clone());let generation=state.generation.clone();
    let (token,dataset)={let guard=state.data.lock().map_err(|_|"数据会话异常")?;(generation.load(Ordering::Acquire),guard.clone().ok_or("请先打开日志文件")?)};
    tauri::async_runtime::spawn_blocking(move||{let _guard=permit;let result=extract_cached(&dataset,&header,&filters,None)?;if generation.load(Ordering::Acquire)!=token{return Err("文件已切换，请重新提取".into());}Ok(result)})
        .await.map_err(|_|"数据提取任务失败".to_string())?
}

#[tauri::command]
pub async fn data_view_range(state:tauri::State<'_,DataState>,header:String,filters:Vec<String>,start:usize,end:usize)->Result<DataResult,String>{
    if start>=end{return Err("显示帧范围无效".into());}
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有数据操作正在进行".into());}
    let permit=BusyGuard(state.busy.clone());let generation=state.generation.clone();
    let (token,dataset)={let guard=state.data.lock().map_err(|_|"数据会话异常")?;(generation.load(Ordering::Acquire),guard.clone().ok_or("请先打开日志文件")?)};
    tauri::async_runtime::spawn_blocking(move||{let _guard=permit;let result=extract_cached(&dataset,&header,&filters,Some((start,end)))?;if generation.load(Ordering::Acquire)!=token{return Err("文件已切换，请重新提取".into());}Ok(result)}).await.map_err(|_|"数据提取任务失败".to_string())?
}

#[tauri::command]
pub async fn data_export_csv(state:tauri::State<'_,DataState>,header:String,filters:Vec<String>)->Result<Option<String>,String>{
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有数据操作正在进行".into());}
    let permit=BusyGuard(state.busy.clone());let generation=state.generation.clone();let data=state.data.clone();
    let (token,dataset)={let guard=data.lock().map_err(|_|"数据会话异常")?;(generation.load(Ordering::Acquire),guard.clone().ok_or("请先打开日志文件")?)};
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=permit;let Some(path)=rfd::FileDialog::new().add_filter("CSV 文件",&["csv"]).set_file_name("data-analysis.csv").save_file() else{return Ok(None)};
        if !path.extension().and_then(|s|s.to_str()).is_some_and(|s|s.eq_ignore_ascii_case("csv")){return Err("请选择 CSV 文件名".into());}
        let parent=path.parent().ok_or("导出目录无效")?;let temp=parent.join(format!(".wiparse-data-{}.tmp",uuid::Uuid::new_v4()));
        let file=std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e|format!("无法创建导出临时文件：{e}"))?;
        let _cleanup=TempOutput(temp.clone());let mut writer=BufWriter::new(file);if let Some(payload)=&dataset.wida{write_wida_csv(&mut writer,payload)?;}else{write_csv(&mut writer,&dataset.text,&header,&filters)?;}
        writer.flush().map_err(|e|format!("写入失败：{e}"))?;writer.get_ref().sync_all().map_err(|e|format!("保存失败：{e}"))?;drop(writer);
        let _session=data.lock().map_err(|_|"数据会话异常")?;if generation.load(Ordering::Acquire)!=token{return Err("日志文件已切换，请重新导出".into());}
        super::commit_file::publish_new(&temp,&path).map_err(|e|if e.kind()==std::io::ErrorKind::AlreadyExists{"目标文件已存在，请选择其他文件名".to_string()}else{format!("无法安全创建导出文件：{e}")})?;
        Ok(Some(path.file_name().and_then(|s|s.to_str()).unwrap_or("data-analysis.csv").to_owned()))
    }).await.map_err(|_|"数据导出任务失败".to_string())?
}

const MAX_LIVE_TAIL:usize=256*1024;
const MAX_LIVE_POINTS:usize=200_000;
const MAX_LIVE_LINES:usize=2_000;
const MAX_LIVE_LINE:usize=65_536;

#[derive(Default)]
struct LiveSession { revision:u64,tail:String,header:String,filters:Vec<String>,columns:Vec<Vec<Option<f64>>>,capped:bool,discarded_tail_bytes:usize }

#[derive(Serialize)]
pub struct LiveResult {pub revision:u64, frames:usize,range_start:usize,range_end:usize,series:Vec<DataSeries>,pending:bool,capped:bool,discarded_tail_bytes:usize }

#[tauri::command]
pub fn data_live_reset(state:tauri::State<'_,DataState>)->Result<u64,String>{reset_live(&state)}
fn reset_live(state:&DataState)->Result<u64,String>{
    let mut session=state.live.lock().map_err(|_|"数据会话异常")?;
    *session=LiveSession::default();Ok(state.live_generation.fetch_add(1,Ordering::AcqRel)+1)
}

#[tauri::command]
pub fn data_live_push(state:tauri::State<'_,DataState>,session:u64,lines:Vec<String>,header:String,filters:Vec<String>)->Result<LiveResult,String>{push_live(&state,session,&lines,&header,&filters)}
fn push_live(state:&DataState,session:u64,lines:&[String],header:&str,filters:&[String])->Result<LiveResult,String>{
    let mut current=state.live.lock().map_err(|_|"数据会话异常")?;
    if state.live_generation.load(Ordering::Acquire)!=session{return Err("DATA.SESSION_CHANGED".into());}
    live_push(&mut current,lines,header,filters)
}

#[tauri::command]
pub fn data_live_range(state:tauri::State<'_,DataState>,session:u64,header:String,filters:Vec<String>,start:usize,end:usize)->Result<LiveResult,String>{
    let current=state.live.lock().map_err(|_|"数据会话异常")?;
    if state.live_generation.load(Ordering::Acquire)!=session{return Err("DATA.SESSION_CHANGED".into());}
    if current.header!=header||current.filters!=filters{return Err("筛选条件已变化，请等待下一次读取".into());}
    present(&current,Some((start,end)))
}

fn live_header_starts(text:&str,header:&str)->Vec<usize>{
    let mut starts=Vec::new();let mut search=0usize;
    while !header.is_empty()&&search<text.len(){
        let Some(relative)=text[search..].find(header) else{break};
        let absolute=search+relative;starts.push(absolute);search=absolute+header.len().max(1);
        if starts.len()>super::resource_policy::points(){break;}
    }
    starts
}

fn trim_live_tail(tail:&mut String)->usize{
    if tail.len()<=MAX_LIVE_TAIL{return 0;}
    let mut cut=tail.len()-MAX_LIVE_TAIL/2;
    while cut<tail.len()&&!tail.is_char_boundary(cut){cut+=1;}
    *tail=tail[cut..].to_owned();
    cut
}

fn commit_live_frames(session:&mut LiveSession){
    let starts=live_header_starts(&session.tail,&session.header);
    if starts.len()<2{return;}
    let Some(&last)=starts.last() else{return};
    let complete=&starts[..starts.len()-1];
    session.revision=session.revision.saturating_add(complete.len() as u64);
    if session.columns.len()!=session.filters.len(){session.columns=vec![Vec::new();session.filters.len()];}
    for (index,start) in complete.iter().copied().enumerate(){
        let end=complete.get(index+1).copied().unwrap_or(last);
        let frame=&session.tail[start..end];
        for (column,filter) in session.columns.iter_mut().zip(session.filters.iter()){column.push(extract_first_filter_value(frame,filter));}
    }
    session.tail=session.tail[last..].to_owned();
    let overflow=session.columns.first().map(|column|column.len().saturating_sub(MAX_LIVE_POINTS)).unwrap_or(0);
    if overflow>0{session.capped=true;for column in &mut session.columns{column.drain(..overflow);}}
}

fn live_push(session:&mut LiveSession,lines:&[String],header:&str,filters:&[String])->Result<LiveResult,String>{
    validate(header,filters)?;
    if lines.len()>MAX_LIVE_LINES||lines.iter().any(|line|line.len()>MAX_LIVE_LINE||line.contains('\0')){return Err("实时日志批次无效".into());}
    if session.header!=header||session.filters!=filters{
        session.header=header.to_owned();session.filters=filters.to_vec();session.columns=vec![Vec::new();filters.len()];session.tail.clear();session.capped=false;session.discarded_tail_bytes=0;session.revision=0;
    }
    for line in lines{
        if !session.tail.is_empty()&&!session.tail.ends_with('\n'){session.tail.push('\n');}
        session.tail.push_str(line);session.tail.push('\n');
        // Commit complete frames before applying the partial-frame bound. A
        // large batch of small valid frames must never be trimmed as a tail.
        if session.tail.len()>MAX_LIVE_TAIL/2 {
            commit_live_frames(session);
            session.discarded_tail_bytes=session.discarded_tail_bytes.saturating_add(trim_live_tail(&mut session.tail));
        }
    }
    commit_live_frames(session);
    present(session,None)
}

fn present(session:&LiveSession,range:Option<(usize,usize)>)->Result<LiveResult,String>{
    let frames=session.columns.first().map(|column|column.len()).unwrap_or(0);
    let (lo,hi)=range.unwrap_or((0,frames));
    if lo>hi||hi>frames||(lo==hi&&frames>0){return Err("显示帧范围超出数据".into());}
    let bucket_size=(hi-lo).div_ceil(MAX_BUCKETS).max(1);
    let mut series=Vec::with_capacity(session.filters.len());
    if session.columns.is_empty(){
        series.extend(session.filters.iter().map(|filter|DataSeries{filter:filter.clone(),count:0,minimum:None,maximum:None,buckets:Vec::new()}));
    }
    for (filter,column) in session.filters.iter().zip(&session.columns){
        let mut item=DataSeries{filter:filter.clone(),count:0,minimum:None,maximum:None,buckets:Vec::new()};
        for (index,value) in column.iter().copied().enumerate().skip(lo).take(hi-lo){
            let Some(value)=value else{continue};
            item.count+=1;item.minimum=Some(item.minimum.map_or(value,|current|current.min(value)));item.maximum=Some(item.maximum.map_or(value,|current|current.max(value)));
            let bucket=(index-lo)/bucket_size;
            if let Some(last)=item.buckets.last_mut().filter(|last|(last.frame-lo)/bucket_size==bucket){last.low=last.low.min(value);last.high=last.high.max(value);}
            else{item.buckets.push(DataBucket{frame:index,low:value,high:value});}
        }
        series.push(item);
    }
    let pending=live_header_starts(&session.tail,&session.header).len()==1;
    Ok(LiveResult{revision:session.revision,frames,range_start:lo,range_end:hi,series,pending,capped:session.capped,discarded_tail_bytes:session.discarded_tail_bytes})
}

fn write_wida_csv(mut writer:impl Write,payload:&WidaPayload)->Result<(),String>{
    write!(writer,"frame").map_err(|e|e.to_string())?;
    for series in &payload.series{write!(writer,",{}",csv_header(&series.label)).map_err(|e|e.to_string())?;}writeln!(writer).map_err(|e|e.to_string())?;
    let mut positions=vec![0usize;payload.series.len()];
    loop{
        let Some(frame)=payload.series.iter().zip(&positions).filter_map(|(series,index)|series.points.get(*index).map(|point|point[0] as usize)).min()else{break};
        write!(writer,"{}",frame+1).map_err(|e|e.to_string())?;
        for (series,index) in payload.series.iter().zip(&mut positions){write!(writer,",").map_err(|e|e.to_string())?;if let Some(point)=series.points.get(*index).filter(|point|point[0] as usize==frame){write!(writer,"{}",point[1]).map_err(|e|e.to_string())?;*index+=1;}}
        writeln!(writer).map_err(|e|e.to_string())?;
    }Ok(())
}

fn csv_header(text:&str)->String{
    let safe=if text.trim_start().starts_with(['=','+','-','@'])||text.starts_with(['\t','\r','\n']){format!("'{text}")}else{text.to_owned()};
    format!("\"{}\"",safe.replace('"',"\"\""))
}
fn write_csv(mut writer:impl Write,text:&str,header:&str,filters:&[String])->Result<(),String>{
    validate(header,filters)?;write!(writer,"frame").map_err(|e|format!("写入失败：{e}"))?;
    for filter in filters{write!(writer,",{}",csv_header(filter)).map_err(|e|format!("写入失败：{e}"))?;}writeln!(writer).map_err(|e|format!("写入失败：{e}"))?;
    let mut starts=Vec::new();for (index,_) in text.match_indices(header){starts.push(index);if starts.len()>super::resource_policy::points(){return Err("帧索引超出内存预算，请缩小数据范围或增加预算".into());}}
    for (index,start) in starts.iter().copied().enumerate(){let end=starts.get(index+1).copied().unwrap_or(text.len());let frame=&text[start..end];write!(writer,"{}",index+1).map_err(|e|format!("写入失败：{e}"))?;
        for filter in filters{write!(writer,",").map_err(|e|format!("写入失败：{e}"))?;if let Some(value)=extract_first_filter_value(frame,filter){write!(writer,"{value}").map_err(|e|format!("写入失败：{e}"))?;}}writeln!(writer).map_err(|e|format!("写入失败：{e}"))?;
    }Ok(())
}

fn load_file(path:&Path)->Result<Dataset,String>{
    if path.extension().is_some_and(|extension|extension.eq_ignore_ascii_case("wida")){
        if std::fs::metadata(path).map_err(|e|e.to_string())?.len()>super::resource_policy::bytes() as u64/2{return Err("提取文件超出当前内存预算，请调整设置".into())}
        let payload=cache::load_wida(path)?;
        if payload.series.len()>8{return Err("提取文件超过 8 个数据系列".into());}
        return Ok(Dataset{name:path.file_name().unwrap_or_default().to_string_lossy().into_owned(),bytes:std::fs::metadata(path).map_err(|e|e.to_string())?.len(),text:Arc::new(String::new()),fingerprint:cache::file_content_fp(path),wida:Some(Arc::new(payload)),path:path.to_owned(),frame_cache:Arc::new(Mutex::new(None))});
    }
    let file=super::resource_policy::open_stable(path)?;
    let metadata=file.metadata().map_err(|e|format!("无法读取文件：{e}"))?;
    if !metadata.is_file()||metadata.len()>(super::resource_policy::bytes()/2) as u64{return Err("文本超出当前内存预算，请调整设置或选择更小的文件".into());}
    let mut bytes=Vec::new();bytes.try_reserve_exact(metadata.len() as usize).map_err(|_|"无法分配文本内存，请减少已打开的数据")?;file.take((super::resource_policy::bytes()/2) as u64+1).read_to_end(&mut bytes).map_err(|e|format!("无法读取文件：{e}"))?;if bytes.len() as u64>(super::resource_policy::bytes()/2) as u64{return Err("文本超出当前内存预算".into());}
    let size=bytes.len() as u64;let text=String::from_utf8(bytes).map_err(|_|"文件不是有效的 UTF-8 文本".to_string())?;
    Ok(Dataset{name:path.file_name().and_then(|s|s.to_str()).unwrap_or("日志文件").to_owned(),bytes:size,fingerprint:{use sha2::Digest;format!("sha256:{:x}",sha2::Sha256::digest(text.as_bytes()))},text:Arc::new(text),wida:None,path:path.to_owned(),frame_cache:Arc::new(Mutex::new(None))})
}
fn validate(header:&str,filters:&[String])->Result<(),String>{
    if header.is_empty()||header.len()>256||filters.is_empty()||filters.len()>8||filters.iter().any(|f|f.is_empty()||f.len()>256){return Err("请填写帧头和 1 至 8 个有效筛选词".into());}
    let unique:std::collections::HashSet<_>=filters.iter().collect();if unique.len()!=filters.len(){return Err("筛选词不能重复".into());}Ok(())
}
#[cfg(test)]
fn extract(text:&str,header:&str,filters:&[String])->Result<DataResult,String>{
    extract_range(text,header,filters,None)
}
#[cfg(test)]
fn extract_range(text:&str,header:&str,filters:&[String],range:Option<(usize,usize)>)->Result<DataResult,String>{
    validate(header,filters)?;
    let starts=frame_starts(text,header)?;extract_indexed(text,&starts,filters,range)
}
fn frame_starts(text:&str,header:&str)->Result<Vec<usize>,String>{
    let mut starts=Vec::new();for (index,_) in text.match_indices(header){starts.push(index);if starts.len()>super::resource_policy::points(){return Err("帧索引超出内存预算，请缩小数据范围或增加预算".into());}}Ok(starts)
}
fn extract_cached(dataset:&Dataset,header:&str,filters:&[String],range:Option<(usize,usize)>)->Result<DataResult,String>{
    if let Some(payload)=&dataset.wida{return present_wida(payload,range);}
    validate(header,filters)?;
    let starts={let mut cache=dataset.frame_cache.lock().map_err(|_|"帧索引异常")?;
        if let Some(index)=cache.as_ref().filter(|index|index.header==header){index.starts.clone()}else{let starts=Arc::new(frame_starts(&dataset.text,header)?);*cache=Some(FrameIndex{header:header.to_owned(),starts:starts.clone()});starts}
    };
    extract_indexed(&dataset.text,&starts,filters,range)
}
fn extract_indexed(text:&str,starts:&[usize],filters:&[String],range:Option<(usize,usize)>)->Result<DataResult,String>{
    let frames=starts.len();let (lo,hi)=range.unwrap_or((0,frames));if lo>hi||hi>frames||(lo==hi&&frames>0){return Err("显示帧范围超出数据".into());}let bucket_size=(hi-lo).div_ceil(MAX_BUCKETS).max(1);
    let mut result:Vec<DataSeries>=filters.iter().map(|filter|DataSeries{filter:filter.clone(),count:0,minimum:None,maximum:None,buckets:Vec::new()}).collect();
    for (index,start) in starts.iter().copied().enumerate().skip(lo).take(hi-lo){
        let end=starts.get(index+1).copied().unwrap_or(text.len());let frame=&text[start..end];let bucket=(index-lo)/bucket_size;
        for (series,filter) in result.iter_mut().zip(filters){if let Some(value)=extract_first_filter_value(frame,filter){
            series.count+=1;series.minimum=Some(series.minimum.map_or(value,|v|v.min(value)));series.maximum=Some(series.maximum.map_or(value,|v|v.max(value)));
            if let Some(last)=series.buckets.last_mut().filter(|last|(last.frame-lo)/bucket_size==bucket){last.low=last.low.min(value);last.high=last.high.max(value);}else{series.buckets.push(DataBucket{frame:index,low:value,high:value});}
        }}
    }Ok(DataResult{frames,range_start:lo,range_end:hi,series:result})
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn original_filter_semantics_and_missing_values(){
        let text="prefix\nFRAME TEMP 20.5 V 1\nFRAME XTEMP 99 TEMP -2e1 V 3\nFRAME TEMP: 8 V 4";
        let result=extract(text,"FRAME",&["TEMP".into(),"V".into()]).unwrap();
        assert_eq!(result.frames,3);assert_eq!(result.series[0].count,2);assert_eq!(result.series[0].minimum,Some(-20.0));assert_eq!(result.series[1].count,3);
        assert!(validate("",&["x".into()]).is_err());assert!(validate("x",&["A".into(),"A".into()]).is_err());
    }
    #[test]fn frame_range_keeps_absolute_indices_and_recomputes_visible_extrema(){
        let text="FRAME X 9\nFRAME X 2\nFRAME X -3\nFRAME X 8\n";
        let result=extract_range(text,"FRAME",&["X".into()],Some((1,3))).unwrap();
        assert_eq!(result.frames,4);assert_eq!((result.range_start,result.range_end),(1,3));
        assert_eq!(result.series[0].count,2);assert_eq!(result.series[0].minimum,Some(-3.0));assert_eq!(result.series[0].maximum,Some(2.0));
        assert_eq!(result.series[0].buckets.iter().map(|b|b.frame).collect::<Vec<_>>(),vec![1,2]);
        assert!(extract_range(text,"FRAME",&["X".into()],Some((1,5))).is_err());
    }
    #[test]fn cached_frame_index_is_reused_and_invalidated_for_a_different_header(){
        let dataset=Dataset{name:"fixture".into(),fingerprint:"fixture".into(),text:Arc::new("A X 1\nA X 2\nB X 3\n".into()),bytes:18,wida:None,path:PathBuf::from("fixture.log"),frame_cache:Arc::new(Mutex::new(None))};
        let first=extract_cached(&dataset,"A",&["X".into()],None).unwrap();assert_eq!(first.frames,2);
        let saved=dataset.frame_cache.lock().unwrap().as_ref().unwrap().starts.clone();
        let window=extract_cached(&dataset,"A",&["X".into()],Some((1,2))).unwrap();assert_eq!(window.series[0].minimum,Some(2.0));
        assert!(Arc::ptr_eq(&saved,&dataset.frame_cache.lock().unwrap().as_ref().unwrap().starts));
        let other=extract_cached(&dataset,"B",&["X".into()],None).unwrap();assert_eq!(other.frames,1);assert_eq!(other.series[0].minimum,Some(3.0));
        assert!(!Arc::ptr_eq(&saved,&dataset.frame_cache.lock().unwrap().as_ref().unwrap().starts));
    }
    #[test]fn large_input_is_bounded_and_keeps_extrema(){
        let mut text=String::new();for i in 0..100_000{use std::fmt::Write;writeln!(text,"FRAME X {}",if i==43210{-9999}else{i%7}).unwrap();}
        let result=extract(&text,"FRAME",&["X".into()]).unwrap();assert_eq!(result.frames,100_000);assert_eq!(result.series[0].count,100_000);assert_eq!(result.series[0].minimum,Some(-9999.0));assert!(result.series[0].buckets.len()<=MAX_BUCKETS);
    }
    #[test]fn export_uses_every_frame_and_escapes_headers(){
        let mut csv=Vec::new();write_csv(&mut csv,"F X 1 Y -2\nF X 3\n","F",&["X".into(),"Y".into(),"=SUM()".into()]).unwrap();
        assert_eq!(String::from_utf8(csv).unwrap(),"frame,\"X\",\"Y\",\"'=SUM()\"\n1,1,-2,\n2,3,,\n");
    }
    #[test]fn optional_real_log_extracts_without_device_access(){
        let Some(path)=std::env::var_os("WIPARSE_ANALYSIS_SAMPLE") else{return};
        let data=load_file(Path::new(&path)).unwrap();let result=extract(&data.text,"ASK",&["Vbrg".into()]).unwrap();
        assert!(result.frames>0);assert!(result.series[0].count>0);
        let end=result.frames.min(100);let window=extract_range(&data.text,"ASK",&["Vbrg".into()],Some((0,end))).unwrap();
        assert_eq!(window.frames,result.frames);assert_eq!(window.range_end,end);assert!(window.series[0].buckets.iter().all(|b|b.frame<end));
    }
    #[test]
    fn a_previous_source_cannot_append_after_live_session_reset(){
        let state=DataState::default();let old=reset_live(&state).unwrap();let current=reset_live(&state).unwrap();
        assert!(push_live(&state,old,&["FRAME X 99".into()],"FRAME",&["X".into()]).err().unwrap().contains("SESSION_CHANGED"));
        let fresh=push_live(&state,current,&["FRAME X 1".into(),"FRAME X 2".into()],"FRAME",&["X".into()]).unwrap();
        assert_eq!(fresh.frames,1);assert_eq!(fresh.series[0].maximum,Some(1.0));
    }

    #[test]
    fn large_valid_batches_commit_frames_before_trimming_the_partial_tail(){
        let mut session=LiveSession::default();let lines=(0..2000).map(|index|format!("FRAME X {index} {}","a".repeat(200))).collect::<Vec<_>>();
        let result=live_push(&mut session,&lines,"FRAME",&["X".into()]).unwrap();
        assert_eq!(result.frames,1999);assert_eq!(result.series[0].count,1999);assert_eq!(result.discarded_tail_bytes,0);
        let continuation=vec!["x".repeat(65000);6];let truncated=live_push(&mut session,&continuation,"FRAME",&["X".into()]).unwrap();assert!(truncated.discarded_tail_bytes>0);assert!(session.tail.len()<=MAX_LIVE_TAIL);
    }

    #[test]fn live_keeps_the_unfinished_frame_and_restarts_when_the_filter_changes(){
        let mut session=LiveSession::default();
        let waiting=live_push(&mut session,&["FRAME TEMP 1.5 extra".into()],"FRAME",&["TEMP".into()]).unwrap();
        assert_eq!(waiting.frames,0);assert!(waiting.pending);
        let committed=live_push(&mut session,&["FRAME TEMP -2".into()],"FRAME",&["TEMP".into()]).unwrap();
        assert_eq!(committed.frames,1);assert_eq!(committed.series[0].minimum,Some(1.5));assert!(committed.pending);
        let both=live_push(&mut session,&["FRAME TEMP 4".into()],"FRAME",&["TEMP".into()]).unwrap();
        assert_eq!(both.frames,2);assert_eq!(both.series[0].minimum,Some(-2.0));assert_eq!(both.series[0].maximum,Some(1.5));
        let changed=live_push(&mut session,&["FRAME V 9".into()],"FRAME",&["V".into()]).unwrap();
        assert_eq!(changed.frames,0);assert!(changed.pending);assert_eq!(changed.series[0].filter,"V");assert_eq!(changed.series[0].count,0);
        let value=live_push(&mut session,&["FRAME V 3".into()],"FRAME",&["V".into()]).unwrap();
        assert_eq!(value.series[0].minimum,Some(9.0));
        assert!(live_push(&mut session,&["bad\0".into()],"FRAME",&["V".into()]).is_err());
    }
    #[test]
    fn changed_rules_do_not_reinterpret_the_previous_partial_frame(){
        let mut session=LiveSession::default();
        live_push(&mut session,&["OLD NEW X 999".into()],"OLD",&["X".into()]).unwrap();
        session.capped=true;session.discarded_tail_bytes=123;
        let changed=live_push(&mut session,&["NEW X 3".into(),"NEW X 4".into()],"NEW",&["X".into()]).unwrap();
        assert_eq!(changed.frames,1);assert_eq!(changed.series[0].maximum,Some(3.0));
        assert!(!changed.capped);assert_eq!(changed.discarded_tail_bytes,0);
        live_push(&mut session,&["NEW X 1000 V 777".into()],"NEW",&["X".into()]).unwrap();
        let changed=live_push(&mut session,&["NEW V 8".into(),"NEW V 9".into()],"NEW",&["V".into()]).unwrap();
        assert_eq!(changed.frames,1);assert_eq!(changed.series[0].maximum,Some(8.0));
    }
    #[test]fn live_window_drops_the_oldest_frames_after_the_original_cap(){
        let mut session=LiveSession::default();
        let mut index=0u64;
        for _ in 0..101{
            let lines=(0..2_000).map(|_|{let line=format!("H N {index}");index+=1;line}).collect::<Vec<_>>();
            live_push(&mut session,&lines,"H",&["N".into()]).unwrap();
        }
        let result=present(&session,None).unwrap();
        assert_eq!(result.frames,MAX_LIVE_POINTS);assert!(result.capped);
        assert_eq!(result.series[0].minimum,Some(1999.0));assert_eq!(result.series[0].maximum,Some(201_998.0));
    }
}

#[cfg(test)]mod migration_cache_tests{
 use super::*;
 #[test]fn sparse_legacy_wida_roundtrips_metadata_ranges_and_csv(){
  let path=std::env::temp_dir().join(format!("wiparse-data-{}.wida",uuid::Uuid::new_v4()));
  let mut payload=series_payload(&["X".into(),"Y".into()],&["voltage".into(),"current".into()],vec![vec![[0.,1.],[2.,5.]],vec![[1.,2.]]],"FRAME",3,"legacy.log".into(),"original".into());
  payload.series[0].y_scale=2.;payload.series[0].y_offset=3.;cache::save_wida(&path,&payload).unwrap();
  let data=load_file(&path).unwrap();let loaded=data.wida.as_ref().unwrap();assert_eq!(loaded.meta.frame_count,3);assert_eq!(loaded.series[0].y_scale,2.);
  let result=extract_cached(&data,"",&[],Some((1,3))).unwrap();assert_eq!(result.series[0].minimum,Some(5.));assert_eq!(result.series[1].count,1);
  let mut csv=Vec::new();write_wida_csv(&mut csv,loaded).unwrap();let text=String::from_utf8(csv).unwrap();assert!(text.contains("1,1,"));assert!(text.contains("2,,2"));assert!(text.contains("3,5,"));
  use std::io::Write;std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(&[0]).unwrap();assert!(cache::load_wida(&path).is_err());std::fs::remove_file(path).unwrap();
 }
}
