//! In-process bridge to the SAME canonical host executor used by the global Agent.
use crate::backend::{AiGuard, InvokeReply};
use serde_json::Value;
use std::time::{Duration, Instant};
pub(crate) struct Request {
    pub method: String,
    pub params: Value,
    pub guard: AiGuard,
    pub reply: crossbeam_channel::Sender<InvokeReply>,
    pub deadline: Instant,
}
pub(crate) fn supports(method: &str) -> bool {
    matches!(method,"platform.catalog"|"platform.invoke") || (method.starts_with("wave.") || method.starts_with("data.") || method.starts_with("report.")
        || method.starts_with("calc.") || method.starts_with("testing."))
        && wiparse_core::engineering_agent::tool(method).is_some_and(|t|!matches!(t.effect,wiparse_core::engineering_agent::Effect::Test|wiparse_core::engineering_agent::Effect::Device))
}
pub(crate) fn enqueue(ctx: &egui::Context, method: &str, params: Value, guard: AiGuard)
    -> Result<crossbeam_channel::Receiver<InvokeReply>, String> {
    let tx: crossbeam_channel::Sender<Request> = ctx.data(|d|d.get_temp(egui::Id::new("wiparse-canonical-agent")))
        .ok_or("AI.HOST_EXECUTOR_UNAVAILABLE")?;
    let (reply,rx)=crossbeam_channel::bounded(1);
    tx.try_send(Request{method:method.into(),params,guard,reply,
        deadline:Instant::now()+Duration::from_secs(if method=="testing.wait"{600}else{90})})
        .map_err(|_|"AI.HOST_EXECUTOR_BUSY")?;
    ctx.request_repaint();Ok(rx)
}
