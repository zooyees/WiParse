//! Opt-in Promptfoo transport through the production serial assistant. Runs in a temporary project.
use super::*;
use std::{env,fs,path::PathBuf};
use wiparse_core::ai::{AiProtocol,AiProvider,AiToolMode};

fn model_config(provider:&str)->Result<AiConfig,String>{
    let mut config=AiConfig::default();
    match provider {
        "deepseek" => {
            let json:serde_json::Value=serde_json::from_slice(&fs::read(env::var("WIPARSE_EVAL_CONFIG").map_err(|_|"missing config path")?).map_err(|_|"cannot read WiParse config")?).map_err(|_|"invalid WiParse config")?;
            config=serde_json::from_value(json["ai"].clone()).map_err(|_|"invalid ai configuration")?;
            if !matches!(config.base_url.trim_end_matches('/'),"https://api.deepseek.com"|"https://api.deepseek.com/v1"|"https://api.deepseek.com/chat/completions"|"https://api.deepseek.com/v1/chat/completions") {return Err("expected official DeepSeek endpoint".into());}
            config.provider=AiProvider::DeepSeek;
        }
        "qwen" => {
            if env::var("DASHSCOPE_API_KEY").unwrap_or_default().is_empty(){return Err("DASHSCOPE_API_KEY unavailable".into());}
            config.provider=AiProvider::Qwen;
            config.base_url=match env::var("WIPARSE_EVAL_QWEN_REGION").as_deref(){
                Ok("intl")=>"https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
                Ok("cn")|Err(_)=>"https://dashscope.aliyuncs.com/compatible-mode/v1",
                _=>return Err("unknown DashScope region".into())}.into();
            config.api_key_env="DASHSCOPE_API_KEY".into();
            config.model=env::var("WIPARSE_EVAL_QWEN_MODEL").unwrap_or_else(|_|"qwen-plus".into());
        }
        _=>return Err("unknown provider".into())
    }
    config.enabled=true;config.stream=false;config.thinking=Some(false);
    config.protocol=AiProtocol::ChatCompletions;config.tool_mode=AiToolMode::Auto;
    config.max_output_tokens=2048;config.timeout_sec=90;
    Ok(config)
}

#[test]
#[ignore="explicit paid model evaluation through tools/agent-eval"]
fn promptfoo_live_serial_session() {
    let case=env::var("WIPARSE_EVAL_CASE").unwrap();
    let provider=env::var("WIPARSE_EVAL_PROVIDER").unwrap();
    let cases:serde_json::Value=serde_json::from_str(include_str!("../../../../../tools/agent-eval/cases.json")).unwrap();
    let prompts=cases[&case].as_array().unwrap();
    let dir=PathBuf::from(env::var("WIPARSE_PROJECT_ROOT").unwrap());
    assert_eq!(dir.canonicalize().unwrap(),env::current_dir().unwrap().canonicalize().unwrap());
    let save=|v:serde_json::Value|fs::write(dir.join("trajectory.json"),serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let config=match model_config(&provider){Ok(v)=>v,Err(e)=>{save(json!({"availability":"unavailable","error":e,"provider":provider}));return;}};
    let ctx=egui::Context::default();
    crate::ai::AiPanel::new(&config).install(&ctx);
    let mut cfg=AppConfig::default();
    cfg.log_monitor.open_log_files.clear();cfg.log_monitor.save_live_to_disk=false;
    let mut panel=SerialToolPanel::new(&cfg,Lang::Zh);
    panel.ports.clear();
    let path=dir.join("charging-fixture.log");
    fs::write(&path, include_str!("../../../../../tools/agent-eval/charging-fixture.txt")).unwrap();
    panel.active_tab=panel.open_path(path).unwrap();
    let until=Instant::now()+Duration::from_secs(10);
    while !panel.pending_loads.is_empty() && Instant::now()<until {panel.poll_file_loads();thread::sleep(Duration::from_millis(5));}
    assert_eq!(panel.tabs[panel.active_tab].line_count(),10);
    panel.ai_chat.open=true;panel.ai_chat.set_context("promptfoo-fixture".into(),&config);
    let mut availability="ran";
    let mut turns=Vec::new();let mut seen=std::collections::HashSet::new();
    for prompt in prompts{
        let started=Instant::now();
        let state=panel.ai_state("promptfoo-fixture",false);
        let mut chat=std::mem::take(&mut panel.ai_chat);
        let first=chat.entries.len();
        let begin=panel.begin_manual_agent(&ctx,&mut chat,&state,prompt.as_str().unwrap().to_owned());
        panel.ai_chat=chat;
        if let Err(e)=begin {save(json!({"availability":"error","provider":provider,"error":e,"turns":turns}));return;}
        let mut calls=Vec::new();
        while panel.ai_chat.working() && started.elapsed()<Duration::from_secs(240){
            panel.tick_ai(&ctx,Lang::Zh,"promptfoo-fixture".into(),&config,false);
            for row in &panel.ai_chat.tool_results{
                if seen.insert(row["evidence_id"].clone().to_string()){
                    calls.push(json!({"id":row["evidence_id"],"method":row["method"],"ok":row["result"]["ok"],
                        "error":row["result"]["error"],"coverage":row["result"]["data"]["coverage"],
                        "transitions":row["result"]["data"]["state_transition_analysis"]["confirmed_active_to_inactive_transitions"]}));
                }
            }
            thread::sleep(Duration::from_millis(15));
        }
        let timeout=panel.ai_chat.working();
        if timeout{panel.ai_chat.cancel();}
        let chat=&panel.ai_chat;
        if chat.status_error && ["AI.NETWORK", "AI.HTTP.", "AI.CREDENTIAL", "AI.CONFIG", "AI.DISABLED"]
            .iter().any(|s| chat.status.contains(s)) {
            availability="transport_error";
        }
        let run=chat.run.as_ref().unwrap();
        let answer=chat.entries.iter().skip(first).rev().find(|e|e.role=="assistant").map(|e|&e.text);
        turns.push(json!({"task":run.id,"phase":run.phase,"status":chat.status,"timeout":timeout,"answer":answer,
            "calls":calls,"plan":run.workflow.state()["steps"],"evidence_ids":run.evidence_ids,
            "elapsed_ms":started.elapsed().as_millis(),"usage_cumulative":chat.usage_total}));
        if chat.status_error || timeout{break;}
    }
    save(json!({"availability":availability,"case":case,"provider":provider,"model":config.model,"turns":turns}));
}
