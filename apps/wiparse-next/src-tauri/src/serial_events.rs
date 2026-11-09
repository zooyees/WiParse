//! Authenticated, cancellable localhost event stream. No periodic log reads.
use std::{collections::BTreeMap,io::{Read,Write},net::{Shutdown,TcpStream},sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}},time::Duration};
use serde_json::{json,Value};
use tauri::ipc::Channel;

#[derive(Default)]pub struct SerialEvents{sessions:Arc<Mutex<BTreeMap<String,Arc<Session>>>>}
#[derive(Default)]struct Session{cancel:AtomicBool,socket:Mutex<Option<TcpStream>>}
struct Registration{sessions:Arc<Mutex<BTreeMap<String,Arc<Session>>>>,id:String}
impl Drop for Registration{fn drop(&mut self){if let Ok(mut sessions)=self.sessions.lock(){sessions.remove(&self.id);}}}

#[tauri::command]
pub fn serial_events_start(state:tauri::State<'_,SerialEvents>,id:String,channel:Channel<Value>)->Result<(),String>{
    if uuid::Uuid::parse_str(&id).is_err(){return Err("实时订阅身份无效".into());}
    let session=Arc::new(Session::default());
    {let mut sessions=state.sessions.lock().map_err(|_|"实时订阅状态异常")?;
     if sessions.contains_key(&id){return Err("实时订阅已存在".into());}
     if sessions.len()>=4{return Err("实时订阅正在释放，请稍后重试".into());}
     sessions.insert(id.clone(),session.clone());}
    let registration=Registration{sessions:state.sessions.clone(),id};
    tauri::async_runtime::spawn_blocking(move||{
        let _registration=registration;let mut delay=250;
        while !session.cancel.load(Ordering::Acquire){
            let result=stream(&session,&channel);
            if let Ok(mut current)=session.socket.lock(){if let Some(socket)=current.take(){let _=socket.shutdown(Shutdown::Both);}}
            if session.cancel.load(Ordering::Acquire){break;}
            let reason=result.as_ref().err().map(String::as_str).unwrap_or("事件连接结束");
            if channel.send(json!({"kind":"error","message":format!("串口实时连接中断：{reason}。正在重连；当前画面可能不是最新数据。" )})).is_err(){break;}
            if result.is_ok(){delay=250;}
            // Backoff applies only to a broken stream, never to live log refresh.
            for _ in 0..delay/50{if session.cancel.load(Ordering::Acquire){break;}std::thread::sleep(Duration::from_millis(50));}
            delay=(delay*2).min(2000);
        }
    });Ok(())
}
#[tauri::command]
pub fn serial_events_stop(state:tauri::State<'_,SerialEvents>,id:String){
    if let Ok(sessions)=state.sessions.lock(){if let Some(session)=sessions.get(&id){session.cancel.store(true,Ordering::Release);if let Ok(socket)=session.socket.lock(){if let Some(socket)=socket.as_ref(){let _=socket.shutdown(Shutdown::Both);}}}}
}
impl Drop for SerialEvents{fn drop(&mut self){if let Ok(sessions)=self.sessions.lock(){for session in sessions.values(){session.cancel.store(true,Ordering::Release);if let Ok(socket)=session.socket.lock(){if let Some(socket)=socket.as_ref(){let _=socket.shutdown(Shutdown::Both);}}}}}}

fn auth()->Result<String,String>{
 let agent=ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(2)).timeout_read(Duration::from_secs(2)).build();
 let response=agent.get("http://127.0.0.1:7878/v1/health").call().map_err(|_|"设备服务不可用")?;
 let health=super::read_bounded_bridge_json(response.into_reader(),64*1024)?;
 let pid=super::host_identity(&health)?;
 let response=agent.get("http://127.0.0.1:7878/v1/capabilities").call().map_err(|_|"无法确认实时日志能力")?;
 let capabilities=super::read_bounded_bridge_json(response.into_reader(),1024*1024)?;
 if !capabilities.pointer("/data/events").and_then(Value::as_array).is_some_and(|events|events.iter().any(|value|value.as_str()==Some("serial.log.changed"))){return Err("设备宿主不支持实时日志事件，请更新并重启随包的 WiParse-Host.exe".into());}
 let owned=super::OWNED_AUTH.lock().unwrap_or_else(|e|e.into_inner()).as_ref().filter(|(id,_)|*id==pid).map(|(_,token)|token.clone());
 let token=if let Some(token)=owned{token}else{let reference=health.pointer("/data/ai_auth_ref").and_then(Value::as_str).ok_or("本机服务凭据不可用")?;wiparse_core::ai::load_local_bridge_token(reference)?};
 if token.is_empty()||token.contains(['\r','\n']){return Err("本机服务凭据无效".into());}Ok(token)
}

