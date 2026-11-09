//! User-selected file exchange. Persisted plugin paths stay relative; absolute
//! staging paths exist only for this bounded local operation.
use serde_json::{json,Value};
use std::{io::{Read,Write},path::{Path,PathBuf}};
const LIMIT:u64=64*1024*1024;
struct TransferFile(PathBuf);
impl Drop for TransferFile{fn drop(&mut self){let _=std::fs::remove_file(&self.0);}}

fn context(project:&str,plugin:&str)->Result<PathBuf,String>{
    let state=super::invoke_legacy_at(super::ENDPOINT,"ui.test_tool.state_safe",json!({}))?;
    if state["project"]!=project||state["selected"]!=plugin||state["running"]==true{return Err("工程、插件或运行状态已变化，请刷新后再操作".into());}
    let root=state["data_root_resolved"].as_str().ok_or("工程目录不可用")?;
    Path::new(root).canonicalize().map_err(|_|"工程目录不存在".into())
}
fn staging(root:&Path,project:&str)->Result<TransferFile,String>{
    let mut directory=root.to_path_buf();
    for segment in ["projects",project,".wiparse-transfer"] {
        directory.push(segment);
        match std::fs::symlink_metadata(&directory){
            Ok(metadata) if !metadata.is_dir()||metadata.file_type().is_symlink()=>return Err("工程交换目录无效".into()),
            Ok(_)=>{},
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>std::fs::create_dir(&directory).map_err(|_|"无法创建工程交换目录")?,
            Err(_)=>return Err("工程交换目录不可用".into()),
        }
    }
    let directory=directory.canonicalize().map_err(|_|"工程交换目录无效")?;
    if !directory.starts_with(root){return Err("工程交换目录越界".into());}
    Ok(TransferFile(directory.join(format!("{}.json",uuid::Uuid::new_v4()))))
}
fn read_bounded(path:&Path)->Result<Vec<u8>,String>{
    let file=std::fs::File::open(path).map_err(|_|"无法打开配置文件")?;
    let metadata=file.metadata().map_err(|_|"无法读取配置文件")?;
    if !metadata.is_file()||metadata.len()>LIMIT{return Err("请选择不超过 64 MiB 的配置文件".into());}
    let mut bytes=Vec::new();file.take(LIMIT+1).read_to_end(&mut bytes).map_err(|_|"配置读取失败")?;
    if bytes.len() as u64>LIMIT{return Err("配置文件超过 64 MiB".into());}
    Ok(bytes)
}
fn write_new(path:&Path,bytes:&[u8])->Result<(),String>{
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|_|"目标文件已存在或不能创建，请选择新的文件名")?;
    let result=file.write_all(bytes).and_then(|_|file.sync_all());drop(file);
    if result.is_err(){let _=std::fs::remove_file(path);return Err("配置保存失败".into());}Ok(())
}

#[tauri::command]
pub async fn hub_file_transfer(project:String,plugin:String,target:String,direction:String,strategy:String)->Result<Option<Value>,String>{
    if !super::valid_identifier(&project)||!super::valid_identifier(&plugin)||!matches!(target.as_str(),"config"|"project")||!matches!(direction.as_str(),"import"|"export")||!matches!(strategy.as_str(),"skip"|"rename"){return Err("配置交换参数无效".into());}
    tauri::async_runtime::spawn_blocking(move||{
        let root=context(&project,&plugin)?;
        let dialog=rfd::FileDialog::new().add_filter("WiParse 配置",&["json"]);
        let selected=if direction=="import"{dialog.pick_file()}else{dialog.set_file_name(if target=="project"{"wiparse-project.json"}else{"wiparse-configs.json"}).save_file()};
        let Some(selected)=selected else{return Ok(None)};
        if context(&project,&plugin)?!=root{return Err("工程目录已变化，请重新操作".into());}
        let temp=staging(&root,&project)?;
        if direction=="import"{let bytes=read_bounded(&selected)?;let _:Value=serde_json::from_slice(&bytes).map_err(|_|"配置不是有效的 JSON")?;write_new(&temp.0,&bytes)?;}
        let operation=if target=="project"{format!("project.file.{direction}")}else{format!("config.{direction}")};
        let result=super::invoke_legacy_at(super::ENDPOINT,&format!("ui.test_tool.{operation}"),json!({"plugin":plugin,"file":temp.0,"expected_project":project,"strategy":strategy}))?;
        if direction=="export"{let bytes=read_bounded(&temp.0)?;write_new(&selected,&bytes)?;}
        Ok(Some(json!({"result":result,"filename":selected.file_name().and_then(|v|v.to_str()).unwrap_or("配置文件")})))
    }).await.map_err(|_|"配置交换任务异常结束；请核对当前状态，不会自动重试".to_string())?
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn exported_file_is_new_and_preserves_all_bytes(){
        let root=std::env::temp_dir().join(format!("wiparse-transfer-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let path=root.join("配置 文件.json");let bytes=b"{\"schema_version\":\"test\"}\n";
        write_new(&path,bytes).unwrap();assert_eq!(read_bounded(&path).unwrap(),bytes);assert!(write_new(&path,b"replace").is_err());assert_eq!(std::fs::read(&path).unwrap(),bytes);
        std::fs::remove_file(path).unwrap();std::fs::remove_dir(root).unwrap();
    }
}
