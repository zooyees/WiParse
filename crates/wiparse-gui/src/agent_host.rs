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
                _ => self.instruments.agent_identity(),
            };
            (page.to_string(),Value::String(identity))
        })
        .collect::<serde_json::Map<_,_>>();
        let binding=Value::Object(binding).to_string();
        ContextState {
            page: self.ai.agent.origin_page().unwrap_or(self.active.as_id()).into(),
            project,
            binding,
            testing: self.serial.ai_controls_locked(self.test_tool.running_plugin_id().is_some()),
        }
    }
    pub(super) fn tick_engineering_agent(&mut self, ctx: &egui::Context) {
        self.tick_report_agent(ctx);
        self.tick_serial_broker(ctx);
        if self.ai.agent.busy() || self.ai.agent.tracking_test() {
            self.waveform.agent_tick(self.lang);
            self.test_tool.agent_tick(self.lang);
        }
        if self.ai.agent.tracking_test() {ctx.request_repaint_after(std::time::Duration::from_millis(100));self.ai.agent.update_test(self.test_tool.agent_run_status());}
        if let Some(source)=self.ai.agent.navigation.take(){
            if source.project==self.test_tool.current_project_id(){self.active=match source.page.as_str(){"serial"=>MainTab::Serial,"waveform"=>MainTab::Waveform,"data_analysis"=>MainTab::DataAnalysis,"test_report"=>MainTab::TestReport,"test_tool"=>MainTab::TestTool,"calculator"=>MainTab::Calculator,_=>MainTab::Instruments};}
        }
        let context = self.agent_context();
        self.ai.agent.set_context(context.clone());
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
        // A host-approved transition (start reception / configured decode) may change its own binding.
        self.ai.agent.host_transition(self.agent_context());
        self.ai.agent.attach(&request.id, response);
    }
    fn agent_dispatch(&mut self, ctx: &egui::Context, session: &mut agent::Session, r: &HostCall) -> HostReply {
        let p = &r.call.params;
        let method = r.call.method.as_str();
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
                "test_tool"=>self.test_tool.ai_summary(),"calculator"=>self.calculator.ai_summary(),_=>self.instruments.ai_summary(),
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
            "wave.measure" | "wave.timing" => return self.waveform.agent_measure(p, ctx, r.cancelled.clone()),
            "wave.frames" => self.waveform.agent_frames(p),
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
                Ok(dataset)=>match self.data_analysis.agent_present(&dataset,p["query_id"].as_str().unwrap(),self.lang){Ok(v)=>{self.active=MainTab::DataAnalysis;json!({"ok":true,"data":v})},Err(e)=>agent::failure(e)},Err(e)=>agent::failure(e)
            },
            "data.extract" => return self.data_analysis.agent_extract(p, ctx, r.cancelled.clone()),
            "data.inspect" => json!({"ok":true,"data":self.data_analysis.ai_summary()}),
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
            "report.inspect" => self.test_report.agent_index(p),
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
            "testing.propose" => return self.test_tool.agent_propose(p,ctx),
            "testing.apply" => self.test_tool.agent_apply_draft(p),
            "testing.select" => self.test_tool.agent_select_cases(p),
            "testing.pause" | "testing.resume" => {
                if !session.owns_test(self.test_tool.agent_job_key().as_deref()){agent::failure("不能操作其他来源发起的测试")}else{self.test_tool.agent_run_control(if method=="testing.pause"{"pause"}else{"resume"})}
            }
            "testing.cases" => self.test_tool.agent_cases(p),
            "testing.run" | "testing.preflight" => {
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
                        json!({"ok":true,"data":{"state":"accepted","run_id":key,"process_completed":false,"status":self.test_tool.agent_run_status(),"tracking":"host tracks completion independently; use testing.status for fresh status","formal_verdict":"not available until actual report"}})
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
                        if status["completed_run_identity"] != p["key"] {
                            agent::failure("未取得匹配运行身份的完成记录，请核对测试报告")
                        } else {
                            json!({"ok":p["action"]=="testing.stop"||status["process_exit_code"]==0,"data":{"process_completed":true,"formal_verdict":"consult actual test report; process completion is not a pass verdict","status":status}})
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
                json!({"ok":true,"data":self.instruments.api_list(self.lang,&json!({}))})
            }
            "instrument.command" | "instrument.capture" => {
                match SerialToolPanel::validate_agent_instrument(method, p).and_then(|v|self.instruments.agent_validate_device(p,method=="instrument.capture").map(|_|v)) {
                    Err(error) => agent::failure(error),
                    Ok(_) => {
                        let verification=if p.get("verify").is_some(){match self.instruments.independent_scope(p){
                            Ok(v)=>v,Err(e)=>return HostReply::value(agent::failure(e)),
                        }}else{Value::Null};
                        let inventory = self.instruments.api_list(self.lang, &json!({}));
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
                let result = self.instruments.api_job_get(p);
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
                    json!({"ok":result.data["ok"],"data":{"command_completed":result.data["ok"],"physical_verified":false,"verification":"Device API job completion only; setpoint readback and physical tolerance not verified. Use an explicit measurement.","job":result.data}})
                } else {
                    reply(result)
                }
            }
            _ => agent::failure("未实现或不允许的宿主调用"),
        };
        HostReply::value(value)
    }
}

