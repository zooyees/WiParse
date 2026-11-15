//! Dispatch only requests issued and authorized by the bounded task session.
use super::*;
use crate::ai::agent::{self, ContextState, HostCall, HostReply};
use serde_json::{json, Value};

fn reply(r: crate::backend::InvokeReply) -> Value {
    json!({"ok":r.ok,"data":r.data,"error":r.error})
}
fn wait(method: &str, params: Value) -> Value {
    json!({"_agent_wait":{"method":method,"params":params,"interval_ms":100}})
}
fn test_owner_matches(owner:Option<&(String,String)>, task:Option<&str>, run:Option<&str>)->bool {
    owner.is_some_and(|(expected_task,expected_run)|task==Some(expected_task.as_str()) && run==Some(expected_run.as_str()))
}
fn scope_observation(plan:&Value,job:&Value,start:i64,end:i64)->Value{
    use wiparse_core::instrument::ScopeMeasType;
    let v=&plan["verify"];
    let metric=serde_json::from_value::<ScopeMeasType>(v["meas_type"].clone()).ok();
    let raw=job["detail"]["response"].as_str().unwrap_or("");
    let reading=metric.and_then(|m|{
        let prefix=format!("CH{} {}: ",v["channel"].as_u64()?,m.label());
        let (digits,unit)=raw.strip_prefix(&prefix)?.split_once(' ')?;
        let (prefix,base)=unit.trim().split_at_checked(unit.trim().len().checked_sub(m.base_unit().len())?)?;
        if base!=m.base_unit(){return None;}
        let scale=match prefix{""=>1.,"m"=>1e-3,"µ"|"μ"|"u"=>1e-6,"n"=>1e-9,"p"=>1e-12,"k"=>1e3,"M"=>1e6,"G"=>1e9,_=>return None};
        let value=digits.parse::<f64>().ok()?*scale;
        (job["ok"]==true && value.is_finite() && value.abs()<1e35 && end>=start &&
            end-start<=v["window_ms"].as_i64()?).then_some(value)
    });
    let (lo,hi,u)=(v["min"].as_f64(),v["max"].as_f64(),v["uncertainty"].as_f64());
    let comparison=match (reading,lo,hi,u){(Some(x),Some(lo),Some(hi),Some(u)) if x+u<lo || x-u>hi=>"outside",
        (Some(x),Some(lo),Some(hi),Some(u)) if x-u>=lo && x+u<=hi=>"inside",_=>"unknown"};
    json!({"kind":"independent_scope_display_reading","source":plan["source"],"observer":plan["observer"],"channel":v["channel"],
        "metric":v["meas_type"],"unit":metric.map(|m|m.base_unit()),"value":reading,"display_response":raw,"display_comparison":comparison,"rule":v,
        "job_id":job["job_id"],"job_session":job["job_session"],"host_read_window_utc_ms":[start,end],"physical_verified":false,
        "scope":"Instrument display precision; acquisition timestamp, probe wiring and calibration are not established by a query."})
}
impl WiParseApp {
    fn agent_context(&self) -> ContextState {
        let project = self.test_tool.current_project_id().to_owned();
        let binding = [
            "serial",
            "waveform",
            "data_analysis",
            "test_report",
            "test_tool",
            "calculator",
            "instruments",
        ]
        .into_iter()

        .map(|page| {
            let identity = match page {
                "serial" => self.serial.agent_identity(&project),
                "waveform" => self.waveform.agent_identity(),
                "data_analysis" => self.data_analysis.agent_identity(),
                "test_report" => self.test_report.agent_identity(),
                "test_tool" => self.test_tool.agent_identity(),
                "calculator" => self.calculator.ai_summary().to_string(),
                _ => self.instruments.published_agent_identity().to_owned(),
            };
            (page.to_string(),Value::String(identity))
        })
        .collect::<serde_json::Map<_,_>>();
        let mut binding=binding;
        binding.insert("route_sources".into(),json!({"serial":self.serial.agent_inspect(&project)["source_title"].as_str().is_some_and(|title|!title.is_empty()),
            "waveform":self.waveform.agent_has_waveform(),"test_report":self.test_report.agent_has_report()}));
        let binding=Value::Object(binding).to_string();
        ContextState {
            page: self.ai.agent.origin_page().unwrap_or(self.active.as_id()).into(),
            project,
            binding,
            testing: self.serial.ai_controls_locked(self.test_tool.running_plugin_id().is_some()),
        }
    }
    pub(super) fn tick_engineering_agent(&mut self, ctx: &egui::Context) {
        // Workspace control requests also finish while another page is open.
        self.test_tool.agent_workspace_tick(ctx);
        self.tick_report_agent(ctx);
        self.tick_serial_broker(ctx);
        if self.ai.agent.busy() || self.ai.agent.tracking_test() {
            self.waveform.agent_tick(self.lang);
            self.test_tool.agent_sync_devices(self.instruments.live_devices_for_hub());self.test_tool.agent_tick(self.lang);
        }
        if self.ai.agent.tracking_test() {ctx.request_repaint_after(std::time::Duration::from_millis(100));self.ai.agent.update_test(self.test_tool.agent_run_status());}
        self.instruments.publish_ai_summary();self.test_tool.publish_ai_summary();
        if let Some(source)=self.ai.agent.navigation.take(){
            if source.project==self.test_tool.current_project_id(){self.active=match source.page.as_str(){"serial"=>MainTab::Serial,"waveform"=>MainTab::Waveform,"data_analysis"=>MainTab::DataAnalysis,"test_report"=>MainTab::TestReport,"test_tool"=>MainTab::TestTool,"calculator"=>MainTab::Calculator,_=>MainTab::Instruments};}
        }
        let context = self.agent_context();
        let calculator_source=if context.page=="calculator" || context.page=="data_analysis" && self.ai.agent.cross_tools {self.calculator.capture_ai_source()}else{None};
        let workspace_source=if context.page=="test_tool" {self.test_tool.capture_workspace_review_source()}else{None};
        let data_source=if context.page=="data_analysis" || context.page=="calculator" && self.ai.agent.cross_tools {self.data_analysis.capture_review_source()}else{None};
        let wave_source=if context.page=="waveform" {self.waveform.capture_review_wave()}else{None};
        self.ai.agent.set_calculator_review_source(calculator_source);
        self.ai.agent.set_workspace_review_source(workspace_source);
        self.ai.agent.set_data_review_source(data_source);
        if context.page!="data_analysis" {self.ai.agent.set_data_review_candidate(None);}
        else if self.ai.agent.wants_data_review_source() {self.ai.agent.set_data_review_candidate(self.data_analysis.capture_review_data());}
        self.ai.agent.set_wave_review_source(wave_source);
        if context.page!="waveform" {self.ai.agent.set_protocol_review_source(None);}
        else if self.ai.agent.wants_protocol_review_source() {
            self.ai.agent.set_protocol_review_source(self.waveform.capture_review_protocol());
        }
        self.ai.agent.set_context(context.clone());
        if self.ai.agent.wants_data_review_source() {
            let service=crate::ai::service(ctx);
            self.ai.agent.complete_data_capture(service.as_ref());
        }
        let Some(request) = self.ai.agent.take_request() else {
            return;
        };
        if !self.ai.agent.authorize(&request, &context) {
            self.ai.agent.attach(
                &request.id,
                HostReply::value(agent::failure("上下文或权限已变化，本次操作未执行")),
            );
            return;
        }
        let mut session = std::mem::take(&mut self.ai.agent);
        let response = self.agent_dispatch(ctx, &mut session, &request);
        self.ai.agent = session;
        self.instruments.publish_ai_summary();self.test_tool.publish_ai_summary();
        // A host-approved transition (start reception / configured decode) may change its own binding.
        self.ai.agent.host_transition(self.agent_context());
        self.ai.agent.attach(&request.id, response);
    }
    fn agent_dispatch(&mut self, ctx: &egui::Context, session: &mut agent::Session, r: &HostCall) -> HostReply {
        let method=r.call.method.as_str();
        let changes_source=(method.starts_with("instrument.") && !matches!(method,"instrument.list"|"instrument.result"|"instrument.readback"))
            || (method.starts_with("testing.") && !matches!(method,"testing.inspect"|"testing.status"|"testing.result"));
        let update=changes_source.then(||self.instruments.begin_source_update());
        let workspace_update=changes_source.then(||self.test_tool.begin_source_update());
        let result=self.agent_dispatch_inner(ctx,session,r);
        if update.is_some(){self.instruments.publish_ai_summary();self.test_tool.publish_ai_summary();}
        drop(workspace_update);drop(update);result
    }
    fn agent_dispatch_inner(&mut self, ctx: &egui::Context, session: &mut agent::Session, r: &HostCall) -> HostReply {
        let p = &r.call.params;
        let method = r.call.method.as_str();
        if method.starts_with("instrument.") && !matches!(method,"instrument.list"|"instrument.result"|"instrument.readback") {
            self.instruments.invalidate_agent_inventory();
        }
        if !r.internal {
            if let Err(error) = wiparse_core::engineering_agent::validate(&r.call) {
                return HostReply::value(agent::failure(error));
            }
        }
        if let Err(error)=session.dispatch_checkpoint(r) {return HostReply::value(agent::failure(error));}
        let project = self.test_tool.current_project_id().to_owned();
        let value = match method {
            "platform.catalog"=>wiparse_core::platform_api::catalog(p),
            "platform.invoke"=>return self.invoke_platform(ctx,r),
            "evidence.get"=>session.evidence(p["id"].as_str().unwrap_or("")),
            "page.inspect" => json!({"ok":true,"data":match r.context.page.as_str() {
                "serial"=>self.serial.ai_summary(),"waveform"=>self.waveform.ai_summary(),
                "data_analysis"=>self.data_analysis.ai_summary(),"test_report"=>self.test_report.ai_summary(),
                "test_tool"=>self.test_tool.published_ai_summary().clone(),"calculator"=>self.calculator.ai_summary(),_=>self.instruments.published_ai_summary().clone(),
            }}),
            "serial.monitor.configure" | "serial.monitor.start" | "serial.monitor.stop" => self.serial.agent_monitor(method,p,&project,self.lang,r.context.testing),
            "serial.inspect" => json!({"ok":true,"data":self.serial.agent_inspect(&project)}),
            "log.files" | "log.analyze" => return self.serial.agent_library(method,p.clone(),ctx,r.cancelled.clone()),
            "serial.logs" => {
                return match self.serial.agent_log_input() {
                    Ok((spec, memory)) => session.log_jobs.analyze(
                        ctx,
                        p.clone(),
                        spec,
                        memory,
                        r.cancelled.clone(),
                    ),
                    Err(e) => HostReply::value(agent::failure(e)),
                }
            }
            "serial.start" | "serial.stop" => match self.serial.agent_control(
                &project,
                method == "serial.start",
                self.lang,
                self.test_tool.running_plugin_id().is_some(),
            ) {
                Ok(value) => {
                    if method == "serial.start" && !value["connected"].as_bool().unwrap_or(false) {
                        wait("serial.ready", json!({}))
                    } else {
                        json!({"ok":true,"data":value})
                    }
                }
                Err(e) => agent::failure(e),
            },
            "serial.ready" if r.internal => {
                let state = self.serial.agent_inspect(&project);
                if state["connected"] == true {
                    json!({"ok":true,"data":state})
                } else if state["requested"] == true {
                    wait("serial.ready", json!({}))
                } else {
                    agent::failure("串口连接未建立，请检查连接状态")
                }
            }
            "wave.channels" => json!({"ok":true,"data":self.waveform.ai_summary()}),
            "wave.measure" | "wave.timing" => return session.reviewed_wave_measure(p,ctx,r.cancelled.clone()).unwrap_or_else(||self.waveform.agent_measure(p,ctx,r.cancelled.clone())),
            "wave.frames" => session.reviewed_protocol_frames(p).unwrap_or_else(||self.waveform.agent_frames(p)),
            "wave.decode" => self.waveform.agent_decode(p, self.lang),
            "wave.decode.result" if r.internal => self
                .waveform
                .agent_decode_status(p["generation"].as_u64().unwrap_or(u64::MAX)),
            "wave.view" => {
                let mut result = None;
                if p["fit"] == true {
                    result = Some(self.waveform.api_fit());
                }
                if p.get("channel").is_some() {
                    result = Some(self.waveform.api_select(&json!({"index":p["channel"]})));
                }
                if p.get("x1").is_some() || p.get("x2").is_some() {
                    result = Some(self.waveform.api_cursors(p));
                }
                match result {
                    Some(result) if !result.ok => reply(result),
                    _ => json!({"ok":true,"data":self.waveform.ai_summary()}),
                }
            }
            "data.present" => match self.data_analysis.file_dataset(p["query_id"].as_str().unwrap_or("")).map(Ok).unwrap_or_else(||self.serial.agent_dataset(p["query_id"].as_str().unwrap_or("")).map(std::sync::Arc::new)) {
                Ok(dataset)=>match self.data_analysis.agent_present(&dataset,p["query_id"].as_str().unwrap(),self.lang){Ok(mut v)=>{
                    wiparse_core::ai::workflow::include_expected_plot_fields(&mut v, p["expected_fields"].as_array().into_iter().flatten().filter_map(Value::as_str));
                    self.active=MainTab::DataAnalysis;
                    json!({"ok":true,"data":v})
                },Err(e)=>agent::failure(e)},Err(e)=>agent::failure(e)
            },
            "data.extract" => return self.data_analysis.agent_extract(p, ctx, r.cancelled.clone()),
            "data.inspect" => match self.data_analysis.published_ai_snapshot() {
                Some(snapshot) => {
                    let page = snapshot.page(p["offset"].as_u64().unwrap_or(0) as usize,p["limit"].as_u64().unwrap_or(32) as usize);
                    if p.get("snapshot_id").is_some_and(|id| id != &page["snapshot_id"]) {
                        json!({"ok":false,"error":"DATA.SNAPSHOT_CHANGED","message":"数据摘要已变化，请从第一页重新读取。"})
                    } else { json!({"ok":true,"data":page}) }
                }
                None => json!({"ok":false,"error":"DATA.SNAPSHOT_UNAVAILABLE"}),
            },
            "data.snapshot.read" | "data.snapshot.stats" => return session.reviewed_data_call(method,p,ctx,r.cancelled.clone()).unwrap_or_else(||HostReply::value(agent::failure("DATA.REVIEWED_TASK_REQUIRED"))),
            "data.statistics" => self
                .data_analysis
                .agent_statistics(&mut session.data_job, true),
            "data.statistics.result" if r.internal => self
                .data_analysis
                .agent_statistics(&mut session.data_job, false),
            "report.create" => return match session.report_evidence(p).and_then(|e|crate::ai::artifact::start(self.test_tool.data_root_path(),project.clone(),p.clone(),e,r.cancelled.clone(),ctx.clone(),|v|v)){Ok(rx)=>HostReply::work(rx),Err(e)=>HostReply::value(agent::failure(e))},
            "report.open" => {
                match crate::ai::artifact::report_path(&self.test_tool.data_root_path(),&project,p["report_id"].as_str().unwrap_or("")){
                    Ok(path) if path.is_file()=>{let _=self.test_report.select_project(&self.test_tool.data_root_path(),&[],&project);self.test_report.open_markdown(path,self.lang);self.active=MainTab::TestReport;json!({"ok":true,"data":{"opened":true,"report_id":p["report_id"]}})},
                    _=>agent::failure("REPORT.NOT_AVAILABLE_IN_CURRENT_PROJECT")
                }
            },
            "report.inspect" => return self.test_report.agent_index(p,&self.test_tool.data_root_path(),ctx,r.cancelled.clone()),
            "report.read" => return self.test_report.agent_read(p,&self.test_tool.data_root_path(),ctx,r.cancelled.clone()),
            "report.search" => return match self.test_report.agent_search_input(p,&self.test_tool.data_root_path()) {
                Ok((spec,query))=>session.report_jobs.analyze(ctx,query,spec,None,r.cancelled.clone()),
                Err(e)=>HostReply::value(agent::failure(e))
            },
            "calc.inspect" => json!({"ok":true,"data":self.calculator.ai_summary()}),
            "calc.evaluate" => CalculatorPanel::new().agent_calculate(p),
            "calc.apply" => self.calculator.agent_calculate(p),
            "testing.inspect" | "testing.status" => json!({"ok":true,"data":self.test_tool.agent_run_status()}),
            "testing.wait" => {
                let mut params=json!({"key":p["run_id"],"action":"testing.wait","deadline_ms":chrono::Utc::now().timestamp_millis()+p["timeout_seconds"].as_i64().unwrap_or(120)*1000});
                if self.test_tool.agent_job_key().is_none(){params["deadline_ms"]=json!(chrono::Utc::now().timestamp_millis()+1000);}
                wait("testing.result",params)
            },
            "testing.propose" => return self.test_tool.agent_propose(p,ctx,r.cancelled.clone()),
            "testing.apply" => self.test_tool.agent_apply_draft(p,&r.cancelled),
            "testing.select" => self.test_tool.agent_select_cases(p),
            "testing.pause" | "testing.resume" => {
                if !session.owns_test(self.test_tool.agent_job_key().as_deref()){agent::failure("不能操作其他来源发起的测试")}else{self.test_tool.agent_run_control(if method=="testing.pause"{"pause"}else{"resume"})}
            }
            "testing.cases" => self.test_tool.agent_cases(p),
            "testing.run" | "testing.preflight" => {
                self.test_tool.agent_sync_devices(self.instruments.live_devices_for_hub());
                if method == "testing.run" && session.selected_skill("test_execution") {
                    if let Err(error) = self.test_tool.agent_verified_preflight_gate() {
                        return HostReply::value(agent::failure(error));
                    }
                }
                let result = self.test_tool.agent_start(method == "testing.preflight");
                if result.ok {
                    let key = self.test_tool.agent_job_key();
                    session.mark_test(key.clone());
                    json!({"ok":true,"data":{"state":"accepted","run_id":key,"process_completed":false,"status":self.test_tool.agent_run_status(),"tracking":"host tracks completion independently; use testing.status for fresh status","formal_verdict":"not available until actual report"}})
                } else {
                    reply(result)
                }
            }
            "testing.stop" => {
                if !session
                    .owns_test(self.test_tool.agent_job_key().as_deref())
                {
                    agent::failure("不能停止其他来源发起的测试")
                } else {
                    let key = self.test_tool.agent_job_key();
                    let result = self.test_tool.api_stop();
                    if result.ok {
                        json!({"ok":true,"data":{"state":"accepted","run_id":key,"process_completed":false,"stop_requested":true,"device_safe_state":"unknown","status":self.test_tool.agent_run_status(),"tracking":"stop requested; use testing.wait with this run_id to observe process exit; device safe state requires separate feedback","formal_verdict":"not available until actual report"}})
                    } else {
                        reply(result)
                    }
                }
            }
            "testing.result" if r.internal => {
                let current = self.test_tool.agent_job_key();
                if current.is_none() {
                    session.mark_test(None);
                    {
                        let status = self.test_tool.agent_run_status();
                        if status["completed_run_identity"] != p["key"] || !status["process_exit_code"].is_i64() {
                            agent::failure("未取得匹配运行身份的完成记录，请核对测试报告")
                        } else {
                            json!({"ok":p["action"]=="testing.stop"||status["process_exit_code"]==0,"data":{"process_completed":true,"device_safe_state":"unknown",
                                "execution_succeeded":status["process_exit_code"]==0,
                                "formal_verdict":"consult actual test report; process completion is not a pass verdict","status":status}})
                        }
                    }
                } else if current.as_deref() == p["key"].as_str() {
                    if p["deadline_ms"].as_i64().is_some_and(|d|chrono::Utc::now().timestamp_millis()>=d){return HostReply::value(agent::failure("TEST.WAIT_TIMEOUT: run still active; not cancelled or retried"));}
                    wait("testing.result", p.clone())
                } else {
                    agent::failure("测试运行已被替换，停止等待旧运行")
                }
            }
            "instrument.readback"=>match self.instruments.agent_readback(&self.test_tool.data_root_path(),p){
                Ok(data)=>json!({"ok":true,"data":data}),Err(error)=>agent::failure(error)
            },
            "instrument.list" => {
                json!({"ok":true,"data":self.instruments.agent_inventory(self.lang,&json!({}),false)})
            }
            "instrument.waveform_source" => {
                let id = p["device_id"].as_u64().unwrap_or(0);
                match self.instruments.agent_validate_waveform_source(id) {
                    Err(error) => agent::failure(error),
                    Ok(()) => {
                        // The host binds the project directory by handle; the
                        // model cannot supply an output path or overwrite flag.
                        let root = self.test_tool.data_root_path();
                        let result = self.instruments.agent_waveform_source(&json!({"device_id":id}), r.cancelled.clone(), &root, &project);
                        if result.ok {
                            wait("instrument.result", json!({"job_id":result.data["job_id"],"job_session":result.data["job_session"],"device_id":result.data["device_id"]}))
                        } else { reply(result) }
                    }
                }
            }
            "instrument.command" | "instrument.capture" => {
                match SerialToolPanel::validate_agent_instrument(method, p).and_then(|v|self.instruments.agent_validate_device(p,method=="instrument.capture").map(|_|v)) {
                    Err(error) => agent::failure(error),
                    Ok(_) => {
                        let verification=if p.get("verify").is_some(){match self.instruments.independent_scope(p){
                            Ok(v)=>v,Err(e)=>return HostReply::value(agent::failure(e)),
                        }}else{Value::Null};
                        let inventory = self.instruments.agent_inventory(self.lang, &json!({}),false);
                        if !inventory["devices"].as_array().is_some_and(|devices| {
                            devices.iter().any(|d| {
                                d["id"] == p["device_id"] || d["device_id"] == p["device_id"]
                            })
                        }) {
                            agent::failure("所选设备不在当前已连接会话中")
                        } else {
                            let result = if method == "instrument.command" {
                                self.instruments.agent_command(&self.test_tool.data_root_path(),p)
                            } else {
                                self.instruments.agent_capture(p)
                            };
                            if result.ok {
                                wait("instrument.result", json!({"job_id":result.data["job_id"],"job_session":result.data["job_session"],"device_id":result.data["device_id"],"verification_plan":verification}))
                            } else {
                                reply(result)
                            }
                        }
                    }
                }
            }
            "instrument.result" if r.internal => {
                let result = self.instruments.agent_job_get(p);
                if result.ok && result.data["state"] == "pending_or_unknown" {
                    wait("instrument.result", p.clone())
                } else if result.ok && result.data["device_id"]!=p["device_id"] {
                    agent::failure("DEVICE.RECEIPT_IDENTITY_MISMATCH")
                } else if result.ok && !p["verification_plan"].is_null() && result.data["ok"]==true {
                    let plan=&p["verification_plan"];
                    let started=chrono::Utc::now().timestamp_millis();
                    let receipt=self.instruments.api_command(&plan["read"]);
                    if !receipt.ok {json!({"ok":false,"data":{"job":result.data,"command_completed":true,"measurement_error":receipt.error}})}
                    else{wait("instrument.result",json!({"job_id":receipt.data["job_id"],"job_session":receipt.data["job_session"],"device_id":plan["read"]["device_id"],"observation_plan":plan,"origin":result.data,"started":started}))}
                } else if result.ok && p["observation_plan"].is_object(){
                    let plan=&p["observation_plan"];
                    let end=chrono::Utc::now().timestamp_millis();
                    let measurement=scope_observation(plan,&result.data,p["started"].as_i64().unwrap_or(0),end);
                    let saved=InstrumentControlPanel::save_measurement(&self.test_tool.data_root_path(),plan,&p["origin"],&measurement);
                    json!({"ok":saved.is_ok() && measurement["value"].is_number(),"data":{"job":p["origin"],"command_completed":true,"physical_verified":false,"measurement":measurement,"archive_error":saved.err()}})
                } else if result.ok {
                    json!({"ok":result.data["ok"],"data":{"command_completed":true,"driver_ok":result.data["ok"],"physical_verified":false,"verification":"Device API job ended; a failed driver result may still leave physical state uncertain. Use an explicit readback and measurement.","job":result.data}})
                } else {
                    reply(result)
                }
            }
            _ => agent::failure("未实现或不允许的宿主调用"),
        };
        HostReply::value(value)
    }
}

