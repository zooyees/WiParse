//! Bounded directory navigation. Only user-selected roots are readable.
use serde_json::{json,Value};
use std::{path::{Path,PathBuf},sync::{Arc,Mutex,atomic::{AtomicU64,Ordering}}};

struct Browser {id:u64,root:PathBuf,current:PathBuf}
#[derive(Default)]
pub struct LogBrowserState {data:Arc<Mutex<Option<Browser>>>,generation:AtomicU64}

fn context(state:&LogBrowserState,session:u64)->Result<PathBuf,String>{
    let guard=state.data.lock().map_err(|_|"日志目录状态异常")?;
    let browser=guard.as_ref().filter(|browser|browser.id==session).ok_or("日志目录已变化，请重新选择")?;
    Ok(browser.root.clone())
}
fn resolve(root:&Path,relative:&str)->Result<PathBuf,String>{
    if relative.len()>4096||relative.chars().any(char::is_control)||Path::new(relative).is_absolute()||relative.contains(':'){return Err("日志目录路径无效".into());}
    let path=root.join(relative).canonicalize().map_err(|_|"日志目录或文件已移动")?;
    if !path.starts_with(root){return Err("只能浏览已选择目录内的内容".into());}Ok(path)
}
fn supported(path:&Path)->bool{
    path.extension().and_then(|value|value.to_str()).is_some_and(|value|["txt","log","csv","json","jsonl"].iter().any(|extension|value.eq_ignore_ascii_case(extension)))
}
fn list(root:&Path,path:&Path,session:u64)->Result<Value,String>{
    if !path.is_dir(){return Err("请选择日志文件夹".into());}
    let mut rows=Vec::new();let mut scanned=0usize;
    for entry in std::fs::read_dir(path).map_err(|_|"无法读取日志目录")?.take(10001){
        scanned+=1;if scanned>10000{break;}
        let entry=entry.map_err(|_|"读取日志目录条目失败")?;
        let file_type=entry.file_type().map_err(|_|"无法读取日志条目属性")?;
        if file_type.is_symlink()||!(file_type.is_dir()||(file_type.is_file()&&supported(&entry.path()))){continue;}
        rows.push((!file_type.is_dir(),entry.file_name().to_string_lossy().into_owned(),entry.path()));
    }
    rows.sort_by(|left,right|left.0.cmp(&right.0).then_with(||left.1.to_lowercase().cmp(&right.1.to_lowercase())));
    let truncated=scanned>10000||rows.len()>1000;
    let entries=rows.into_iter().take(1000).map(|(file,name,path)|json!({"name":name,"directory":!file,"relative":path.strip_prefix(root).unwrap_or(&path).to_string_lossy()})).collect::<Vec<_>>();
    let relative=path.strip_prefix(root).map_err(|_|"日志目录已变化")?;
    let parent=if relative.as_os_str().is_empty(){None}else{Some(relative.parent().unwrap_or(Path::new("")).to_string_lossy().into_owned())};
    Ok(json!({"session":session,"name":root.file_name().and_then(|value|value.to_str()).unwrap_or("日志目录"),"relative":relative.to_string_lossy(),"parent":parent,"entries":entries,"truncated":truncated}))
}

#[tauri::command]
pub async fn serial_browser_status(state:tauri::State<'_,LogBrowserState>)->Result<Option<Value>,String>{
    let snapshot={let guard=state.data.lock().map_err(|_|"日志目录状态异常")?;guard.as_ref().map(|browser|(browser.id,browser.root.clone(),browser.current.clone()))};
    let data=state.data.clone();let new_id=state.generation.fetch_add(1,Ordering::AcqRel)+1;
    tauri::async_runtime::spawn_blocking(move||{
        if let Some((session,root,current))=snapshot{return list(&root,&current,session).map(Some);}
        let preference=super::invoke_legacy_at(super::ENDPOINT,"ui.serial.browser_root_safe",json!({}))?;
        let Some(saved)=preference["resolved"].as_str() else{return Ok(None)};
        let root=Path::new(saved).canonicalize().map_err(|_|"上次日志目录已移动，请重新选择")?;let listing=list(&root,&root,new_id)?;
        let mut guard=data.lock().map_err(|_|"日志目录状态异常")?;if guard.is_some(){return Ok(None);}
        *guard=Some(Browser{id:new_id,current:root.clone(),root});Ok(Some(listing))
    }).await.map_err(|_|"日志目录恢复失败".to_string())?
}

#[tauri::command]
pub async fn serial_browser_pick(state:tauri::State<'_,LogBrowserState>)->Result<Option<Value>,String>{
    let data=state.data.clone();let session=state.generation.fetch_add(1,Ordering::AcqRel)+1;
    tauri::async_runtime::spawn_blocking(move||{
        let Some(root)=rfd::FileDialog::new().set_title("选择日志目录").pick_folder() else{return Ok(None)};
        let root=root.canonicalize().map_err(|_|"日志目录不可用")?;let listing=list(&root,&root,session)?;
        super::invoke_legacy_at(super::ENDPOINT,"ui.serial.browser_root_safe",json!({"directory":root}))?;
        *data.lock().map_err(|_|"日志目录状态异常")?=Some(Browser{id:session,current:root.clone(),root});Ok(Some(listing))
    }).await.map_err(|_|"日志目录选择失败".to_string())?
}
#[tauri::command]
pub async fn serial_browser_list(state:tauri::State<'_,LogBrowserState>,session:u64,relative:String)->Result<Value,String>{
    let root=context(&state,session)?;let data=state.data.clone();
    tauri::async_runtime::spawn_blocking(move||{
        let path=resolve(&root,&relative)?;let listing=list(&root,&path,session)?;
        let mut guard=data.lock().map_err(|_|"日志目录状态异常")?;let browser=guard.as_mut().filter(|browser|browser.id==session).ok_or("日志目录已变化")?;browser.current=path;
        Ok(listing)
    }).await.map_err(|_|"日志目录读取失败".to_string())?
}
#[tauri::command]
pub async fn serial_browser_open_file(state:tauri::State<'_,LogBrowserState>,session:u64,relative:String)->Result<Value,String>{
    let root=context(&state,session)?;let path=resolve(&root,&relative)?;
    if !path.is_file()||!supported(&path){return Err("不支持此日志文件类型".into());}
    super::invoke_legacy("ui.serial.open_safe".into(),json!({"path":path})).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_navigation_never_escapes_selected_root(){
        let root=std::env::temp_dir().join(format!("wiparse-log-browser-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();std::fs::create_dir(root.join("nested")).unwrap();std::fs::write(root.join("nested/中文.log"),b"record").unwrap();
        let root=root.canonicalize().unwrap();assert!(resolve(&root,"..").is_err());assert!(resolve(&root,"C:/Windows").is_err());
        let result=list(&root,&root.join("nested"),3).unwrap();assert_eq!(result["parent"],"");assert_eq!(result["entries"][0]["name"],"中文.log");
        std::fs::remove_file(root.join("nested/中文.log")).unwrap();std::fs::remove_dir(root.join("nested")).unwrap();std::fs::remove_dir(root).unwrap();
    }
}
