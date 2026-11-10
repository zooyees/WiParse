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
        let response = self.agent_dispatch(ctx, &request);
        // A host-approved transition (start reception / configured decode) may change its own binding.
        self.ai.agent.host_transition(self.agent_context());
        self.ai.agent.attach(&request.id, response);
    }
    fn agent_dispatch(&mut self, ctx: &egui::Context, r: &HostCall) -> HostReply {
        let p = &r.call.params;
        let method = r.call.method.as_str();
        if !r.internal {
            if let Err(error) = wiparse_core::engineering_agent::validate(&r.call) {
                return HostReply::value(agent::failure(error));
            }
        }
        let project = self.test_tool.current_project_id().to_owned();
        let value = match method {
            "evidence.get"=>self.ai.agent.evidence(p["id"].as_str().unwrap_or("")),
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
                    Ok((spec, memory)) => self.ai.agent.log_jobs.analyze(
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
            "data.inspect" => json!({"ok":true,"data":self.data_analysis.ai_summary()}),
            "data.statistics" => self
                .data_analysis
                .agent_statistics(&mut self.ai.agent.data_job, true),
            "data.statistics.result" if r.internal => self
                .data_analysis
                .agent_statistics(&mut self.ai.agent.data_job, false),
            "report.inspect" => self.test_report.agent_index(p),
            "report.search" => return match self.test_report.agent_search_input(p,&self.test_tool.data_root_path()) {
                Ok((spec,query))=>self.ai.agent.report_jobs.analyze(ctx,query,spec,None,r.cancelled.clone()),
                Err(e)=>HostReply::value(agent::failure(e))
            },
            "calc.inspect" => json!({"ok":true,"data":self.calculator.ai_summary()}),
            "calc.evaluate" => CalculatorPanel::new().agent_calculate(p),
            "calc.apply" => self.calculator.agent_calculate(p),
            "testing.inspect" | "testing.status" => json!({"ok":true,"data":self.test_tool.agent_run_status()}),
            "testing.propose" => return self.test_tool.agent_propose(p,ctx),
            "testing.apply" => self.test_tool.agent_apply_draft(p),
            "testing.select" => self.test_tool.agent_select_cases(p),
            "testing.pause" | "testing.resume" => {
                if !self.ai.agent.owns_test(self.test_tool.agent_job_key().as_deref()){agent::failure("不能操作其他来源发起的测试")}else{self.test_tool.agent_run_control(if method=="testing.pause"{"pause"}else{"resume"})}
            }
            "testing.cases" => self.test_tool.agent_cases(p),
            "testing.run" | "testing.preflight" => {
                let result = self.test_tool.agent_start(method == "testing.preflight");
                if result.ok {
                    let key = self.test_tool.agent_job_key();
                    self.ai.agent.mark_test(key.clone());
                    json!({"ok":true,"data":{"state":"accepted","run_id":key,"process_completed":false,"status":self.test_tool.agent_run_status(),"tracking":"host tracks completion independently; use testing.status for fresh status","formal_verdict":"not available until actual report"}})
                } else {
                    reply(result)
                }
            }
            "testing.stop" => {
                if !self
                    .ai
                    .agent
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
                    self.ai.agent.mark_test(None);
                    {
                        let status = self.test_tool.agent_run_status();
                        if status["completed_run_identity"] != p["key"] {
                            agent::failure("未取得匹配运行身份的完成记录，请核对测试报告")
                        } else {
                            json!({"ok":p["action"]=="testing.stop"||status["process_exit_code"]==0,"data":{"process_completed":true,"formal_verdict":"consult actual test report; process completion is not a pass verdict","status":status}})
                        }
                    }
                } else if current.as_deref() == p["key"].as_str() {
                    wait("testing.result", p.clone())
                } else {
                    agent::failure("测试运行已被替换，停止等待旧运行")
                }
            }
            "instrument.list" => {
                json!({"ok":true,"data":self.instruments.api_list(self.lang,&json!({}))})
            }
            "instrument.command" | "instrument.capture" => {
                match SerialToolPanel::validate_agent_instrument(method, p).and_then(|v|self.instruments.agent_validate_device(p,method=="instrument.capture").map(|_|v)) {
                    Err(error) => agent::failure(error),
                    Ok(_) => {
                        let inventory = self.instruments.api_list(self.lang, &json!({}));
                        if !inventory["devices"].as_array().is_some_and(|devices| {
                            devices.iter().any(|d| {
                                d["id"] == p["device_id"] || d["device_id"] == p["device_id"]
                            })
                        }) {
                            agent::failure("所选设备不在当前已连接会话中")
                        } else {
                            let result = if method == "instrument.command" {
                                self.instruments.api_command(p)
                            } else {
                                self.instruments.agent_capture(p)
                            };
                            if result.ok {
                                wait("instrument.result", json!({"job_id":result.data["job_id"]}))
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