/// Isolated application host for serial-agent integration tests. This runs the
/// production Broker and demo instrument worker; approval is injected only
/// after the Broker has created its concrete, bound confirmation request.
#[cfg(test)]
pub(crate) struct SerialBrokerTestHost {
    app: WiParseApp,
    pub approvals: usize,
    pub observed_waits: usize,
    pub last_action: Option<serde_json::Value>,
}
#[cfg(test)]
impl SerialBrokerTestHost {
    pub(crate) fn new(root: &std::path::Path, ctx: &egui::Context) -> Self {
        let mut cfg = wiparse_core::config::AppConfig::default();
        cfg.log_monitor.open_log_files.clear();
        cfg.log_monitor.save_live_to_disk = false;
        cfg.apps.test_tool.data_root = root.to_string_lossy().into_owned();
        let mut app = WiParseApp::new_without_api_for_test(cfg);
        let reply = app.instruments.api_connect(&json!({"resource":"DEMO::SCOPE","kind":"oscilloscope"}), app.lang);
        assert!(reply.ok);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.instruments.api_list(app.lang, &json!({}))["devices"].as_array().is_none_or(Vec::is_empty) {
            app.instruments.pump(ctx);
            assert!(std::time::Instant::now() < deadline, "demo scope did not connect");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("wiparse-canonical-agent"), app.serial_agent.tx.clone()));
        Self { app, approvals: 0, observed_waits: 0, last_action: None }
    }
    pub(crate) fn project(&self) -> String { self.app.test_tool.current_project_id() }
    pub(crate) fn select_report(&mut self, path: &std::path::Path) {
        self.app.test_report.test_select_report_without_persisting(path);
    }
    pub(crate) fn tick(&mut self, ctx: &egui::Context, serial: &mut SerialToolPanel) {
        std::mem::swap(&mut self.app.serial, serial);
        self.app.instruments.pump(ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| self.app.tick_serial_broker(ctx));
        if let Some(pending) = &mut self.app.serial_agent.pending {
            if pending.started { self.observed_waits += 1; }
            if !pending.started && pending.approval.is_none() {
                self.last_action = Some(json!({"method":pending.origin.method,"params":pending.origin.params}));
                pending.approval = Some(broker_approval_fingerprint(pending));
                self.approvals += 1;
            }
        }
        std::mem::swap(&mut self.app.serial, serial);
    }
}

