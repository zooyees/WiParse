//! Read-only, user-selected local report preview. HTML is never executed and
//! external resources are not fetched by this module.
use std::{io::{Cursor,Read},path::{Path,PathBuf},sync::{Arc,Mutex,atomic::{AtomicBool,AtomicU64,AtomicUsize,Ordering}}};
use base64::Engine;
use serde::Serialize;
use sha2::{Digest,Sha256};

const MAX_BYTES:u64=4*1024*1024;
const MAX_CHARS:usize=400_000;
fn image_budget()->u64{(super::resource_policy::bytes()/32) as u64}
fn session_image_budget()->usize{super::resource_policy::bytes()/8}

#[derive(Clone,Serialize)]
pub struct ReportDocument {filename:String,kind:String,text:String,sha256:String,bytes:usize,truncated:bool,session:u64,back_available:bool}
struct ReportPage {document:ReportDocument,directory:PathBuf,image_root:PathBuf}
struct ReportSession {document:ReportDocument,directory:PathBuf,image_root:PathBuf,image_bytes:usize,images:std::collections::BTreeSet<String>,history:Vec<ReportPage>}
#[derive(Default)]
pub struct ReportState {data:Arc<Mutex<Option<ReportSession>>>,busy:Arc<AtomicBool>,generation:Arc<AtomicU64>,image_busy:Arc<AtomicUsize>}
struct BusyGuard(Arc<AtomicBool>);
impl Drop for BusyGuard {fn drop(&mut self){self.0.store(false,Ordering::Release);}}
struct ImageGuard(Arc<AtomicUsize>);
impl Drop for ImageGuard {fn drop(&mut self){self.0.fetch_sub(1,Ordering::AcqRel);}}

#[tauri::command]
pub fn report_analysis_context(state:tauri::State<'_,ReportState>,session:u64)->Result<serde_json::Value,String>{
    let guard=state.data.lock().map_err(|_|"报告会话异常")?;let document=&guard.as_ref().filter(|current|current.document.session==session).ok_or("报告已切换")?.document;
    Ok(serde_json::json!({"filename":document.filename,"sha256":document.sha256,"bytes":document.bytes,"kind":document.kind,"preview_truncated":document.truncated,"excerpt":document.text.chars().take(6000).collect::<String>(),"excerpt_truncated":document.text.chars().count()>6000,"images_attached":false}))
}

#[tauri::command]
pub async fn report_open(state:tauri::State<'_,ReportState>)->Result<Option<ReportDocument>,String>{
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有报告正在打开".into());}
    let data=state.data.clone();let busy=state.busy.clone();let generation=state.generation.clone();let token=generation.fetch_add(1,Ordering::AcqRel)+1;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=BusyGuard(busy);
        let Some(path)=rfd::FileDialog::new().add_filter("报告和文本",&["md","markdown","txt","log","json"]).pick_file() else{return Ok(None)};
        let canonical=path.canonicalize().map_err(|e|format!("无法读取报告：{e}"))?;
        let directory=canonical.parent().ok_or("报告目录无效")?.to_path_buf();
        let mut document=read_document(&canonical)?;document.session=token;
        let mut guard=data.lock().map_err(|_|"报告会话异常")?;
        if generation.load(Ordering::Acquire)!=token{return Err("打开操作已取消".into());}
        *guard=Some(ReportSession{document:document.clone(),image_root:directory.clone(),directory,image_bytes:0,images:Default::default(),history:Vec::new()});Ok(Some(document))
    }).await.map_err(|_|"报告打开失败".to_string())?
}

fn indexed_file(tree:&serde_json::Value,path:&str)->bool {
    tree.get("tests").and_then(|v|v.as_array()).into_iter().flatten().any(|test|
        test.get("results").and_then(|v|v.as_array()).into_iter().flatten().any(|run|
            run.get("groups").and_then(|v|v.as_array()).into_iter().flatten().any(|group|
                group.get("files").and_then(|v|v.as_array()).into_iter().flatten().any(|file|file.get("path").and_then(|v|v.as_str())==Some(path)))))
}