/// The serial model keeps its conversation; execution state reuses the canonical host.
pub(super) struct SerialBroker {
    tx: crossbeam_channel::Sender<crate::ai::broker::Request>,
    rx: crossbeam_channel::Receiver<crate::ai::broker::Request>,
    session: agent::Session,
    pending: Option<BrokerPending>,
}
struct BrokerPending {
    origin: crate::ai::broker::Request,
    request: HostCall,
    item: crate::ai::execution::HostItem,
    started: bool,
    target_page: String,
    target_binding: Value,
    approved: bool,
}
impl Default for SerialBroker {
    fn default()->Self {let(tx,rx)=crossbeam_channel::bounded(8);Self{tx,rx,session:Default::default(),pending:None}}
}
fn target_binding(context:&ContextState,page:&str)->Value {
    serde_json::from_str::<Value>(&context.binding).unwrap_or_default()[page].clone()
}
impl WiParseApp {
    fn tick_serial_broker(&mut self,ctx:&egui::Context) {
        use std::sync::atomic::Ordering;
        use std::time::{Duration,Instant};
        ctx.data_mut(|d|d.insert_temp(egui::Id::new("wiparse-canonical-agent"),self.serial_agent.tx.clone()));
        let mut broker=std::mem::take(&mut self.serial_agent);
        if broker.session.tracking_test() {
            self.test_tool.agent_tick(self.lang);
            broker.session.update_test(self.test_tool.agent_run_status());
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if broker.pending.is_none() {
            if let Ok(origin)=broker.rx.try_recv() {
                let mut context=self.agent_context();context.page="serial".into();
                let page=wiparse_core::engineering_agent::validate(&wiparse_core::engineering_agent::Call{method:origin.method.clone(),params:origin.params.clone()}).map_or("any",|t|t.page).to_owned();
                let request=HostCall{id:uuid::Uuid::new_v4().to_string(),call:wiparse_core::engineering_agent::Call{method:origin.method.clone(),params:origin.params.clone()},context:context.clone(),cancelled:origin.guard.cancelled.clone(),internal:false};
                broker.session.set_context(context.clone());
                broker.pending=Some(BrokerPending{origin,request,item:Default::default(),started:false,target_binding:target_binding(&context,&page),target_page:page,approved:false});
            }
        }
        if let Some(mut pending)=broker.pending.take() {
            let now=Instant::now();
            let context=self.agent_context();
            let validated=wiparse_core::engineering_agent::validate(&wiparse_core::engineering_agent::Call{method:pending.origin.method.clone(),params:pending.origin.params.clone()});
            let permission=validated.as_ref().is_ok_and(|t|!matches!(t.effect,wiparse_core::engineering_agent::Effect::Device|wiparse_core::engineering_agent::Effect::Test)||pending.origin.guard.controls);
            let stale=context.project!=pending.origin.guard.project || !(if pending.started {self.serial.ai_platform_result_guard_valid(&pending.origin.guard)}else{self.serial.ai_platform_guard_for(&pending.origin.guard,&pending.origin.method,&pending.origin.params)})
                || target_binding(&context,&pending.target_page)!=pending.target_binding;
            let mut result=if pending.request.cancelled.load(Ordering::Acquire)||stale {Some(agent::failure("AI.CANCELLED_OR_STALE"))}
                else if !permission {Some(agent::failure("AI.CONTROL_LOCKED_OR_INVALID_TOOL"))}
                else if now>=pending.origin.deadline {Some(agent::failure("AI.TOOL_TIMEOUT: execution may be unknown; do not retry output-changing commands"))}
                else {None};
            let review=pending.origin.method=="platform.invoke"&&validated.as_ref().is_ok_and(|t|matches!(t.effect,wiparse_core::engineering_agent::Effect::Device|wiparse_core::engineering_agent::Effect::Test));
            let mut declined=false;
            if result.is_none()&&review&&!pending.approved {
                egui::Window::new("AI 平台操作确认").id(egui::Id::new("platform-action-review")).collapsible(false).resizable(true).show(ctx,|ui|{
                    ui.label("请核对目标与参数；确认后执行这一次操作。");
                    egui::ScrollArea::vertical().max_height(320.).show(ui,|ui|{ui.monospace(serde_json::to_string_pretty(&pending.origin.params).unwrap_or_default());});
                    ui.horizontal(|ui|{if ui.button("确认执行").clicked(){pending.approved=true;}if ui.button("取消此操作").clicked(){declined=true;}});
                });
            }
            if declined{result=Some(agent::failure("PLATFORM.USER_DECLINED"));}
            if result.is_none() && (!review||pending.approved) {
                let continuation=pending.item.take_due(now);
                let dispatch=!pending.started || continuation.is_some();
                if let Some(call)=continuation {
                    pending.request.call=call;
                    pending.request.internal=true;
                }
                let received=if dispatch {
                    pending.started=true;
                    let response=self.agent_dispatch(ctx,&mut broker.session,&pending.request);
                    if !pending.request.internal && pending.origin.method=="platform.invoke" && matches!(pending.origin.params["method"].as_str(),Some("ui.serial.open"|"ui.serial.close"|"ui.serial.clear"|"ui.serial.tab"|"serial.select"|"serial.monitor.start"|"serial.monitor.stop")) {
                        self.serial.ai_accept_source_transition(&mut pending.origin.guard);
                    }
                    pending.target_binding=target_binding(&self.agent_context(),&pending.target_page);
                    pending.item.attach(response)
                } else { pending.item.receive() };
                if let Some(value)=received {
                    result=pending.item.advance(value,&pending.request.call,pending.request.internal);
                }
            }
            if let Some(value)=result {
                let _=pending.origin.reply.try_send(crate::backend::InvokeReply{ok:value["ok"]!=false,cmd:pending.origin.method.clone(),error:value["error"].as_str().map(str::to_owned),data:value["data"].clone()});
            }else{broker.pending=Some(pending);}
        }
        if broker.pending.is_some()||!broker.rx.is_empty(){self.waveform.agent_tick(self.lang);self.test_tool.agent_tick(self.lang);ctx.request_repaint_after(Duration::from_millis(50));}
        self.serial_agent=broker;
    }
}

impl WiParseApp {
 fn invoke_platform(&mut self,ctx:&egui::Context,r:&HostCall)->HostReply {
  use std::sync::atomic::Ordering;
  if r.cancelled.load(Ordering::Acquire){return HostReply::value(agent::failure("PLATFORM.CANCELLED"));}
  if let Err(e)=wiparse_core::engineering_agent::validate(&r.call){return HostReply::value(agent::failure(e));}
  let method=r.call.params["method"].as_str().unwrap().to_owned();let params=r.call.params["params"].clone();
  if method=="instrument.command" {
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
        if session.tracking_test() {self.test_tool.agent_tick(self.lang);session.update_test(self.test_tool.agent_run_status());}
        if let Some(source)=session.navigation.take(){if source.project==context.project{self.active=MainTab::TestReport;self.show_test_report=true;}}
        if let Some(request)=session.take_request() {
            let response=if session.authorize(&request,&context){self.agent_dispatch(ctx,&mut session,&request)}else{HostReply::value(agent::failure("报告来源或权限已变化，本次操作未执行"))};
            let mut updated=self.agent_context();updated.page="test_report".into();
            session.host_transition(updated);session.attach(&request.id,response);
        }
        if session.busy()||session.tracking_test(){ctx.request_repaint_after(std::time::Duration::from_millis(100));}
        self.report_ai.session=session;
    }
}