/// The serial model keeps its conversation; execution state reuses the canonical host.
pub(super) struct SerialBroker {
    tx: crossbeam_channel::Sender<crate::ai::broker::Request>,
    rx: crossbeam_channel::Receiver<crate::ai::broker::Request>,
    session: agent::Session,
    pending: Option<BrokerPending>,
    test_owner: Option<(String, String)>,
}
struct BrokerPending {
    origin: crate::ai::broker::Request,
    request: HostCall,
    item: crate::ai::execution::HostItem,
    started: bool,
    target_page: String,
    target_binding: Value,
    test_binding: Value,
    test_run_binding: Option<String>,
    approval: Option<String>,
    review_focus_pending: bool,
}
impl Default for SerialBroker {
    fn default()->Self {let(tx,rx)=crossbeam_channel::bounded(8);Self{tx,rx,session:Default::default(),pending:None,test_owner:None}}
}
fn target_binding(context:&ContextState,page:&str)->Value {
    serde_json::from_str::<Value>(&context.binding).unwrap_or_default()[page].clone()
}
fn broker_approval_fingerprint(pending: &BrokerPending) -> String {
    crate::ai::broker::approval_fingerprint(&pending.origin.method,&pending.origin.params,
        pending.origin.guard.task_id.as_deref(),&pending.origin.guard.project,
        &pending.origin.guard.binding,&json!({"target":pending.target_binding,"test_workspace":pending.test_binding,"test_run":pending.test_run_binding}))
}

