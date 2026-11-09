//! Desktop migration adapter: the existing serial AI engine remains the owner.
use super::*;
use crate::backend::{invoke_err as err,invoke_ok as ok,InvokeReply};
use serde::Deserialize;

pub(super) struct Preview {
    token:String,
    created:Instant,
    state:Snapshot,
    data:serde_json::Value,
    question:String,
    config:AiConfig,
    history:String,
    controls:bool,
    watch:Option<WatchOptions>,
}

#[derive(Clone,Deserialize)]
#[serde(deny_unknown_fields)]
struct WatchOptions {
    interval:u64,
    contains:Vec<String>,
    limits:Vec<alerts::Limit>,
}

impl SerialToolPanel {
    pub(crate) fn api_serial_ai(&mut self,ctx:&egui::Context,config:&AiConfig,project:&str,testing:bool,method:&str,params:&serde_json::Value)->InvokeReply {
        self.ai_test_locked=testing;
        self.tick_ai(ctx,self.last_lang,project.into(),config,testing);
        let state=self.ai_state(project,testing);
        let binding=serde_json::to_string(&state.binding).unwrap_or_default();
        let mut chat=std::mem::take(&mut self.ai_chat);
        chat.set_context(project.into(),config);
        let action=method.strip_prefix("ui.ai.serial.").unwrap_or("");
        let result=(||->Result<serde_json::Value,String>{
            if !matches!(action,"state_safe"|"close_safe") && params["binding"].as_str()!=Some(binding.as_str()) {return Err("AI.SOURCE_CHANGED: 当前工程或串口源已变化，请刷新后操作".into());}
            match action {
                "state_safe"=>{},
                "control_safe"=>{
                    let enabled=params["enabled"].as_bool().ok_or("AI.INVALID_REQUEST")?;
                    if enabled&&(testing||chat.pending.is_some()||chat.tool.is_some()||chat.watch.is_some()){return Err("AI.CONTROL_LOCKED: 请先结束当前任务后再授权控制".into());}
                    if !enabled{chat.cancel();}
                    chat.allow_automation=enabled;
                    chat.web_preview=None;
                },
                "preview_safe"=>{
                    if chat.pending.is_some()||chat.tool.is_some(){return Err("AI.BUSY".into());}
                    config.validate().map_err(|error|error.to_string())?;
                    if !config.enabled{return Err("AI.DISABLED".into());}
                    let question=params["question"].as_str().filter(|q|!q.trim().is_empty()&&q.len()<=16*1024).ok_or("AI.INVALID_QUESTION")?.to_owned();
                    let scope=match params["scope"].as_str(){Some("recent")=>DataScope::Recent,Some("none")=>DataScope::None,_=>return Err("AI.INVALID_SCOPE".into())};
                    let watch=if params.get("watch").is_some_and(|v|!v.is_null()) {
                        let options:WatchOptions=serde_json::from_value(params["watch"].clone()).map_err(|_|"AI.INVALID_WATCH")?;
                        if !(MIN_INTERVAL_SECONDS..=MAX_INTERVAL_SECONDS).contains(&options.interval){return Err("AI.INTERVAL_RANGE: 支持 1–300 秒".into());}
                        alerts::validate(&Some(options.contains.clone()),&Some(options.limits.clone()))?;
                        if scope!=DataScope::Recent||!state.binding.live||!state.connection_confirmed{return Err("AI.LIVE_SOURCE_REQUIRED".into());}
                        Some(options)
                    }else{None};
                    let data=self.ai_scoped_observation(scope)?;
                    let token=uuid::Uuid::new_v4().to_string();
                    let history=serde_json::to_string(&chat.history).map_err(|_|"AI.HISTORY_INVALID")?;
                    let reply=json!({"token":token,"destination":config.endpoint().map_err(|e|e.to_string())?,"model":config.model,"question":question,"source":state,"observation":data,"history":chat.history,"controls":chat.allow_automation,"watch":params.get("watch")});
                    chat.web_preview=Some(Preview{token,created:Instant::now(),state:state.clone(),data,question,config:config.clone(),history,controls:chat.allow_automation,watch});
                    return Ok(reply);
                },
                "send_safe"=>{
                    if params["consent"]!=true{return Err("AI.CONSENT_REQUIRED".into());}
                    if chat.pending.is_some()||chat.tool.is_some(){return Err("AI.BUSY".into());}
                    let preview=chat.web_preview.take().ok_or("AI.PREVIEW_REQUIRED")?;
                    if params["token"].as_str()!=Some(preview.token.as_str())||preview.created.elapsed()>Duration::from_secs(180)
                        ||preview.state.binding!=state.binding||preview.config!=*config||preview.controls!=chat.allow_automation
                        ||serde_json::to_string(&chat.history).ok().as_deref()!=Some(preview.history.as_str()) {return Err("AI.PREVIEW_STALE: 内容或权限已变化，请重新预览".into());}
                    chat.open=true;
                    chat.tool_round=0;chat.tool_results.clear();
                    if let Some(options)=preview.watch {
                        chat.interval_seconds=options.interval;
                        chat.rules=alerts::Rules{contains:options.contains,limits:options.limits};
                        self.begin_ai_watch(&mut chat,preview.question,testing)?;
                    }else{
                        // Use exactly the approved data, not a fresh sample.
                        chat.submit(ctx,&preview.state,preview.data,preview.question,false)?;
                    }
                },
                "cancel_safe"=>{chat.cancel();chat.status="已取消等待 / Cancelled".into();},
                "stop_safe"=>{chat.watch=None;chat.cancel();chat.web_preview=None;chat.status="AI监控已停止，串口接收保持原状态".into();},
                "close_safe"=>{chat.close();chat.web_preview=None;},
                "clear_safe"=>{chat.close();chat.entries.clear();chat.history.clear();chat.web_preview=None;},
                "resume_safe"=>{
                    if let Some(watch)=&mut chat.watch {watch.ai_paused=false;watch.failures=0;watch.next=Instant::now();}
                },
                _=>return Err("AI.INVALID_ACTION".into()),
            }
            use std::hash::{Hash,Hasher};
            let mut hash=std::collections::hash_map::DefaultHasher::new();
            for entry in &chat.entries {entry.role.hash(&mut hash);entry.time.hash(&mut hash);entry.text.hash(&mut hash);entry.error.hash(&mut hash);entry.automatic.hash(&mut hash);}
            let messages_revision=format!("{:016x}",hash.finish());
            let mut result=json!({"binding":binding,"source":state,"enabled":config.enabled,"destination":config.endpoint().unwrap_or_default(),"model":config.model,
                "busy":chat.pending.is_some()||chat.tool.is_some(),"controls":chat.allow_automation,"status":chat.status,"status_error":chat.status_error,
                "monitoring":chat.watch.is_some(),"ai_paused":chat.watch.as_ref().is_some_and(|watch|watch.ai_paused),"alerts":chat.alert_count,
                "interval":chat.interval_seconds,"watch_goal":chat.watch.as_ref().map(|watch|watch.goal.as_str()),"messages_revision":messages_revision});
            if params["messages_revision"].as_str()!=Some(messages_revision.as_str()) {
                result["entries"]=json!(chat.entries.iter().map(|entry|json!({"role":entry.role,"text":entry.text,"time":entry.time,"error":entry.error,"automatic":entry.automatic})).collect::<Vec<_>>());
            }
            Ok(result)
        })();
        self.ai_chat=chat;
        ctx.request_repaint();
        match result{Ok(value)=>ok(method,value),Err(error)=>err(method,&error)}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture()->(SerialToolPanel,egui::Context,AiConfig){
        let mut cfg=AppConfig::default();cfg.log_monitor.open_log_files.clear();cfg.log_monitor.save_live_to_disk=false;
        let panel=SerialToolPanel::new(&cfg,Lang::Zh);
        let config=AiConfig{enabled:true,model:"fixture".into(),base_url:"http://127.0.0.1:9/v1".into(),..Default::default()};
        (panel,egui::Context::default(),config)
    }
    #[test]
    fn preview_freezes_the_approved_log_and_requires_fresh_source_and_consent(){
        let(mut panel,ctx,config)=fixture();panel.tabs[0].append_lines(vec!["FIRST VIN=5".into()],true);
        let state=panel.api_serial_ai(&ctx,&config,"lab",false,"ui.ai.serial.state_safe",&json!({}));
        let binding=state.data["binding"].clone();
        let prepared=panel.api_serial_ai(&ctx,&config,"lab",false,"ui.ai.serial.preview_safe",&json!({"binding":binding,"question":"分析","scope":"recent"}));
        assert!(prepared.ok,"{:?}",prepared.error);assert!(panel.ai_chat.pending.is_none());
        panel.tabs[0].append_lines(vec!["LATER VIN=99".into()],true);
        assert!(!panel.ai_chat.web_preview.as_ref().unwrap().data.to_string().contains("LATER"));
        let no_consent=panel.api_serial_ai(&ctx,&config,"lab",false,"ui.ai.serial.send_safe",&json!({"binding":binding,"token":prepared.data["token"]}));
        assert_eq!(no_consent.error.as_deref(),Some("AI.CONSENT_REQUIRED"));assert!(panel.ai_chat.pending.is_none());
        let changed=panel.api_serial_ai(&ctx,&config,"other",false,"ui.ai.serial.send_safe",&json!({"binding":binding,"token":prepared.data["token"],"consent":true}));
        assert!(changed.error.as_deref().unwrap().contains("SOURCE_CHANGED"));assert!(panel.ai_chat.pending.is_none());
    }
    #[test]
    fn unchanged_conversation_poll_does_not_resend_message_bodies(){
        let(mut panel,ctx,config)=fixture();panel.ai_chat.entry("assistant","cached reply");
        let first=panel.api_serial_ai(&ctx,&config,"lab",false,"ui.ai.serial.state_safe",&json!({}));
        assert!(first.data["entries"].is_array());
        let next=panel.api_serial_ai(&ctx,&config,"lab",false,"ui.ai.serial.state_safe",&json!({"messages_revision":first.data["messages_revision"]}));
        assert!(next.data.get("entries").is_none());assert!(next.data["source"].is_object());
    }

    #[test]
    fn control_grant_is_explicit_and_test_lock_cannot_be_bypassed(){
        let(mut panel,ctx,config)=fixture();
        let state=panel.api_serial_ai(&ctx,&config,"lab",false,"ui.ai.serial.state_safe",&json!({}));
        let binding=state.data["binding"].clone();assert_eq!(state.data["controls"],false);
        let allowed=panel.api_serial_ai(&ctx,&config,"lab",false,"ui.ai.serial.control_safe",&json!({"binding":binding,"enabled":true}));
        assert!(allowed.ok);assert_eq!(allowed.data["controls"],true);
        let revoked=panel.api_serial_ai(&ctx,&config,"lab",false,"ui.ai.serial.control_safe",&json!({"binding":binding,"enabled":false}));assert_eq!(revoked.data["controls"],false);
        let blocked=panel.api_serial_ai(&ctx,&config,"lab",true,"ui.ai.serial.control_safe",&json!({"binding":binding,"enabled":true}));assert!(blocked.error.as_deref().unwrap().contains("CONTROL_LOCKED"));
    }
}
