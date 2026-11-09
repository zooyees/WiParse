//! Explicit firmware selection, immutable staging and a confirmed probe job.
use serde_json::{Value,json};
use sha2::{Digest,Sha256};
use std::{collections::BTreeMap,io::{Read,Write},path::PathBuf,sync::{Arc,Mutex}};

#[derive(Clone)]
struct Selected{path:PathBuf,name:String,bytes:u64,sha256:String}
#[derive(Default)]
pub struct ProbeFirmwareState{selected:Arc<Mutex<BTreeMap<String,Selected>>>}
const MAX_IMAGE:u64=1024*1024*1024;
#[tauri::command]
pub async fn probe_select_firmware(state:tauri::State<'_,ProbeFirmwareState>)->Result<Option<Value>,String>{
    let selections=state.selected.clone();
    tauri::async_runtime::spawn_blocking(move||{
        let Some(path)=rfd::FileDialog::new().add_filter("固件",&["elf","axf","hex","bin"]).pick_file()else{return Ok(None)};
        let path=path.canonicalize().map_err(|e|e.to_string())?;let mut input=std::fs::File::open(&path).map_err(|e|e.to_string())?;
        let metadata=input.metadata().map_err(|e|e.to_string())?;if !metadata.is_file()||metadata.len()==0||metadata.len()>MAX_IMAGE{return Err("请选择非空且不超过 1 GiB 的固件文件".into());}
        let mut hash=Sha256::new();let mut buffer=[0u8;256*1024];let mut bytes=0u64;
        loop{let count=input.read(&mut buffer).map_err(|e|e.to_string())?;if count==0{break;}bytes+=count as u64;if bytes>MAX_IMAGE{return Err("固件读取期间超过预算".into());}hash.update(&buffer[..count]);}
        if bytes!=metadata.len(){return Err("固件文件已变化，请重新选择".into());}
        let selected=Selected{name:path.file_name().unwrap_or_default().to_string_lossy().into_owned(),path,bytes,sha256:format!("{:x}",hash.finalize())};
        let token=uuid::Uuid::new_v4().to_string();let reply=json!({"token":token,"name":selected.name,"bytes":selected.bytes,"sha256":selected.sha256});
        let mut selections=selections.lock().map_err(|_|"固件选择状态异常")?;if selections.len()>=32{selections.clear();}selections.insert(token,selected);Ok(Some(reply))
    }).await.map_err(|_|"固件读取失败".to_string())?
}

#[tauri::command]
pub async fn probe_execute_safe(state:tauri::State<'_,ProbeFirmwareState>,device_id:u64,resource:String,action:String,target:String,selection:Option<String>,verify:bool,base_address:Option<String>,consent:bool)->Result<Value,String>{
    if !consent||resource.is_empty()||resource.len()>4096||resource.chars().any(char::is_control)||target.trim().is_empty()||target.len()>256||target.chars().any(char::is_control)||!matches!(action.as_str(),"flash"|"erase"){return Err("PROBE.INVALID_REQUEST: 烧录/擦除确认信息无效".into());}
    let image=if action=="flash"{Some(state.selected.lock().map_err(|_|"固件选择状态异常")?.get(selection.as_deref().unwrap_or("")).cloned().ok_or("PROBE.SELECTION_EXPIRED: 固件选择已失效，请重新选择")?)}else{None};
    let address=if let Some(value)=base_address.filter(|value|!value.trim().is_empty()){let value=value.trim();Some(if let Some(value)=value.strip_prefix("0x").or_else(||value.strip_prefix("0X")){u64::from_str_radix(value,16)}else{value.parse::<u64>()}.map_err(|_|"BIN 基地址无效")?)}else{None};
    let (staged,checksum)=tauri::async_runtime::spawn_blocking(move||->Result<(Option<PathBuf>,Option<String>),String>{
        let Some(image)=image else{return Ok((None,None))};
        let root=wiparse_core::paths::project_path("cache/probe-staging");std::fs::create_dir_all(&root).map_err(|e|e.to_string())?;
        let extension=image.path.extension().and_then(|extension|extension.to_str()).unwrap_or("elf").to_ascii_lowercase();if !["elf","axf","hex","bin"].contains(&extension.as_str()){return Err("固件格式无效".into());}
        let path=root.join(format!("{}.{}",uuid::Uuid::new_v4(),extension));
        let copy=||->Result<(),String>{let mut input=std::fs::File::open(&image.path).map_err(|e|e.to_string())?;let mut output=std::fs::OpenOptions::new().create_new(true).write(true).open(&path).map_err(|e|e.to_string())?;let mut hash=Sha256::new();let mut bytes=0u64;let mut buffer=[0u8;256*1024];loop{let count=input.read(&mut buffer).map_err(|e|e.to_string())?;if count==0{break;}bytes+=count as u64;if bytes>image.bytes{return Err("固件文件已变化".into());}hash.update(&buffer[..count]);output.write_all(&buffer[..count]).map_err(|e|e.to_string())?;}if bytes!=image.bytes||format!("{:x}",hash.finalize())!=image.sha256{return Err("固件内容已变化，请重新选择并确认".into());}output.sync_all().map_err(|e|e.to_string())?;Ok(())};
        if let Err(error)=copy(){let _=std::fs::remove_file(&path);return Err(error);}
        Ok((Some(path.canonicalize().map_err(|e|e.to_string())?),Some(image.sha256)))
    }).await.map_err(|_|"PROBE.PREPARATION_FAILED: 固件快照失败".to_string())?.map_err(|error|format!("PROBE.PREPARATION_FAILED: {error}"))?;
    let reply=super::invoke_legacy("ui.instrument.probe_operation_safe".into(),json!({"device_id":device_id,"resource":resource,"action":action,"expected_target":target.trim(),"path":staged,"sha256":checksum,"verify":verify,"base_address":address,"consent":true})).await;
    // A timed-out submission may already be executing: ownership stays with the
    // host in that case. Never delete the snapshot or automatically resubmit it.
    if let Err(error)=&reply{if ["INSTRUMENT.TEST_IN_PROGRESS","INSTRUMENT.SESSION_CHANGED","INSTRUMENT.BUSY","INSTRUMENT.INVALID_REQUEST","INSTRUMENT.INVALID_STAGING"].iter().any(|code|error.contains(code)){if let Some(path)=staged{let _=std::fs::remove_file(path);}}}
    reply
}
