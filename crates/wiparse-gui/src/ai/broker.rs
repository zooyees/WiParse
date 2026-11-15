//! In-process bridge to the SAME canonical host executor used by the global Agent.
use crate::backend::{AiGuard, InvokeReply};
use serde_json::Value;
use std::time::Instant;
pub(crate) fn approval_fingerprint(method:&str,params:&Value,task:Option<&str>,project:&str,binding:&str,target:&Value)->String {
    let scope=serde_json::json!({"method":method,"params":params,"task":task,
        "project":project,"serial_binding":binding,"target":target});
    wiparse_core::update::hex_sha256(scope.to_string().as_bytes())
}
pub(crate) struct Request {
    pub method: String,
    pub params: Value,
    pub guard: AiGuard,
    pub reply: crossbeam_channel::Sender<InvokeReply>,
    pub deadline: Instant,
}
pub(crate) fn has_instrument_executor(ctx:&egui::Context,method:&str)->bool {
    matches!(method,"instrument.list"|"instrument.command"|"instrument.capture"|"instrument.waveform_source") &&
        ctx.data(|d| d.get_temp::<crossbeam_channel::Sender<Request>>(egui::Id::new("wiparse-canonical-agent")).is_some())
}
pub(crate) fn available(ctx:&egui::Context)->bool {
    ctx.data(|d| d.get_temp::<crossbeam_channel::Sender<Request>>(egui::Id::new("wiparse-canonical-agent")).is_some())
}
pub(crate) fn supports(method: &str) -> bool {
    if matches!(method, "testing.preflight" | "testing.run" | "testing.stop" | "testing.pause" | "testing.resume") { return true; }
    matches!(method,"platform.catalog"|"platform.invoke"|"instrument.readback") || (method.starts_with("wave.") || method.starts_with("data.") || method.starts_with("report.")
        || method.starts_with("calc.") || method.starts_with("testing."))
        && wiparse_core::engineering_agent::tool(method).is_some_and(|t|!matches!(t.effect,wiparse_core::engineering_agent::Effect::Test|wiparse_core::engineering_agent::Effect::Device))
}
pub(crate) fn enqueue(ctx: &egui::Context, method: &str, params: Value, guard: AiGuard)
    -> Result<crossbeam_channel::Receiver<InvokeReply>, String> {
    let tx: crossbeam_channel::Sender<Request> = ctx.data(|d|d.get_temp(egui::Id::new("wiparse-canonical-agent")))
        .ok_or("AI.HOST_EXECUTOR_UNAVAILABLE")?;
    let (reply,rx)=crossbeam_channel::bounded(1);
    tx.try_send(Request{method:method.into(),params,guard,reply,
        deadline:Instant::now()+super::execution::timeout(method)})
        .map_err(|_|"AI.HOST_EXECUTOR_BUSY")?;
    ctx.request_repaint();Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirmation_binds_each_action_and_context_component() {
        let hash=|method,params:&Value,task,project,binding,target:&Value|
            approval_fingerprint(method,params,task,project,binding,target);
        let params=serde_json::json!({"device_id":1});let target=serde_json::json!({"session":"s1"});
        let expected=hash("testing.run",&params,Some("t1"),"p1","b1",&target);
        assert_ne!(expected,hash("testing.stop",&params,Some("t1"),"p1","b1",&target));
        assert_ne!(expected,hash("testing.run",&serde_json::json!({"device_id":2}),Some("t1"),"p1","b1",&target));
        assert_ne!(expected,hash("testing.run",&params,Some("t2"),"p1","b1",&target));
        assert_ne!(expected,hash("testing.run",&params,Some("t1"),"p2","b1",&target));
        assert_ne!(expected,hash("testing.run",&params,Some("t1"),"p1","b2",&target));
        assert_ne!(expected,hash("testing.run",&params,Some("t1"),"p1","b1",&serde_json::json!({"session":"s2"})));
    }

    #[test]
    fn simultaneous_enqueue_is_bounded_and_never_silently_drops_a_request() {
        use std::sync::{Arc, Barrier, atomic::AtomicBool};
        let ctx = egui::Context::default();
        let (tx, rx) = crossbeam_channel::bounded::<Request>(1);
        ctx.data_mut(|data| data.insert_temp(egui::Id::new("wiparse-canonical-agent"), tx));
        let barrier = Arc::new(Barrier::new(9));
        let mut workers = Vec::new();
        for n in 0..8 {
            let ctx = ctx.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                let guard = AiGuard { task_id: Some(format!("task-{n}")), skill_ids: vec![],
                    binding: "source".into(), project: "project".into(), controls: false,
                    cancelled: Arc::new(AtomicBool::new(false)) };
                barrier.wait();
                enqueue(&ctx, "platform.catalog", serde_json::json!({}), guard)
            }));
        }
        barrier.wait();
        let results: Vec<_> = workers.into_iter().map(|worker| worker.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| matches!(result, Err(e) if e == "AI.HOST_EXECUTOR_BUSY")).count(), 7);
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err());
    }
}
