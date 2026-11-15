//! Incremental Chat Completions SSE. Tool calls are released only after validation.
use super::*;
use std::io::{BufRead, BufReader};

pub(super) fn read(
    config: &AiConfig, input: impl Read, cancelled: &AtomicBool,
    progress: &std::sync::Mutex<Option<String>>,
) -> Result<AiReply, AiError> {
    let mut reader = BufReader::new(input.take(MAX_RESPONSE + 1));
    let mut line = Vec::new();
    let mut bytes = 0;
    let mut message = json!({"role":"assistant","content":""});
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut calls: Vec<Value> = Vec::new();
    let mut model = config.model.clone();
    let mut usage = Value::Null;
    let mut finish = None;
    let mut published = std::time::Instant::now();
    let mut preview_capped = false;
    loop {
        if cancelled.load(Ordering::Acquire) { return Err(AiError::Cancelled); }
        line.clear();
        let count = reader.read_until(b'\n', &mut line).map_err(|_|AiError::Network)?;
        bytes += count as u64;
        if bytes > MAX_RESPONSE { return Err(AiError::ResponseDetail("服务响应超过允许的字节数")); }
        if count == 0 { break; }
        let line = std::str::from_utf8(&line).map_err(|_|AiError::Response)?;
        let Some(data) = line.strip_prefix("data:").map(str::trim) else { continue; };
        if data == "[DONE]" { break; }
        let event: Value = serde_json::from_str(data).map_err(|_|AiError::ResponseDetail("服务返回了无效的流式 JSON 片段"))?;
        if event.get("error").is_some() { return Err(AiError::Response); }
        if let Some(name) = event["model"].as_str() { model = name.into(); }
        if !event["usage"].is_null() { usage = event["usage"].clone(); }
        let choice = &event["choices"][0];
        let delta = &choice["delta"];
        append_text_content(&mut text,&delta["content"]);
        if let Some(chunk) = delta["reasoning_content"].as_str() { reasoning.push_str(chunk); }
        if let Some(parts) = delta["tool_calls"].as_array() {
            for part in parts {
                let index = part["index"].as_u64().filter(|i|*i<4).ok_or(AiError::Response)? as usize;
                while calls.len()<=index { calls.push(json!({"id":"","type":"function","function":{"name":"","arguments":""}})); }
                for field in ["id"] { if let Some(v)=part[field].as_str() { let mut old=calls[index][field].as_str().unwrap_or("").to_owned(); old.push_str(v); calls[index][field]=json!(old); } }
                for field in ["name","arguments"] { if let Some(v)=part["function"][field].as_str() { let mut old=calls[index]["function"][field].as_str().unwrap_or("").to_owned(); old.push_str(v); calls[index]["function"][field]=json!(old); } }
            }
        }
        if let Some(reason) = choice["finish_reason"].as_str() { finish = Some(reason.to_owned()); }
        if published.elapsed() >= Duration::from_millis(100) && !text.is_empty() && !preview_capped
            && !text.trim_start().starts_with('{') && !text.trim_start().starts_with("```json") {
            let preview = if text.len() > 256 * 1024 {
                preview_capped = true;
                let mut end = 256 * 1024;
                while !text.is_char_boundary(end) { end -= 1; }
                format!("{}\n\n流式预览已达 256 KiB，完整回复仍在接收 / Preview capped; full response is still receiving.",&text[..end])
            } else { text.clone() };
            if let Ok(mut snapshot) = progress.lock() { *snapshot = Some(preview); }
            published = std::time::Instant::now();
        }
    }
    message["content"] = json!(text);
    if !reasoning.is_empty() { message["reasoning_content"] = json!(reasoning); }
    if !calls.is_empty() {
        // Never execute partial/truncated tool arguments, even if JSON happens to parse.
        if finish.as_deref() != Some("tool_calls") { return Err(AiError::Response); }
        message["tool_calls"] = json!(calls);
    }
    parse_reply(config,json!({"model":model,"usage":usage,"choices":[{"message":message,"finish_reason":finish}]}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publishes_content_before_eof_without_reasoning_or_arguments() {
        struct Chunks<'a> { index: usize, progress: &'a std::sync::Mutex<Option<String>>, seen: &'a AtomicBool }
        impl Read for Chunks<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                if self.index==1 { std::thread::sleep(Duration::from_millis(110)); }
                if self.index==2 { self.seen.store(self.progress.lock().unwrap().as_deref()==Some("你好"),Ordering::Release); }
                let chunk=match self.index {
                    0=>"data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"private reasoning\",\"content\":\"你\"}}]}\n\n",
                    1=>"data: {\"choices\":[{\"delta\":{\"content\":\"好\"}}]}\n\n",
                    2=>"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n",
                    _=>return Ok(0),
                };
                self.index+=1;out[..chunk.len()].copy_from_slice(chunk.as_bytes());Ok(chunk.len())
            }
        }
        let progress=std::sync::Mutex::new(None);let seen=AtomicBool::new(false);
        let reply=read(&AiConfig::default(),Chunks{index:0,progress:&progress,seen:&seen},&AtomicBool::new(false),&progress).unwrap();
        assert!(seen.load(Ordering::Acquire));assert_eq!(reply.text,"你好");
    }
    #[test]
    fn native_tool_fragments_are_assembled_and_truncated_calls_rejected() {
        let data = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"wiparse_call\",\"arguments\":\"{\\\"method\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"serial.view\\\",\\\"params\\\":{}}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n"
        );
        let reply=read(&AiConfig::default(),data.as_bytes(),&AtomicBool::new(false),&std::sync::Mutex::new(None)).unwrap();
        let call=&reply.tool_message.unwrap().calls[0];
        assert_eq!(call.name,"wiparse_call");
        assert_eq!(serde_json::from_str::<Value>(&call.arguments).unwrap()["method"],"serial.view");
        assert!(read(&AiConfig::default(),data.replace("tool_calls\"}]}","length\"}]}").as_bytes(),&AtomicBool::new(false),&std::sync::Mutex::new(None)).is_err());
    }
}