#[cfg(test)]
mod broker_integration_tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    #[test]
    fn broker_rejects_forged_project_source_and_control_before_review() {
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut host = SerialBrokerTestHost::new(root.path(), &ctx);
        let app = &mut host.app;
        let project = app.test_tool.current_project_id();
        for scenario in ["project", "source", "control"] {
            let mut guard = app.serial.test_broker_control_guard(&project);
            match scenario {
                "project" => guard.project = "forged-project".into(),
                "source" => guard.binding = "forged-source".into(),
                _ => guard.controls = false,
            }
            let (reply, rx) = crossbeam_channel::bounded(1);
            app.serial_agent.tx.send(crate::ai::broker::Request {
                method: "instrument.command".into(),
                params: json!({"device_id":1,"command":"ScopeStop"}),
                guard, reply, deadline: Instant::now() + Duration::from_secs(60),
            }).unwrap();
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.tick_serial_broker(ctx));
            let result = rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(!result.ok, "{scenario}: {result:?}");
            assert_eq!(result.data["executed"], false, "{scenario}: {result:?}");
            assert!(app.serial_agent.pending.is_none(), "{scenario}: confirmation must not be offered");
            assert!(rx.try_recv().is_err(), "{scenario}: duplicate result");
        }
    }

    #[test]
    fn demo_session_replacement_invalidates_pending_instrument_approval() {
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut host = SerialBrokerTestHost::new(root.path(), &ctx);
        let app = &mut host.app;
        let project = app.test_tool.current_project_id();
        let guard = app.serial.test_broker_control_guard(&project);
        let (reply, rx) = crossbeam_channel::bounded(1);
        let params = json!({"device_id": 1, "command": "ScopeStop"});
        let origin = crate::ai::broker::Request { method: "instrument.command".into(), params: params.clone(), guard, reply, deadline: Instant::now() + Duration::from_secs(60) };
        let mut context = app.agent_context();
        context.page = "serial".into();
        let page = wiparse_core::engineering_agent::validate(&wiparse_core::engineering_agent::Call { method: origin.method.clone(), params: params.clone() }).unwrap().page.to_owned();
        let request = HostCall { id: uuid::Uuid::new_v4().to_string(), call: wiparse_core::engineering_agent::Call { method: origin.method.clone(), params }, context: context.clone(), cancelled: origin.guard.cancelled.clone(), internal: false };
        let mut pending = BrokerPending { origin, request, item: crate::ai::execution::HostItem::default(), started: false, target_binding: target_binding(&context, &page), test_binding:target_binding(&context,"test_tool"), test_run_binding:None, target_page: page, approval: None, review_focus_pending: true };
        pending.approval = Some(broker_approval_fingerprint(&pending));
        app.serial_agent.pending = Some(pending);

        assert!(app.instruments.api_disconnect(&json!({"device_id":1})).ok);
        assert!(app.instruments.api_connect(&json!({"resource":"DEMO::SCOPE","kind":"oscilloscope"}), app.lang).ok);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.instruments.pump(&ctx);
            let devices = app.instruments.api_list(app.lang, &json!({}));
            let devices = devices["devices"].as_array().unwrap();
            if devices.iter().any(|device| device["device_id"] == 2) && !devices.iter().any(|device| device["device_id"] == 1) { break; }
            assert!(Instant::now() < deadline, "demo session replacement did not complete");
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.tick_serial_broker(ctx));
        let result = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!result.ok);
        assert_eq!(result.data["executed"], false);
        assert!(result.error.unwrap().contains("AI.CANCELLED_OR_STALE"));
        assert!(app.serial_agent.pending.is_none());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn expired_broker_request_does_not_block_next_demo_action() {
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut host = SerialBrokerTestHost::new(root.path(), &ctx);
        let app = &mut host.app;
        let project = app.test_tool.current_project_id();
        let mut receivers = Vec::new();
        let base_guard = app.serial.test_broker_control_guard(&project);
        for expired in [true, false] {
            let (reply, rx) = crossbeam_channel::bounded(1);
            let mut guard = base_guard.clone();
            guard.cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            app.serial_agent.tx.send(crate::ai::broker::Request {
                method: "instrument.command".into(), params: json!({"device_id":1,"command":"ScopeStop"}),
                guard, reply, deadline: if expired { Instant::now() - Duration::from_secs(1) } else { Instant::now() + Duration::from_secs(60) },
            }).unwrap();
            receivers.push(rx);
        }
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.tick_serial_broker(ctx));
        let expired = receivers[0].recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!expired.ok);
        assert_eq!(expired.data["executed"], false);
        assert!(expired.error.as_deref().unwrap_or("").contains("AI.TOOL_TIMEOUT"), "unexpected expiry result: {expired:?}");
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.tick_serial_broker(ctx));
        let pending = app.serial_agent.pending.as_mut().expect("next action requires its own confirmation");
        assert!(!pending.started);
        assert!(pending.approval.is_none());
        assert!(receivers[1].try_recv().is_err());
        pending.approval = Some(broker_approval_fingerprint(pending));
        let deadline = Instant::now() + Duration::from_secs(5);
        let result = loop {
            app.instruments.pump(&ctx);
            let _ = ctx.run(egui::RawInput::default(), |ctx| app.tick_serial_broker(ctx));
            if let Ok(result) = receivers[1].try_recv() { break result; }
            assert!(Instant::now() < deadline, "demo action did not complete");
            std::thread::sleep(Duration::from_millis(2));
        };
        assert!(result.ok, "demo action failed: {result:?}");
        assert!(receivers[0].try_recv().is_err());
        assert!(receivers[1].try_recv().is_err());
    }

    #[test]
    fn broker_rejects_expired_cancelled_and_changed_approvals_before_dispatch() {
        let root=tempfile::tempdir().unwrap();let mut cfg=wiparse_core::config::AppConfig::default();
        cfg.log_monitor.open_log_files.clear();cfg.log_monitor.save_live_to_disk=false;
        cfg.apps.test_tool.data_root=root.path().to_string_lossy().into_owned();
        let mut app=WiParseApp::new_without_api_for_test(cfg);let ctx=egui::Context::default();
        for scenario in ["expired","cancelled","changed","run_changed"] {
            let project=app.test_tool.current_project_id();
            let guard=app.serial.test_broker_control_guard(&project);
            let (reply,rx)=crossbeam_channel::bounded(1);
            let params=json!({"device_id":999,"command":"ScopeStop"});
            let origin=crate::ai::broker::Request{method:"instrument.command".into(),params:params.clone(),guard,reply,
                deadline:if scenario=="expired"{Instant::now()-Duration::from_secs(1)}else{Instant::now()+Duration::from_secs(60)}};
            let mut context=app.agent_context();context.page="serial".into();
            let page=wiparse_core::engineering_agent::validate(&wiparse_core::engineering_agent::Call{method:origin.method.clone(),params:params.clone()}).unwrap().page.to_owned();
            let request=HostCall{id:uuid::Uuid::new_v4().to_string(),call:wiparse_core::engineering_agent::Call{method:origin.method.clone(),params},context:context.clone(),cancelled:origin.guard.cancelled.clone(),internal:false};
            let mut pending=BrokerPending{origin,request,item:crate::ai::execution::HostItem::default(),started:false,
                target_binding:target_binding(&context,&page),test_binding:target_binding(&context,"test_tool"),test_run_binding:None,target_page:page,approval:None,review_focus_pending:true};
            pending.approval=Some(broker_approval_fingerprint(&pending));
            if scenario=="cancelled"{crate::ai::agent::cancel_host_call(&pending.origin.guard.cancelled);}
            if scenario=="changed"{pending.origin.params["device_id"]=json!(1000);}
            if scenario=="run_changed"{pending.test_run_binding=Some("replacement-run".into());}
            app.serial_agent.pending=Some(pending);
            let _=ctx.run(egui::RawInput::default(),|ctx|app.tick_serial_broker(ctx));
            let result=rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(!result.ok,"{scenario}: {result:?}");
            let error=result.error.unwrap();
            assert!(error.contains(match scenario {"expired"=>"AI.TOOL_TIMEOUT","cancelled"=>"AI.CANCELLED_OR_STALE",_=>"AI.APPROVAL_STALE"}),"{scenario}: {error}");
            assert!(app.serial_agent.pending.is_none());
            let _=ctx.run(egui::RawInput::default(),|ctx|app.tick_serial_broker(ctx));
            assert!(rx.try_recv().is_err(),"{scenario}: repeated response");
        }
    }

    #[test]
    fn concurrent_expired_broker_requests_each_complete_once_without_dispatch() {
        use std::sync::{Arc, Barrier, atomic::AtomicBool};
        let root=tempfile::tempdir().unwrap();let mut cfg=wiparse_core::config::AppConfig::default();
        cfg.log_monitor.open_log_files.clear();cfg.log_monitor.save_live_to_disk=false;
        cfg.apps.test_tool.data_root=root.path().to_string_lossy().into_owned();
        let mut app=WiParseApp::new_without_api_for_test(cfg);let ctx=egui::Context::default();
        let project=app.test_tool.current_project_id();let guard=app.serial.test_broker_control_guard(&project);
        let barrier=Arc::new(Barrier::new(5));let mut workers=Vec::new();
        for _ in 0..4 {
            let tx=app.serial_agent.tx.clone();let barrier=barrier.clone();let mut guard=guard.clone();
            guard.cancelled=Arc::new(AtomicBool::new(false));
            workers.push(std::thread::spawn(move||{
                let (reply,rx)=crossbeam_channel::bounded(1);barrier.wait();
                tx.send(crate::ai::broker::Request{method:"instrument.command".into(),params:json!({"device_id":999,"command":"ScopeStop"}),
                    guard,reply,deadline:Instant::now()-Duration::from_secs(1)}).unwrap();rx
            }));
        }
        barrier.wait();let receivers:Vec<_>=workers.into_iter().map(|w|w.join().unwrap()).collect();
        for _ in 0..6 {let _=ctx.run(egui::RawInput::default(),|ctx|app.tick_serial_broker(ctx));}
        for rx in receivers {let result=rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(!result.ok);assert!(result.error.unwrap().contains("AI.TOOL_TIMEOUT"));assert!(rx.try_recv().is_err());}
        assert!(app.serial_agent.pending.is_none());assert!(app.serial_agent.rx.is_empty());
    }
}
impl WiParseApp {
    fn tick_serial_broker(&mut self,ctx:&egui::Context) {
        use std::sync::atomic::Ordering;
        use std::time::{Duration,Instant};
        ctx.data_mut(|d|d.insert_temp(egui::Id::new("wiparse-canonical-agent"),self.serial_agent.tx.clone()));
        let mut broker=std::mem::take(&mut self.serial_agent);
        if broker.session.tracking_test() {
            self.test_tool.agent_sync_devices(self.instruments.live_devices_for_hub());self.test_tool.agent_tick(self.lang);
            broker.session.update_test(self.test_tool.agent_run_status());
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if broker.pending.is_none() {
            if let Ok(origin)=broker.rx.try_recv() {
                let mut context=self.agent_context();context.page="serial".into();
                let page=wiparse_core::engineering_agent::validate(&wiparse_core::engineering_agent::Call{method:origin.method.clone(),params:origin.params.clone()}).map_or("any",|t|t.page).to_owned();
                let mut request=HostCall{id:uuid::Uuid::new_v4().to_string(),call:wiparse_core::engineering_agent::Call{method:origin.method.clone(),params:origin.params.clone()},context:context.clone(),cancelled:origin.guard.cancelled.clone(),internal:false};
                if origin.guard.skill_ids.iter().any(|id|id=="test_result") && request.call.method=="report.read"
                    && request.call.params["scan"].is_null() && request.call.params["limit"].is_null()
                    && !crate::ai::execution::structured_adapter_request(&request.call) {
                    request.call.params["limit"]=json!(8);
                }
                broker.session.set_context(context.clone());
                let mut item = crate::ai::execution::HostItem::default();
                if origin.guard.skill_ids.iter().any(|id| id == "qi_protocol") { item.collect_frames(&request.call); }
                if origin.guard.skill_ids.iter().any(|id| id == "test_result") { item.collect_report(&request.call); }
                if origin.guard.skill_ids.iter().any(|id| id == "serial_log" || id == "charging_protection") { item.collect_log(&request.call); }
                let test_run_binding = if matches!(origin.method.as_str(),"testing.stop"|"testing.pause"|"testing.resume") { self.test_tool.agent_job_key() } else { None };
                broker.pending=Some(BrokerPending{origin,request,item,started:false,target_binding:target_binding(&context,&page),test_binding:target_binding(&context,"test_tool"),test_run_binding,target_page:page,approval:None,review_focus_pending:true});
            }
        }
        if let Some(mut pending)=broker.pending.take() {
            let now=Instant::now();
            let context=self.agent_context();
            let validated=wiparse_core::engineering_agent::validate(&wiparse_core::engineering_agent::Call{method:pending.origin.method.clone(),params:pending.origin.params.clone()});
            let permission=validated.as_ref().is_ok_and(|t|!matches!(t.effect,wiparse_core::engineering_agent::Effect::Device|wiparse_core::engineering_agent::Effect::Test)||pending.origin.guard.controls);
            let stale=context.project!=pending.origin.guard.project || !(if pending.started {self.serial.ai_platform_result_guard_valid(&pending.origin.guard,&pending.origin.method)}else{self.serial.ai_platform_guard_for(&pending.origin.guard,&pending.origin.method,&pending.origin.params)})
                || target_binding(&context,&pending.target_page)!=pending.target_binding
                || (!pending.started && validated.as_ref().is_ok_and(|t|matches!(t.effect,wiparse_core::engineering_agent::Effect::Device|wiparse_core::engineering_agent::Effect::Test))
                    && target_binding(&context,"test_tool")!=pending.test_binding);
            let mut result=if pending.request.cancelled.load(Ordering::Acquire)||stale {
                    Some(if pending.started {json!({"ok":false,"error":"AI.CANCELLED_OR_STALE","execution_state":"unknown; inspect device before retry"})}
                         else {json!({"ok":false,"executed":false,"error":"AI.CANCELLED_OR_STALE"})})
                }
                else if !permission {Some(if pending.started {json!({"ok":false,"error":"AI.CONTROL_LOCKED_OR_INVALID_TOOL","execution_state":"unknown; inspect device before retry"})}
                    else {json!({"ok":false,"executed":false,"error":"AI.CONTROL_LOCKED_OR_INVALID_TOOL"})})}
                else if now>=pending.origin.deadline {
                    Some(if pending.started {json!({"ok":false,"error":"AI.TOOL_TIMEOUT","execution_state":"unknown; inspect device before retry"})}
                         else {json!({"ok":false,"executed":false,"error":"AI.TOOL_TIMEOUT: approval expired; action not dispatched"})})
                }
                else {None};
            if result.is_some() {
                if let Some(partial) = pending.item.partial_result("SKILL.COLLECTION_CANCELLED_OR_STALE") { result = Some(partial); }
            }
            let review=validated.as_ref().is_ok_and(|t|matches!(t.effect,wiparse_core::engineering_agent::Effect::Device|wiparse_core::engineering_agent::Effect::Test));
            if result.is_none() && review && pending.origin.guard.task_id.is_none() {
                result=Some(json!({"ok":false,"executed":false,"error":"TEST.TASK_ID_REQUIRED"}));
            }
            if result.is_none() && pending.origin.method == "platform.invoke"
                && matches!(pending.origin.params["method"].as_str(), Some("scope.wave" | "scope.shot")) {
                result=Some(json!({"ok":false,"executed":false,"error":"DEVICE.UNBOUND_SCOPE_INDEX: use the connected instrument session and instrument.waveform_source or instrument.capture"}));
            }
            if result.is_none() && pending.origin.method == "platform.invoke"
                && matches!(pending.origin.params["method"].as_str(), Some("instrument.capture" | "instrument.waveform_source")) {
                result=Some(json!({"ok":false,"executed":false,"error":"DEVICE.USE_CANONICAL_CAPTURE: use the typed instrument.capture or instrument.waveform_source tool"}));
            }
            if result.is_none() && pending.origin.method == "platform.invoke"
                && matches!(pending.origin.params["method"].as_str(), Some("serial.monitor.start" | "serial.monitor.stop")) {
                result=Some(json!({"ok":false,"executed":false,"error":"SERIAL.USE_CANONICAL_MONITOR: use the typed serial monitor tool for source-bound confirmation"}));
            }
            if result.is_none() && !pending.started && pending.approval.is_none() && pending.origin.method == "platform.invoke"
                && pending.origin.params["method"] == "instrument.command" {
                let params = &pending.origin.params["params"];
                let validation = if params.get("verify").is_some() { Err("DEVICE.USE_CANONICAL_COMMAND_FOR_VERIFICATION".into()) }
                    else { SerialToolPanel::validate_agent_instrument("instrument.command", params)
                        .and_then(|_| self.instruments.agent_validate_device(params, false)) };
                if let Err(error) = validation { result=Some(json!({"ok":false,"executed":false,"error":error})); }
            }
            if result.is_none() && !pending.started && pending.approval.is_none() && pending.origin.method == "instrument.waveform_source" {
                let target = pending.origin.params["device_id"].as_u64().unwrap_or(0);
                if let Err(error) = self.instruments.agent_validate_waveform_source(target) {
                    result=Some(json!({"ok":false,"executed":false,"error":error}));
                }
            }
            if result.is_none() && !pending.started && pending.approval.is_none() && matches!(pending.origin.method.as_str(), "instrument.command" | "instrument.capture") {
                let method = pending.origin.method.as_str();
                let validation = SerialToolPanel::validate_agent_instrument(method, &pending.origin.params)
                    .and_then(|_| self.instruments.agent_validate_device(&pending.origin.params, method == "instrument.capture"));
                if let Err(error) = validation { result=Some(json!({"ok":false,"executed":false,"error":error})); }
            }
            if result.is_none() && !pending.started && matches!(pending.origin.method.as_str(),"testing.stop"|"testing.pause"|"testing.resume") {
                let run=self.test_tool.agent_job_key();
                if pending.test_run_binding != run || !test_owner_matches(broker.test_owner.as_ref(),pending.origin.guard.task_id.as_deref(),run.as_deref()) {
                    result=Some(json!({"ok":false,"executed":false,"error":"TEST.NOT_TASK_OWNER"}));
                }
            }
            let mut declined=false;
            if result.is_none()&&review&&!pending.started&&pending.approval.is_none() {
                let zh = matches!(self.lang, wiparse_core::i18n::Lang::Zh);
                egui::Window::new(if zh {"AI 平台操作确认"} else {"Confirm AI platform action"}).id(egui::Id::new("platform-action-review")).collapsible(false).resizable(true).show(ctx,|ui|{
                    ui.label(if zh {"请核对目标与参数；确认后执行这一次操作。"} else {"Review the target and parameters; confirmation permits this action once."});
                    ui.label(format!("{}: {} · {}: {} · {}: {}", if zh {"操作"} else {"Action"}, pending.origin.method, if zh {"任务"} else {"Task"}, pending.origin.guard.task_id.as_deref().unwrap_or(""), if zh {"项目"} else {"Project"}, pending.origin.guard.project));
                    if pending.origin.method == "instrument.waveform_source" {
                        ui.label(if zh {"采集完成后将以新文件保存到当前工程的 AI 证据目录。"} else {"After acquisition, a new file is saved in the current project's AI evidence directory."});
                    }
                    if pending.target_page == "instruments" {
                        let params = if pending.origin.method == "platform.invoke" { &pending.origin.params["params"] } else { &pending.origin.params };
                        let target = params["device_id"].as_u64().and_then(|id| self.instruments.agent_review_target(id))
                            .unwrap_or_else(|| "未能解析已连接设备，请核对参数 / Connected device not resolved; inspect parameters".into());
                        ui.label(format!("{}: {target}", if zh {"设备"} else {"Device"}));
                    }
                    if pending.origin.method.starts_with("testing.") {
                        let status = self.test_tool.agent_run_status();
                        ui.label(format!("{}: {}", if zh {"插件"} else {"Plugin"}, status["plugin"].as_str().unwrap_or(if zh {"未选择"} else {"not selected"})));
                        if let Some(run) = &pending.test_run_binding { ui.label(format!("{}: {run}", if zh {"运行编号"} else {"Run ID"})); }
                    }
                    egui::ScrollArea::vertical().max_height(320.).show(ui,|ui|{ui.monospace(serde_json::to_string_pretty(&pending.origin.params).unwrap_or_default());});
                    ui.horizontal(|ui|{
                        if ui.button(if zh {"确认执行"} else {"Confirm action"}).clicked(){pending.approval=Some(broker_approval_fingerprint(&pending));}
                        let reject = ui.button(if zh {"取消此操作"} else {"Cancel action"});
                        if pending.review_focus_pending { reject.request_focus(); pending.review_focus_pending=false; }
                        if reject.clicked(){declined=true;}
                    });
                });
            }
            if declined{result=Some(json!({"ok":false,"executed":false,"error":"PLATFORM.USER_DECLINED"}));}
            if result.is_none() && review && !pending.started && pending.approval.as_ref().is_some_and(|approval| approval != &broker_approval_fingerprint(&pending)) {
                result=Some(json!({"ok":false,"executed":false,"error":"AI.APPROVAL_STALE"}));
            }
            if result.is_none() && (!review||pending.started||pending.approval.is_some()) {
                let continuation=pending.item.take_due(now);
                let dispatch=!pending.started || continuation.is_some();
                if let Some(call)=continuation {
                    pending.request.call=call;
                    pending.request.internal=true;
                }
                let received=if dispatch {
                    if !pending.started { pending.approval = None; }
                    if !pending.started && matches!(pending.origin.method.as_str(), "testing.preflight" | "testing.run") {
                        self.test_tool.agent_sync_devices(self.instruments.live_devices_for_hub());
                    }
                    if !pending.started && pending.origin.method == "testing.run" {
                        if let Err(error) = self.test_tool.agent_verified_preflight_gate() {
                            let _=pending.origin.reply.try_send(crate::backend::InvokeReply{ok:false,cmd:pending.origin.method.clone(),error:Some(error),data:json!({"executed":false})});
                            self.serial_agent=broker;
                            return;
                        }
                    }
                    pending.started=true;
                    let no_previous_job = self.test_tool.agent_job_key().is_none();
                    let response=self.agent_dispatch(ctx,&mut broker.session,&pending.request);
                    if no_previous_job && response.value["ok"] == true && matches!(pending.origin.method.as_str(), "testing.run" | "testing.preflight") {
                        if let (Some(task), Some(run)) = (pending.origin.guard.task_id.clone(), self.test_tool.agent_job_key()) {
                            broker.test_owner = Some((task,run));
                        }
                    }
                    if !pending.request.internal && pending.origin.method=="platform.invoke" && matches!(pending.origin.params["method"].as_str(),Some("ui.serial.open"|"ui.serial.close"|"ui.serial.clear"|"ui.serial.tab"|"serial.select"|"serial.monitor.start"|"serial.monitor.stop")) {
                        self.serial.ai_accept_source_transition(&mut pending.origin.guard);
                    }
                    self.instruments.publish_ai_summary();self.test_tool.publish_ai_summary();
                    pending.target_binding=target_binding(&self.agent_context(),&pending.target_page);
                    pending.item.attach(response)
                } else { pending.item.receive() };
                if let Some(value)=received {
                    result=pending.item.advance(value,&pending.request.call,pending.request.internal);
                }
            }
            if let Some(value)=result {
                if pending.started && matches!(value["error"].as_str(), Some("AI.CANCELLED_OR_STALE" | "AI.TOOL_TIMEOUT")) {
                    crate::ai::agent::cancel_host_call(&pending.request.cancelled);
                }
                let mut data = value["data"].clone();
                if value["executed"] == false {
                    if !data.is_object() { data = json!({}); }
                    data["executed"] = json!(false);
                }
                let _=pending.origin.reply.try_send(crate::backend::InvokeReply{ok:value["ok"]!=false,cmd:pending.origin.method.clone(),error:value["error"].as_str().map(str::to_owned),data});
            }else{broker.pending=Some(pending);}
        }
        if broker.pending.is_some()||!broker.rx.is_empty(){self.waveform.agent_tick(self.lang);self.test_tool.agent_sync_devices(self.instruments.live_devices_for_hub());self.test_tool.agent_tick(self.lang);ctx.request_repaint_after(Duration::from_millis(50));}
        ctx.data_mut(|data|data.insert_temp(egui::Id::new("wiparse-serial-test-owner"),broker.test_owner.clone()));
        self.serial_agent=broker;
    }
}

impl WiParseApp {
 fn invoke_platform(&mut self,ctx:&egui::Context,r:&HostCall)->HostReply {
  use std::sync::atomic::Ordering;
  if r.cancelled.load(Ordering::Acquire){return HostReply::value(agent::failure("PLATFORM.CANCELLED"));}
  if let Err(e)=wiparse_core::engineering_agent::validate(&r.call){return HostReply::value(agent::failure(e));}
  let method=r.call.params["method"].as_str().unwrap().to_owned();let params=r.call.params["params"].clone();
  if matches!(method.as_str(), "scope.wave" | "scope.shot") {
      return HostReply::value(json!({"ok":false,"executed":false,"error":"DEVICE.UNBOUND_SCOPE_INDEX: use the connected instrument session and instrument.waveform_source or instrument.capture"}));
  }
  if matches!(method.as_str(), "instrument.capture" | "instrument.waveform_source") {
      return HostReply::value(json!({"ok":false,"executed":false,"error":"DEVICE.USE_CANONICAL_CAPTURE: use the typed instrument.capture or instrument.waveform_source tool"}));
  }
  if matches!(method.as_str(), "serial.monitor.start" | "serial.monitor.stop") {
      return HostReply::value(json!({"ok":false,"executed":false,"error":"SERIAL.USE_CANONICAL_MONITOR: use the typed serial monitor tool for source-bound confirmation"}));
  }
  if method=="instrument.command" {
      if params.get("verify").is_some() {
          return HostReply::value(json!({"ok":false,"executed":false,"error":"DEVICE.USE_CANONICAL_COMMAND_FOR_VERIFICATION"}));
      }
      if let Err(error) = SerialToolPanel::validate_agent_instrument("instrument.command", &params)
          .and_then(|_| self.instruments.agent_validate_device(&params, false)) {
          return HostReply::value(json!({"ok":false,"executed":false,"error":error}));
      }
      let result=self.instruments.agent_command(&self.test_tool.data_root_path(),&params);
      return HostReply::value(if result.ok {
          wait("instrument.result",result.data)
      } else { reply(result) });
  }
  let(tx,rx)=crossbeam_channel::bounded(1);
  if wiparse_core::platform_api::is_stateful(&method){
   let request=crate::backend::PendingRequest{method:method.clone(),params,reply:tx,deadline:std::time::Instant::now()+std::time::Duration::from_secs(90),ai_guard:None};
   let persist=crate::backend::stateful::handle(&self.api,&mut backend::UiHost{
    cfg:&mut self.cfg,lang:&mut self.lang,active:&mut self.active,
    show_serial:&mut self.show_serial,show_calculator:&mut self.show_calculator,show_instruments:&mut self.show_instruments,show_waveform:&mut self.show_waveform,show_data_analysis:&mut self.show_data_analysis,show_test_report:&mut self.show_test_report,show_test_tool:&mut self.show_test_tool,
    serial:&mut self.serial,instruments:&mut self.instruments,calculator:&mut self.calculator,waveform:&mut self.waveform,data_analysis:&mut self.data_analysis,test_report:&mut self.test_report,test_tool:&mut self.test_tool},request);
   if persist{self.persist_ui_prefs();self.ensure_visible_tab();}
  }else{
   let bridge=self.api.clone();let method=method.clone();let cancel=r.cancelled.clone();
   std::thread::spawn(move||{let result=if cancel.load(Ordering::Acquire){crate::backend::invoke_err(&method,"PLATFORM.CANCELLED")}else{crate::backend::dispatch::invoke(&bridge,&method,params)};let _=tx.send(result);});
  }
  let(out_tx,out_rx)=crossbeam_channel::bounded(1);let ctx=ctx.clone();
  std::thread::spawn(move||{
   let value=match rx.recv_timeout(std::time::Duration::from_secs(90)){
    Ok(result)=>{
     if result.ok&&method.starts_with("instrument.")&&result.data["job_id"].as_u64().is_some(){wait("instrument.result",json!({"job_id":result.data["job_id"],"job_session":result.data["job_session"],"device_id":result.data["device_id"]}))}
     else {json!({"ok":result.ok,"data":wiparse_core::platform_api::redact(result.data),"error":result.error,"method":method,"completion_note":"API handler result; inspect loading/running/job state where present. Acceptance is not physical verification."})}
    },Err(_)=>agent::failure("PLATFORM.RESULT_UNKNOWN: do not automatically repeat state-changing actions")
   };let _=out_tx.send(value);ctx.request_repaint();
  });HostReply::work(out_rx)
 }
}

impl WiParseApp {
    fn tick_report_agent(&mut self,ctx:&egui::Context) {
        if !(self.report_ai.open && self.active==MainTab::TestReport) && !self.report_ai.session.busy() && !self.report_ai.session.tracking_test() {return;}
        let Some(service)=crate::ai::service(ctx) else {return;};
        let mut session=std::mem::take(&mut self.report_ai.session);
        let mut context=self.agent_context(); context.page="test_report".into();
        session.configure(&self.cfg.ai);
        session.set_report_context(context.clone());
        session.poll(ctx,&service);
        if session.tracking_test() {self.test_tool.agent_sync_devices(self.instruments.live_devices_for_hub());self.test_tool.agent_tick(self.lang);session.update_test(self.test_tool.agent_run_status());}
        if let Some(source)=session.navigation.take(){if source.project==context.project{self.active=MainTab::TestReport;self.show_test_report=true;}}
        if let Some(request)=session.take_request() {
            let response=if session.authorize(&request,&context){self.agent_dispatch(ctx,&mut session,&request)}else{HostReply::value(agent::failure("报告来源或权限已变化，本次操作未执行"))};
            self.instruments.publish_ai_summary();self.test_tool.publish_ai_summary();
            let mut updated=self.agent_context();updated.page="test_report".into();
            session.host_transition(updated);session.attach(&request.id,response);
        }
        if session.busy()||session.tracking_test(){ctx.request_repaint_after(std::time::Duration::from_millis(100));}
        self.report_ai.session=session;
    }
}

#[cfg(test)]
mod test_owner_guard_tests {
    use super::test_owner_matches;
    #[test]
    fn test_control_requires_exact_task_and_current_run() {
        let owner=Some(("task-a".to_owned(),"run-a".to_owned()));
        assert!(test_owner_matches(owner.as_ref(),Some("task-a"),Some("run-a")));
        assert!(!test_owner_matches(owner.as_ref(),Some("task-b"),Some("run-a")));
        assert!(!test_owner_matches(owner.as_ref(),Some("task-a"),Some("run-b")));
        assert!(!test_owner_matches(owner.as_ref(),None,Some("run-a")));
        assert!(!test_owner_matches(owner.as_ref(),Some("task-a"),None));
    }
}
