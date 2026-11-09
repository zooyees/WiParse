//! Complete-runtime updates. Staging never edits the running installation.
use std::{path::{Path,PathBuf},sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}},time::Duration,io::{Read,Write}};
use serde_json::{json,Value};
use sha2::{Digest,Sha256};

#[derive(Default)]pub struct UpdateState{busy:Arc<AtomicBool>,prepared:Mutex<Option<PathBuf>>}
struct Busy(Arc<AtomicBool>);impl Drop for Busy{fn drop(&mut self){self.0.store(false,Ordering::Release);}}
fn root()->Result<PathBuf,String>{let exe=std::env::current_exe().map_err(|e|e.to_string())?;let bin=exe.parent().ok_or("运行目录无效")?;if bin.file_name().and_then(|s|s.to_str())!=Some("bin"){return Err("更新仅支持独立运行包；开发目录不执行更新".into())}bin.parent().ok_or("运行目录无效".into()).map(Path::to_owned)}
fn sessions(root:&Path)->Vec<PathBuf>{std::fs::read_dir(root.join(".updates")).into_iter().flatten().filter_map(Result::ok).map(|e|e.path()).filter(|p|p.is_dir()).collect()}
pub fn recovery_required()->Result<(),String>{let Ok(root)=root()else{return Ok(())};for session in sessions(&root){let p=session.join("journal.json");if !p.exists(){continue}let journal:Value=serde_json::from_slice(&std::fs::read(&p).map_err(|e|e.to_string())?).map_err(|_|"更新日志损坏；请从设置执行回滚恢复")?;if journal["phase"]=="applying"{return Err("检测到未完成更新，请先在设置中执行回滚恢复".into())}}Ok(())}
fn rollback_allowed(root:&Path,session:&Path)->Result<bool,String>{
 let journal:Value=serde_json::from_slice(&std::fs::read(session.join("journal.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
 for other in sessions(root){if other==session{continue}if let Ok(bytes)=std::fs::read(other.join("journal.json")){let value:Value=serde_json::from_slice(&bytes).map_err(|_|"更新日志损坏")?;if value["phase"]=="applying"{return Ok(false)}}}
 if journal["phase"]=="applying"{return Ok(true)}if journal["phase"]!="installed"{return Ok(false)}
 let current=std::fs::read(root.join("runtime-manifest.json")).map_err(|e|e.to_string())?;let installed=std::fs::read(session.join("manifest.json")).map_err(|e|e.to_string())?;Ok(Sha256::digest(current)==Sha256::digest(installed))
}
fn script_request(root:&Path,session:&Path,mode:&str,archive:Option<&Path>,host:u32,restart:bool)->Result<(PathBuf,PathBuf),String>{
    std::fs::create_dir_all(session).map_err(|e|e.to_string())?;
    let script=session.join(format!("helper-{}.ps1",uuid::Uuid::new_v4()));let request=session.join(format!("request-{}.json",uuid::Uuid::new_v4()));
    std::fs::write(&script,include_str!("runtime_update.ps1")).map_err(|e|e.to_string())?;
    std::fs::write(&request,serde_json::to_vec(&json!({"root":root,"session":session,"mode":mode,"archive":archive,"parent_pid":std::process::id(),"host_pid":host,"restart":restart})).unwrap()).map_err(|e|e.to_string())?;Ok((script,request))
}
fn powershell(script:&Path,request:&Path)->std::process::Command{let mut cmd=std::process::Command::new("powershell.exe");cmd.args(["-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File"]).arg(script).arg("-Request").arg(request);#[cfg(windows)]{use std::os::windows::process::CommandExt;cmd.creation_flags(0x08000000);}cmd}
fn stage(root:&Path,path:&Path)->Result<Value,String>{let session=root.join(".updates").join(uuid::Uuid::new_v4().to_string());let (script,request)=script_request(root,&session,"stage",Some(path),0,false)?;let output=powershell(&script,&request).output().map_err(|e|format!("无法启动更新校验：{e}"))?;let value:Value=serde_json::from_slice(&output.stdout).map_err(|_|format!("更新校验失败：{}",String::from_utf8_lossy(&output.stderr).chars().take(1200).collect::<String>()))?;if !output.status.success()||value["ok"]!=true{return Err(value["error"].as_str().unwrap_or("更新校验失败").into())}Ok(value)}
fn valid_url(url:&str)->bool{(url.starts_with("https://")||url.starts_with("http://127.0.0.1:")||url.starts_with("http://localhost:"))&&!url.contains('@')&&!url.chars().any(char::is_control)}
fn origin(url:&str)->Option<&str>{let i=url.find("://")?+3;Some(&url[..url[i..].find('/').map(|x|x+i).unwrap_or(url.len())])}
fn fetch_manifest(url:&str)->Result<Value,String>{if !valid_url(url)||url.len()>4096{return Err("更新地址须为 HTTPS 或本机测试服务".into())}let response=ureq::AgentBuilder::new().redirects(0).timeout(Duration::from_secs(30)).build().get(url).call().map_err(|e|format!("检查更新失败：{e}"))?;let mut bytes=Vec::new();response.into_reader().take(1024*1024+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;if bytes.len()>1024*1024{return Err("更新索引过大".into())}let value:Value=serde_json::from_slice(&bytes).map_err(|_|"更新索引不是有效 JSON")?;let archive=value["archive_url"].as_str().unwrap_or("");if value["product"]!="wiparse-next"||value["schema"]!=1||value["sha256"].as_str().is_none_or(|s|s.len()!=64||!s.bytes().all(|b|b.is_ascii_hexdigit()))||!valid_url(archive)||origin(url)!=origin(archive)||value["bytes"].as_u64().is_none_or(|n|n==0||n>4*1024*1024*1024){return Err("更新索引不符合新版整包契约或下载地址跨域".into())}Ok(value)}

#[tauri::command]pub async fn runtime_update_status()->Result<Value,String>{tauri::async_runtime::spawn_blocking(||{let root=match root(){Ok(root)=>root,Err(reason)=>return Ok(json!({"supported":false,"reason":reason}))};let manifest=std::fs::read(root.join("runtime-manifest.json")).ok().and_then(|b|serde_json::from_slice::<Value>(&b).ok());let history=sessions(&root).into_iter().filter_map(|path|{let value:Value=serde_json::from_slice(&std::fs::read(path.join("journal.json")).ok()?).ok()?;Some(json!({"session":path.file_name()?.to_str()?,"phase":value["phase"],"version":value["version"],"can_rollback":rollback_allowed(&root,&path).unwrap_or(false)}))}).collect::<Vec<_>>();Ok(json!({"supported":manifest.is_some(),"current":manifest.as_ref().map(|m|json!({"version":m["version"],"build":m["build"]})),"history":history,"recovery_required":recovery_required().err()}))}).await.map_err(|e|e.to_string())?}
#[tauri::command]pub async fn runtime_update_check(url:String)->Result<Value,String>{tauri::async_runtime::spawn_blocking(move||fetch_manifest(&url)).await.map_err(|e|e.to_string())?}
#[tauri::command]pub async fn runtime_update_prepare(state:tauri::State<'_,UpdateState>,url:Option<String>)->Result<Option<Value>,String>{
 if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("更新任务进行中".into())}let busy=state.busy.clone();let value=tauri::async_runtime::spawn_blocking(move||{let _guard=Busy(busy);let root=root()?;recovery_required()?;
 let path=if let Some(url)=url{let manifest=fetch_manifest(&url)?;let directory=root.join(".updates");std::fs::create_dir_all(&directory).map_err(|e|e.to_string())?;let path=directory.join(format!("download-{}.zip",uuid::Uuid::new_v4()));let response=ureq::AgentBuilder::new().redirects(0).timeout(Duration::from_secs(300)).build().get(manifest["archive_url"].as_str().unwrap()).call().map_err(|e|format!("下载失败：{e}"))?;let mut reader=response.into_reader();let mut file=std::fs::OpenOptions::new().create_new(true).write(true).open(&path).map_err(|e|e.to_string())?;let mut hash=Sha256::new();let mut bytes=0u64;let mut buffer=[0u8;65536];let result=(||{loop{let n=reader.read(&mut buffer).map_err(|e|e.to_string())?;if n==0{break}bytes+=n as u64;if bytes>manifest["bytes"].as_u64().unwrap(){return Err("下载大小超出索引".to_owned())}hash.update(&buffer[..n]);file.write_all(&buffer[..n]).map_err(|e|e.to_string())?;}file.sync_all().map_err(|e|e.to_string())?;if bytes!=manifest["bytes"].as_u64().unwrap()||format!("{:x}",hash.finalize())!=manifest["sha256"].as_str().unwrap().to_ascii_lowercase(){return Err("下载校验失败".into())}Ok(())})();drop(file);if let Err(error)=result{let _=std::fs::remove_file(path);return Err(error)}path
 }else{let Some(path)=rfd::FileDialog::new().add_filter("新版完整运行包",&["zip"]).pick_file()else{return Ok(None)};path};stage(&root,&path).map(Some)
 }).await.map_err(|e|e.to_string())??;if let Some(ref value)=value{*state.prepared.lock().map_err(|_|"更新状态异常")?=value["session"].as_str().map(PathBuf::from)}Ok(value)
}
#[tauri::command]pub async fn runtime_update_apply(app:tauri::AppHandle,state:tauri::State<'_,UpdateState>,rollback_session:Option<String>,consent:bool)->Result<(),String>{
 if state.busy.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err("更新任务进行中".into())}let _busy=Busy(state.busy.clone());
 if !consent{return Err("请确认关闭程序并更新或恢复整个运行包".into())}let root=root()?;
 let host=super::OWNED_HOST.lock().map_err(|_|"宿主状态异常")?.as_ref().map(|c|c.id());let observed=super::probe_host()?;
 if observed.is_some()&&observed!=host{return Err("设备宿主属于其他窗口，请先正常关闭其他窗口再重试".into())}
 if observed.is_some(){super::invoke_legacy_at(super::ENDPOINT,"ui.runtime.idle_safe",json!({}))?;}
 let (session,mode)=if let Some(id)=rollback_session{if id.len()!=36||uuid::Uuid::parse_str(&id).is_err(){return Err("回滚会话无效".into())}(root.join(".updates").join(id),"rollback")}else{(state.prepared.lock().map_err(|_|"更新状态异常")?.clone().ok_or("请先准备并校验更新包")?,"apply")};
 if mode=="rollback"&&!rollback_allowed(&root,&session)?{return Err("仅允许回滚当前运行版本对应的更新，请先恢复未完成更新".into())}
 let (script,request)=script_request(&root,&session,mode,None,host.unwrap_or(0),true)?;powershell(&script,&request).spawn().map_err(|e|format!("启动更新助手失败：{e}"))?;app.exit(0);Ok(())
}

#[cfg(test)]mod tests{use super::*;#[test]fn update_sources_reject_plain_http_credentials_and_cross_origin(){assert!(valid_url("https://updates.example/next.json"));assert!(!valid_url("http://example/a"));assert!(!valid_url("https://user@host/a"));assert_eq!(origin("https://a:42/x"),Some("https://a:42"));assert_ne!(origin("https://a/x"),origin("https://b/x"));}}