fn resolve_indexed_report(root:&str,project:&str,path:&str,tree:&serde_json::Value)->Result<(PathBuf,PathBuf,PathBuf),String>{
    if !super::valid_identifier(project)||!indexed_file(tree,path){return Err("工程或报告列表已变化，请刷新".into());}
    let data_root=Path::new(root).canonicalize().map_err(|_|"数据目录不可用")?;
    let projects_root=data_root.join("projects").canonicalize().map_err(|_|"工程目录不可用")?;
    let project_root=projects_root.join(project).canonicalize().map_err(|_|"工程目录不可用")?;
    if projects_root.parent()!=Some(data_root.as_path())||project_root.parent()!=Some(projects_root.as_path()){return Err("工程目录无效".into());}
    let canonical=Path::new(path).canonicalize().map_err(|_|"报告文件不可用")?;
    if !canonical.starts_with(&project_root)||canonical==project_root{return Err("报告不在当前工程目录内".into());}
    Ok((data_root,project_root,canonical))
}

#[tauri::command]
pub async fn report_open_indexed(state:tauri::State<'_,ReportState>,project:String,path:String)->Result<ReportDocument,String>{
    if !super::valid_identifier(&project)||path.is_empty()||path.len()>4096||path.contains('\0'){return Err("报告标识无效".into());}
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有报告正在打开".into());}
    let data=state.data.clone();let busy=state.busy.clone();let generation=state.generation.clone();let token=generation.fetch_add(1,Ordering::AcqRel)+1;
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=BusyGuard(busy);
        let response=super::invoke_legacy_at(super::ENDPOINT,"ui.report.tree_safe",serde_json::json!({}))?;
        if response.get("project_id").and_then(|v|v.as_str())!=Some(project.as_str()){return Err("工程或报告列表已变化，请刷新".into());}
        let root=response.get("data_root").and_then(|v|v.as_str()).ok_or("工程目录无效")?;
        let (data_root,image_root,canonical)=resolve_indexed_report(root,&project,&path,&response["tree"])?;
        let directory=canonical.parent().ok_or("报告目录无效")?.to_path_buf();let mut document=read_document(&canonical)?;
        let current=super::invoke_legacy_at(super::ENDPOINT,"ui.test_tool.state_safe",serde_json::json!({}))?;
        let current_root=current.get("data_root_resolved").and_then(|v|v.as_str()).and_then(|v|Path::new(v).canonicalize().ok());
        if current.get("project").and_then(|v|v.as_str())!=Some(project.as_str())||current_root.as_ref()!=Some(&data_root){return Err("工程已切换，请刷新报告列表".into());}
        document.session=token;let mut guard=data.lock().map_err(|_|"报告会话异常")?;
        if generation.load(Ordering::Acquire)!=token{return Err("打开操作已取消".into());}
        *guard=Some(ReportSession{document:document.clone(),directory,image_root,image_bytes:0,images:Default::default(),history:Vec::new()});Ok(document)
    }).await.map_err(|_|"报告打开失败".to_string())?
}
#[tauri::command]
pub fn report_status(state:tauri::State<'_,ReportState>)->Result<Option<ReportDocument>,String>{Ok(state.data.lock().map_err(|_|"报告会话异常")?.as_ref().map(|value|value.document.clone()))}
#[tauri::command]
pub fn report_close(state:tauri::State<'_,ReportState>)->Result<(),String>{state.generation.fetch_add(1,Ordering::AcqRel);*state.data.lock().map_err(|_|"报告会话异常")?=None;Ok(())}

pub fn resolve_project_file(project:&str,path:&str)->Result<PathBuf,String>{
    if !super::valid_identifier(project)||path.is_empty()||path.len()>4096||path.contains('\0')||path.contains('\n')||path.contains('\r'){return Err("报告标识无效".into());}
    let response=super::invoke_legacy_at(super::ENDPOINT,"ui.report.tree_safe",serde_json::json!({}))?;
    if response.get("project_id").and_then(|value|value.as_str())!=Some(project){return Err("工程或报告列表已变化，请刷新".into());}
    let root=response.get("data_root").and_then(|value|value.as_str()).ok_or("工程目录无效")?;
    let (data_root,_,canonical)=resolve_indexed_report(root,project,path,&response["tree"])?;
    let current=super::invoke_legacy_at(super::ENDPOINT,"ui.test_tool.state_safe",serde_json::json!({}))?;
    let current_root=current.get("data_root_resolved").and_then(|value|value.as_str()).and_then(|value|Path::new(value).canonicalize().ok());
    if current.get("project").and_then(|value|value.as_str())!=Some(project)||current_root.as_ref()!=Some(&data_root){return Err("工程已切换，请刷新报告列表".into());}
    Ok(canonical)
}

#[tauri::command]
pub async fn report_show(project:String,path:String,action:String)->Result<(),String>{
    if !matches!(action.as_str(),"reveal"|"open"){return Err("不支持的打开方式".into());}
    tauri::async_runtime::spawn_blocking(move||{
        let canonical=resolve_project_file(&project,&path)?;
        show_indexed_report(&canonical,&action)
    }).await.map_err(|_|"无法交给系统打开".to_string())?
}

pub(crate) fn show_indexed_report(path:&Path,action:&str)->Result<(),String>{
    let text=path.display().to_string();
    if text.contains('\n')||text.contains('\r')||text.contains('\0'){return Err("路径无效".into());}
    let mut command=std::process::Command::new(if action=="reveal"{"explorer"}else{"rundll32.exe"});
    if action=="reveal"{command.arg(format!("/select,{text}"));}else{command.args(["url.dll,FileProtocolHandler",&text]);}
    command.spawn().map_err(|error|format!("无法交给系统打开：{error}"))?;
    Ok(())
}

#[tauri::command]
pub async fn report_image(state:tauri::State<'_,ReportState>,session:u64,source:String)->Result<String,String>{
    if source.len()>2048{return Err("图片路径过长".into());}
    if state.image_busy.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<4).then_some(n+1)).is_err(){return Err("图片加载繁忙".into());}
    let permit=ImageGuard(state.image_busy.clone());let data=state.data.clone();
    let (directory,image_root)={let guard=data.lock().map_err(|_|"报告会话异常")?;let current=guard.as_ref().ok_or("报告已关闭")?;if current.document.session!=session{return Err("报告已切换".into());}(current.directory.clone(),current.image_root.clone())};
    tauri::async_runtime::spawn_blocking(move||{
        let _guard=permit;
        let (mime,bytes)=read_image(&directory,&image_root,&source)?;
        let mut guard=data.lock().map_err(|_|"报告会话异常")?;
        let current=guard.as_mut().ok_or("报告已关闭")?;
        if current.document.session!=session{return Err("报告已切换".into());}
        if !current.images.contains(&source)&&current.image_bytes.saturating_add(bytes.len())>session_image_budget(){return Err("报告图片超过预览总量上限".into());}
        if current.images.insert(source){current.image_bytes+=bytes.len();}
        Ok(format!("data:{mime};base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes)))
    }).await.map_err(|_|"图片加载失败".to_string())?
}

/// Resolve a user-clicked evidence link against the currently opened report.
/// Local files may be absolute for old reports, but must remain in its allowed
/// root. Schemes, UNC paths, missing files and stale sessions are rejected.
pub fn resolve_report_link(state: &ReportState, session: u64, source: &str) -> Result<PathBuf,String> {
    let guard=state.data.lock().map_err(|_|"报告会话异常")?;
    let current=guard.as_ref().ok_or("报告已关闭")?;
    if current.document.session!=session {return Err("报告已切换，请重新打开链接".into());}
    resolve_link_path(&current.directory,&current.image_root,source)
}
fn resolve_link_path(directory:&Path,root:&Path,source:&str)->Result<PathBuf,String>{
    if source.is_empty()||source.len()>4096{return Err("证据路径无效".into());}
    let path_part=source.split(['#','?']).next().unwrap_or("");
    let decoded=percent_encoding::percent_decode_str(path_part).decode_utf8().map_err(|_|"证据路径编码无效")?;
    if decoded.is_empty()||decoded.contains(['\0','\r','\n'])||decoded.starts_with("//")||decoded.starts_with(r"\\") {return Err("只支持允许目录内的证据文件".into());}
    if let Some(colon)=decoded.find(':') {
        if colon!=1||!decoded.as_bytes()[0].is_ascii_alphabetic(){return Err("不支持此链接类型".into());}
    }
    let candidate=directory.join(decoded.as_ref()).canonicalize().map_err(|_|"证据文件不存在")?;
    if !candidate.starts_with(root)||!candidate.is_file(){return Err("证据文件位于允许范围之外".into());}
    let extension=candidate.extension().and_then(|value|value.to_str()).unwrap_or("").to_ascii_lowercase();
    if !["md","markdown","txt","log","json","jsonl","csv","isf","wfm","png","jpg","jpeg","gif","webp","bmp","pdf","html","htm"].contains(&extension.as_str()) {return Err("不支持打开此类证据文件".into());}
    Ok(candidate)
}

#[tauri::command]
pub async fn report_open_linked(state:tauri::State<'_,ReportState>,session:u64,source:String)->Result<ReportDocument,String>{
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有报告正在打开".into());}
    let path=resolve_report_link(&state,session,&source);
    let permit=BusyGuard(state.busy.clone());
    let path=path?;let data=state.data.clone();let generation=state.generation.clone();
    tauri::async_runtime::spawn_blocking(move||{
        let _permit=permit;let mut document=read_document(&path)?;
        let mut guard=data.lock().map_err(|_|"报告会话异常")?;
        let previous=guard.as_mut().ok_or("报告已关闭")?;
        if previous.document.session!=session{return Err("报告已切换，请重新打开链接".into());}
        let image_root=previous.image_root.clone();
        let mut history=std::mem::take(&mut previous.history);
        history.push(ReportPage{document:previous.document.clone(),directory:previous.directory.clone(),image_root:image_root.clone()});
        if history.len()>8{history.remove(0);}
        document.session=generation.fetch_add(1,Ordering::AcqRel)+1;document.back_available=true;
        *guard=Some(ReportSession{document:document.clone(),directory:path.parent().ok_or("证据目录无效")?.to_path_buf(),image_root,image_bytes:0,images:Default::default(),history});
        Ok(document)
    }).await.map_err(|_|"证据读取失败".to_string())?
}

/// Return to the previously viewed frozen document; paths remain private.
#[tauri::command]
pub fn report_back(state:tauri::State<'_,ReportState>,session:u64)->Result<ReportDocument,String>{
    if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("已有报告正在打开".into());}
    let _permit=BusyGuard(state.busy.clone());let mut guard=state.data.lock().map_err(|_|"报告会话异常")?;
    let current=guard.as_mut().ok_or("报告已关闭")?;
    if current.document.session!=session{return Err("报告已切换".into());}
    let page=current.history.pop().ok_or("没有可返回的报告")?;
    current.document=page.document;current.directory=page.directory;current.image_root=page.image_root;current.image_bytes=0;current.images.clear();
    current.document.session=state.generation.fetch_add(1,Ordering::AcqRel)+1;current.document.back_available=!current.history.is_empty();
    Ok(current.document.clone())
}

#[tauri::command]
pub async fn report_show_linked(state:tauri::State<'_,ReportState>,session:u64,source:String)->Result<(),String>{
    let path=resolve_report_link(&state,session,&source)?;
    let extension=path.extension().and_then(|v|v.to_str()).unwrap_or("").to_ascii_lowercase();
    if !["pdf","html","htm"].contains(&extension.as_str()){return Err("此证据应在 WiParse 内查看".into());}
    show_indexed_report(&path,"open")
}

#[tauri::command]
pub fn report_open_external(state:tauri::State<'_,ReportState>,session:u64,source:String)->Result<(),String>{
    {let guard=state.data.lock().map_err(|_|"报告会话异常")?;if guard.as_ref().is_none_or(|current|current.document.session!=session){return Err("报告已切换".into());}}
    if source.len()>4096||source.contains(['\0','\r','\n']){return Err("链接无效".into());}
    let url=tauri::Url::parse(&source).map_err(|_|"链接无效")?;
    if !matches!(url.scheme(),"http"|"https")||url.host_str().is_none()||!url.username().is_empty()||url.password().is_some(){return Err("仅支持普通 HTTP/HTTPS 链接".into());}
    // Only called for an explicit click. Never fetch external report resources.
    std::process::Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler",url.as_str()]).spawn().map_err(|_|"无法打开浏览器")?;Ok(())
}

fn read_image(directory:&Path,image_root:&Path,source:&str)->Result<(&'static str,Vec<u8>),String>{
    let path=resolve_link_path(directory,image_root,source)?;
    let file=super::resource_policy::open_stable(&path)?;
    let metadata=file.metadata().map_err(|_|"无法读取图片")?;
    if !metadata.is_file()||metadata.len()>image_budget(){return Err("图片超过当前内存预算或不是普通文件".into());}
    let mut bytes=Vec::new();file.take(image_budget()+1).read_to_end(&mut bytes).map_err(|_|"无法读取图片")?;
    if bytes.len() as u64>image_budget(){return Err("图片超过当前内存预算".into());}
    let format=image::guess_format(&bytes).map_err(|_|"图片格式不支持")?;
    let mime=match format {image::ImageFormat::Png=>"image/png",image::ImageFormat::Jpeg=>"image/jpeg",image::ImageFormat::Gif=>"image/gif",image::ImageFormat::WebP=>"image/webp",_=>return Err("图片格式不支持".into())};
    let (width,height)=image::ImageReader::with_format(Cursor::new(&bytes),format).into_dimensions().map_err(|_|"图片格式无效")?;
    if width==0||height==0||u64::from(width)*u64::from(height)*4>image_budget()*2{return Err("图片尺寸超过预览上限".into());}
    Ok((mime,bytes))
}

fn read_document(path:&Path)->Result<ReportDocument,String>{
    let extension=path.extension().and_then(|v|v.to_str()).unwrap_or("").to_ascii_lowercase();
    if !["md","markdown","txt","log","json","jsonl"].contains(&extension.as_str()){return Err("仅支持 Markdown、文本、日志和 JSON 报告".into());}
    let mut file=super::resource_policy::open_stable(path)?;let meta=file.metadata().map_err(|e|e.to_string())?;
    if !meta.is_file(){return Err("报告不是普通文件".into())}
    let mut hash=Sha256::new();let mut bytes=Vec::new();let mut buffer=[0u8;65536];let mut total=0usize;
    loop{let n=file.read(&mut buffer).map_err(|e|e.to_string())?;if n==0{break}hash.update(&buffer[..n]);total+=n;if bytes.len()<MAX_BYTES as usize{let take=n.min(MAX_BYTES as usize-bytes.len());bytes.extend_from_slice(&buffer[..take]);}}
    let complete=total<=MAX_BYTES as usize;
    let contents=match std::str::from_utf8(&bytes){Ok(text)=>text,Err(error) if !complete&&error.error_len().is_none()=>std::str::from_utf8(&bytes[..error.valid_up_to()]).map_err(|_|"报告不是 UTF-8")?,Err(_)=>return Err("报告不是有效 UTF-8 文本".into())};
    let text:String=contents.chars().take(MAX_CHARS).collect();let truncated=!complete||contents.chars().count()>MAX_CHARS;
    let sha256=format!("{:x}",hash.finalize());
    Ok(ReportDocument{filename:path.file_name().and_then(|v|v.to_str()).unwrap_or("报告").to_string(),kind:if extension=="markdown" {"md".into()}else{extension},text,sha256,bytes:total,truncated,session:0,back_available:false})
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn evidence_links_accept_portable_and_legacy_paths_but_reject_escape_and_executables(){
        let temp=std::env::temp_dir().join(format!("wiparse-links-{}",uuid::Uuid::new_v4()));
        let root=temp.join("project");let reports=root.join("reports");let waves=root.join("waves");
        std::fs::create_dir_all(&reports).unwrap();std::fs::create_dir_all(&waves).unwrap();
        let sample=waves.join("CH 1.isf");std::fs::write(&sample,b"fixture").unwrap();std::fs::write(root.join("run.exe"),b"no").unwrap();std::fs::write(temp.join("outside.md"),b"outside").unwrap();
        let root=root.canonicalize().unwrap();let reports=reports.canonicalize().unwrap();
        assert_eq!(resolve_link_path(&reports,&root,"../waves/CH%201.isf#CH1").unwrap(),sample.canonicalize().unwrap());
        assert!(resolve_link_path(&reports,&root,sample.to_str().unwrap()).is_ok());
        for source in ["../../outside.md","%2e%2e/%2e%2e/outside.md","../run.exe","https://example.invalid/a.md","file:///C:/a.md","//host/a.md"] {assert!(resolve_link_path(&reports,&root,source).is_err(),"{source}");}
        let target=temp.canonicalize().unwrap();assert!(target.starts_with(std::env::temp_dir().canonicalize().unwrap()));std::fs::remove_dir_all(target).unwrap();
    }

    #[test]fn report_location_rejects_a_broken_path(){assert!(show_indexed_report(Path::new("a\nb.md"),"reveal").is_err());}
    #[test]fn indexed_report_must_match_an_actual_tree_entry(){
        let tree=serde_json::json!({"tests":[{"results":[{"groups":[{"files":[{"path":"C:\\project\\report.md"}]}]}]}]});
        assert!(indexed_file(&tree,"C:\\project\\report.md"));
        assert!(!indexed_file(&tree,"C:\\project\\other.md"));
        assert!(!indexed_file(&serde_json::json!({}),"C:\\project\\report.md"));
    }
    #[test]fn indexed_report_rejects_a_listed_file_outside_its_project(){
        let root=std::env::temp_dir().join(format!("wiparse-report-test-{}",uuid::Uuid::new_v4()));
        let project=root.join("projects").join("lab");std::fs::create_dir_all(&project).unwrap();
        let inside=project.join("report.md");let outside=root.join("outside.md");std::fs::write(&inside,"inside").unwrap();std::fs::write(&outside,"outside").unwrap();
        let make_tree=|path:&Path|serde_json::json!({"tests":[{"results":[{"groups":[{"files":[{"path":path.display().to_string()}]}]}]}]});
        assert!(resolve_indexed_report(root.to_str().unwrap(),"lab",inside.to_str().unwrap(),&make_tree(&inside)).is_ok());
        assert!(resolve_indexed_report(root.to_str().unwrap(),"lab",outside.to_str().unwrap(),&make_tree(&outside)).is_err());
        std::fs::remove_file(inside).unwrap();std::fs::remove_file(outside).unwrap();std::fs::remove_dir(project).unwrap();std::fs::remove_dir(root.join("projects")).unwrap();std::fs::remove_dir(root).unwrap();
    }
    #[test]fn reads_actual_document_with_hash_and_rejects_other_types(){
        let root=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../");
        let report=read_document(&root.join("README.md")).unwrap();assert!(!report.text.is_empty());assert_eq!(report.sha256.len(),64);assert_eq!(report.kind,"md");
        assert!(read_document(&root.join("Cargo.toml")).is_err());
        let alternate=std::env::temp_dir().join(format!("wiparse-{}.markdown",uuid::Uuid::new_v4()));
        std::fs::write(&alternate,"# Report").unwrap();assert_eq!(read_document(&alternate).unwrap().kind,"md");std::fs::remove_file(alternate).unwrap();
    }
    #[test]fn local_images_are_jail_limited_and_external_sources_rejected(){
        let directory=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../test-tools/plugins/fpga-firmware-test/assets").canonicalize().unwrap();
        let (mime,bytes)=read_image(&directory,&directory,"simulation-waveform.png").unwrap();assert_eq!(mime,"image/png");assert!(!bytes.is_empty());
        let project_root=directory.parent().unwrap();assert!(read_image(&directory,project_root,"../assets/simulation-waveform.png").is_ok());
        for invalid in ["https://example.invalid/image.png","../plugin.json","%2e%2e/plugin.json","C:/secret.png","data:image/png;base64,abc"] {assert!(read_image(&directory,&directory,invalid).is_err(),"{invalid}");}
    }
}

#[cfg(test)]mod large_report_tests{
 use super::*;
 #[test]fn report_over_four_mib_has_full_hash_and_bounded_utf8_preview(){
  let path=std::env::temp_dir().join(format!("wiparse-report-{}.md",uuid::Uuid::new_v4()));let text="中文\n".repeat(700_000);std::fs::write(&path,text.as_bytes()).unwrap();let document=read_document(&path).unwrap();assert_eq!(document.bytes,text.len());assert!(document.truncated);assert!(document.text.chars().count()<=MAX_CHARS);assert_eq!(document.sha256,format!("{:x}",Sha256::digest(text.as_bytes())));std::fs::remove_file(path).unwrap();
 }
}
