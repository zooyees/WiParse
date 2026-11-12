//! Content-free request measurements, emitted through the existing application log.
use super::*;

pub fn measurement(request: &AiRequest, history: &[AiMessage], tools: Option<&tools::Conversation>,
    elapsed_ms: u128, reply: &Result<AiReply,AiError>) -> Value {
    let payload=serde_json::from_str::<Value>(&request.text).unwrap_or(Value::Null);
    let task=payload["task_id"].as_str().or_else(||payload["observation"]["task"]["id"].as_str())
        .and_then(|s|uuid::Uuid::parse_str(s).ok()).map(|id|id.to_string());
    let mut usage=usage::Usage::default();
    match reply {Ok(r)=>usage.record(&r.usage),Err(_)=>usage.failure()}
    json!({"schema":1,"task_id":task,"elapsed_ms":elapsed_ms,"success":reply.is_ok(),
        "purpose_bytes":request.purpose.len(),"payload_bytes":request.text.len(),
        "history_bytes":serde_json::to_vec(history).map_or(0,|v|v.len()),
        "function_bytes":tools.map_or(0,|t|serde_json::to_vec(&t.functions).map_or(0,|v|v.len())),
        "tool_history_bytes":tools.map_or(0,|t|serde_json::to_vec(&t.rounds).map_or(0,|v|v.len())),
        "host_update_bytes":tools.and_then(|t|t.host_context.as_ref()).map_or(0,|s|s.len()),
        "usage":usage,"measurement":"UTF-8 bytes are not tokens; usage is provider-reported only"})
}

/// Optional request ledger for a benchmark invocation. The start record survives a killed runner.
pub(super) struct BenchmarkRequest(Option<(std::path::PathBuf,uuid::Uuid)>);
impl BenchmarkRequest {
    pub(super) fn start()->Self{
        let r=Self(std::env::var_os("WIPARSE_BENCHMARK_USAGE_FILE").map(|path|(path.into(),uuid::Uuid::new_v4())));
        r.event("started",Value::Null);r
    }
    pub(super) fn usage(&self,u:&Value){
        let mut cost=usage::Usage::default();cost.record(u);self.event("response",json!(cost.total_tokens));
    }
    fn event(&self,phase:&str,cost:Value){
        use std::io::Write;
        if let Some((path,id))=&self.0 {
            let line=format!("{}\n",json!({"request_id":id,"phase":phase,"total_tokens":cost}));
            if let Err(error)=std::fs::OpenOptions::new().append(true).create(true).open(path).and_then(|mut f|f.write_all(line.as_bytes())){
                tracing::warn!(%error,"benchmark usage ledger unavailable");
            }
        }
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn metrics_exclude_content_and_unknown_usage_is_not_zero() {
        let secret="private-log-and-key-must-not-appear";
        let request=AiRequest{purpose:secret.into(),text:json!({"task_id":secret,"log":secret}).to_string()};
        let reply=Ok(AiReply{text:secret.into(),model:secret.into(),usage:Value::Null,finish_reason:None,tool_message:None});
        let m=measurement(&request,&[AiMessage::User(secret.into())],None,120,&reply);
        assert!(!m.to_string().contains(secret));assert!(m["task_id"].is_null());
        assert!(m["usage"]["total_tokens"].is_null());assert_eq!(m["usage"]["reported_responses"],0);
    }
}
