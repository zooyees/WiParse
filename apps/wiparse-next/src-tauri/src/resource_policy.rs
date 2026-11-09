//! Configurable allocation budget, independent of legacy presentation limits.
use std::{path::Path,sync::atomic::{AtomicUsize,Ordering}};
use serde_json::{json,Value};
static MIB:AtomicUsize=AtomicUsize::new(2048);
pub fn bytes()->usize{MIB.load(Ordering::Acquire)*1024*1024}
pub fn points()->usize{bytes()/48}
pub fn source_limit()->usize{bytes()/65536}
fn path()->std::path::PathBuf{wiparse_core::paths::app_root().join("next-resource-policy.json")}
pub fn init(){if let Ok(raw)=std::fs::read(path()){if let Ok(v)=serde_json::from_slice::<Value>(&raw){if let Some(n)=v["memory_mib"].as_u64().filter(|n|(512..=16384).contains(n)){MIB.store(n as usize,Ordering::Release)}}}}
pub fn open_stable(path:&Path)->Result<std::fs::File,String>{let mut options=std::fs::OpenOptions::new();options.read(true);#[cfg(windows)]{use std::os::windows::fs::OpenOptionsExt;options.share_mode(1);}options.open(path).map_err(|e|format!("无法稳定读取文件（文件可能正被写入）：{e}"))}
#[tauri::command]pub fn resource_policy_get()->Value{json!({"memory_mib":MIB.load(Ordering::Acquire),"wave_points":points(),"source_metadata_limit":source_limit(),"text_bytes":bytes()/2})}
#[tauri::command]pub fn resource_policy_save(memory_mib:usize)->Result<Value,String>{if !(512..=16384).contains(&memory_mib){return Err("内存预算范围为 512–16384 MiB，请为操作系统和其他工具保留内存".into())}let target=path();let temp=target.with_extension(format!("{}.tmp",uuid::Uuid::new_v4()));std::fs::write(&temp,serde_json::to_vec(&json!({"memory_mib":memory_mib})).unwrap()).map_err(|e|e.to_string())?;std::fs::rename(temp,target).map_err(|e|e.to_string())?;MIB.store(memory_mib,Ordering::Release);Ok(resource_policy_get())}