// Incomplete records survive timeout boundaries. Limits apply before parsing.
#[derive(Default)]struct Records{pending:Vec<u8>,headers:bool}
impl Records{
 fn push(&mut self,bytes:&[u8],mut changed:impl FnMut())->Result<(),String>{
  self.pending.extend_from_slice(bytes);
  if !self.headers{
   if let Some(end)=self.pending.windows(4).position(|part|part==b"\r\n\r\n"){
    if end>16384{return Err("事件响应头过大".into());}
    let header=std::str::from_utf8(&self.pending[..end]).map_err(|_|"无效事件响应")?;
    if !header.starts_with("HTTP/1.1 200 ")&&!header.starts_with("HTTP/1.0 200 "){return Err("事件订阅被拒绝".into());}
    self.pending.drain(..end+4);self.headers=true;
   }else{if self.pending.len()>16384{return Err("事件响应头过大".into());}return Ok(());}
  }
  let mut consumed=0;
  for line in self.pending.split_inclusive(|byte|*byte==b'\n'){
   if !line.ends_with(b"\n"){break;}
   if line.len()>16*1024*1024{return Err("事件记录过大".into());}
   let event:Value=serde_json::from_slice(line).map_err(|_|"无效事件记录")?;
   if matches!(event["type"].as_str(),Some("subscribed"|"serial.log.changed")){changed();}
   consumed+=line.len();
  }
  self.pending.drain(..consumed);
  if self.pending.len()>16*1024*1024{return Err("事件记录过大".into());}Ok(())
 }
}
fn stream(session:&Session,channel:&Channel<Value>)->Result<(),String>{
 let token=auth()?;if session.cancel.load(Ordering::Acquire){return Ok(());}
 let mut socket=TcpStream::connect_timeout(&"127.0.0.1:7878".parse().unwrap(),Duration::from_secs(2)).map_err(|e|e.to_string())?;
 socket.set_read_timeout(Some(Duration::from_secs(1))).map_err(|e|e.to_string())?;
 socket.set_write_timeout(Some(Duration::from_secs(2))).map_err(|e|e.to_string())?;
 {let mut current=session.socket.lock().map_err(|_|"事件连接状态异常")?;*current=Some(socket.try_clone().map_err(|e|e.to_string())?);}
 write!(socket,"GET /v1/events?since_seq=0&types=serial.log.changed HTTP/1.1\r\nHost: 127.0.0.1:7878\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n").map_err(|e|e.to_string())?;
 let mut records=Records::default();let mut buffer=[0u8;8192];
 while !session.cancel.load(Ordering::Acquire){match socket.read(&mut buffer){
  Ok(0)=>return Err("事件连接已关闭".into()),
  Ok(count)=>{let mut changed=false;records.push(&buffer[..count],||changed=true)?;if changed&&channel.send(json!({"kind":"changed"})).is_err(){session.cancel.store(true,Ordering::Release);break;}},
  Err(e) if matches!(e.kind(),std::io::ErrorKind::TimedOut|std::io::ErrorKind::WouldBlock|std::io::ErrorKind::Interrupted)=>{},
  Err(e)=>return Err(e.to_string()),
 }}
 Ok(())
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn fragmented_headers_and_records_survive_idle_reads(){
  let mut records=Records::default();let mut changes=0;
  for part in [b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\n\r\n{\"type\":\"sub".as_slice(),b"scribed\"}\n{\"type\":\"serial.log.changed\"}\n{\"type\":\"serial.line\"}\n"]{records.push(part,||changes+=1).unwrap();}
  assert_eq!(changes,2);assert!(records.pending.is_empty());
 }
 #[test]fn rejected_and_oversized_streams_do_not_look_like_success(){
  assert!(Records::default().push(b"HTTP/1.1 401 Unauthorized\r\n\r\n",||panic!()).is_err());
  assert!(Records::default().push(&vec![b'a';16385],||panic!()).is_err());
  let mut records=Records{headers:true,..Records::default()};assert!(records.push(b"not JSON\n",||panic!()).is_err());
 }
}
