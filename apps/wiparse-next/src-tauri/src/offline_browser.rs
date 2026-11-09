//! Offline tool roots are selected by the user and shared with legacy preferences.
use serde::{Deserialize};
use serde_json::{Value,json};
use std::{collections::BTreeMap,path::{Path,PathBuf},sync::{Arc,Mutex,atomic::{AtomicU64,Ordering}}};

pub(crate) struct Browser{session:u64,root:PathBuf,current:PathBuf}
#[derive(Default)]
pub struct OfflineBrowserState{pub(crate) data:Arc<Mutex<BTreeMap<String,Browser>>>,generation:AtomicU64}
#[derive(Deserialize)]
pub struct OfflineFile{pub session:u64,pub relative:String}
fn valid_scope(scope:&str)->Result<(),String>{if matches!(scope,"wave"|"data"){Ok(())}else{Err("离线工具类型无效".into())}}
fn supported(scope:&str,path:&Path)->bool{path.extension().and_then(|extension|extension.to_str()).is_some_and(|extension|if scope=="wave"{["csv","txt","isf","wfm"].iter().any(|allowed|extension.eq_ignore_ascii_case(allowed))}else{["txt","log","csv","wida"].iter().any(|allowed|extension.eq_ignore_ascii_case(allowed))})}
fn resolve(root:&Path,relative:&str)->Result<PathBuf,String>{
    if relative.len()>4096||relative.chars().any(char::is_control)||Path::new(relative).is_absolute()||relative.contains(':'){return Err("目录路径无效".into());}
    let path=root.join(relative).canonicalize().map_err(|_|"文件或目录已移动")?;if !path.starts_with(root){return Err("只能打开已选择目录内的文件".into());}Ok(path)
}
pub(crate) fn resolve_file(data:&Mutex<BTreeMap<String,Browser>>,scope:&str,file:&OfflineFile)->Result<PathBuf,String>{
    let root=data.lock().map_err(|_|"目录会话异常")?.get(scope).filter(|browser|browser.session==file.session).map(|browser|browser.root.clone()).ok_or("目录已切换，请刷新")?;
    let path=resolve(&root,&file.relative)?;if !path.is_file()||!supported(scope,&path){return Err("文件类型不支持".into());}Ok(path)
}
fn listing(scope:&str,root:&Path,current:&Path,session:u64)->Result<Value,String>{
    if !current.is_dir(){return Err("请选择文件目录".into());}
    let mut rows=Vec::new();let mut scanned=0;
    for entry in std::fs::read_dir(current).map_err(|e|e.to_string())?.take(10001){scanned+=1;if scanned>10000{break;}let entry=entry.map_err(|e|e.to_string())?;let kind=entry.file_type().map_err(|e|e.to_string())?;
        if kind.is_symlink()||!(kind.is_dir()||kind.is_file()&&supported(scope,&entry.path())){continue;}
        rows.push((!kind.is_dir(),entry.file_name().to_string_lossy().into_owned(),entry.path()));
    }
    rows.sort_by(|a,b|a.0.cmp(&b.0).then_with(||a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let truncated=scanned>10000||rows.len()>1000;let relative=current.strip_prefix(root).map_err(|_|"目录越界")?;
    Ok(json!({"session":session,"name":root.file_name().unwrap_or_default().to_string_lossy(),"relative":relative.to_string_lossy(),"parent":if relative.as_os_str().is_empty(){None}else{Some(relative.parent().unwrap_or(Path::new("")).to_string_lossy().into_owned())},"truncated":truncated,"entries":rows.into_iter().take(1000).map(|(file,name,path)|json!({"name":name,"directory":!file,"relative":path.strip_prefix(root).unwrap_or(&path).to_string_lossy()})).collect::<Vec<_>>()}))
}
#[tauri::command]
pub async fn offline_browser_status(state:tauri::State<'_,OfflineBrowserState>,scope:String)->Result<Option<Value>,String>{
    valid_scope(&scope)?;let data=state.data.clone();let session=state.generation.fetch_add(1,Ordering::AcqRel)+1;
    tauri::async_runtime::spawn_blocking(move||{let prior=data.lock().map_err(|_|"目录状态异常")?.get(&scope).map(|browser|(browser.root.clone(),browser.current.clone(),browser.session));
        if let Some((root,current,id))=prior{return listing(&scope,&root,&current,id).map(Some);}
        let preferences=super::invoke_legacy_at(super::ENDPOINT,"ui.offline.browser_root_safe",json!({"scope":scope}))?;
        let Some(root)=preferences["resolved"].as_str().filter(|value|!value.is_empty())else{return Ok(None)};
        let root=PathBuf::from(root).canonicalize().map_err(|_|"保存的目录已移动，请重新选择")?;let result=listing(&scope,&root,&root,session)?;
        let mut guard=data.lock().map_err(|_|"目录状态异常")?;if guard.contains_key(&scope){return Ok(None);}guard.insert(scope,Browser{session,current:root.clone(),root});Ok(Some(result))
    }).await.map_err(|_|"恢复目录失败".to_string())?
}
#[tauri::command]
pub async fn offline_browser_pick(state:tauri::State<'_,OfflineBrowserState>,scope:String,use_cache:Option<bool>)->Result<Option<Value>,String>{
    valid_scope(&scope)?;if use_cache==Some(true)&&scope!="data"{return Err("只有数据分析支持提取缓存".into());}
    let data=state.data.clone();let session=state.generation.fetch_add(1,Ordering::AcqRel)+1;
    tauri::async_runtime::spawn_blocking(move||{
        let root=if use_cache==Some(true){wiparse_core::data_extract_cache::ensure_cache_dir().map_err(|e|e.to_string())?}else{let Some(path)=rfd::FileDialog::new().set_title("选择离线文件目录").pick_folder()else{return Ok(None)};path};
        let root=root.canonicalize().map_err(|_|"目录不可用")?;let result=listing(&scope,&root,&root,session)?;
        super::invoke_legacy_at(super::ENDPOINT,"ui.offline.browser_root_safe",json!({"scope":scope,"directory":root}))?;
        data.lock().map_err(|_|"目录状态异常")?.insert(scope,Browser{session,current:root.clone(),root});Ok(Some(result))
    }).await.map_err(|_|"选择目录失败".to_string())?
}
#[tauri::command]
pub async fn offline_browser_list(state:tauri::State<'_,OfflineBrowserState>,scope:String,session:u64,relative:String)->Result<Value,String>{
    valid_scope(&scope)?;let data=state.data.clone();let root=data.lock().map_err(|_|"目录状态异常")?.get(&scope).filter(|browser|browser.session==session).map(|browser|browser.root.clone()).ok_or("目录已切换")?;
    tauri::async_runtime::spawn_blocking(move||{let current=resolve(&root,&relative)?;let result=listing(&scope,&root,&current,session)?;let mut guard=data.lock().map_err(|_|"目录状态异常")?;let browser=guard.get_mut(&scope).filter(|browser|browser.session==session).ok_or("目录已切换")?;browser.current=current;Ok(result)}).await.map_err(|_|"读取目录失败".to_string())?
}
