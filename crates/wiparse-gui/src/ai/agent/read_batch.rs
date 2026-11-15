//! Provider batches collect independently authorized observations in one model turn.
use super::*;
use wiparse_core::ai::tools::{self, Assistant, Round};

impl Session {
    /// Recover a stalled investigation with one observation of the bound page.
    /// Successful inspection is evidence about the source, never a physical verdict.
    pub(super) fn try_scope_read(&mut self)->bool {
        if self.reviewed_local_task() {return false;}
        let Some(c)=&self.context else{return false;};
        if !self.read_data || self.round>=self.max_rounds{return false;}
        let method=match c.page.as_str(){
            "waveform"=>"wave.channels","serial"=>"log.files","test_report"=>"report.inspect",
            "data_analysis"=>"data.inspect","instrument"=>"instrument.list","testing"=>"testing.status",
            _=>"page.inspect",
        };
        if self.steps.iter().any(|s|s.task==self.task && s.call.method==method){return false;}
        self.enqueue(Call{method:method.into(),params:json!({})},false);
        true
    }
    pub(super) fn prepare_read_batch(&mut self, message: &Assistant) -> Result<Call, String> {
        if !(2..=4).contains(&message.calls.len()) {
            return Err("AGENT.READ_BATCH_LIMIT: request 2 to 4 independent reads".into());
        }
        let context = self.context.as_ref().ok_or("AGENT.SOURCE_REQUIRED")?;
        let mut ids = std::collections::HashSet::new();
        let mut batch = VecDeque::new();
        for request in &message.calls {
            if request.id.is_empty() || !ids.insert(&request.id) {
                return Err("AGENT.DUPLICATE_TOOL_ID".into());
            }
            let (method, params) = tools::normalized(request)
                .map_err(|_| "AGENT.INVALID_BATCH_ARGUMENTS".to_owned())?;
            let call = Call { method, params };
            if self.reviewed_local_task() && !(self.reviewed_tool_allowed(&call.method) && call.method!="agent.plan") {
                return Err("AGENT.REVIEWED_CALCULATION_SCOPE".into());
            }
            let tool = contract::validate(&call)?;
            // Async jobs that replace a shared dataset or require a previous
            // result must use their ordinary single-call path.
            if !matches!(call.method.as_str(),
                "agent.help" | "agent.evidence" | "evidence.get" | "platform.catalog" |
                "page.inspect" | "wave.channels" | "wave.measure" | "wave.timing" | "wave.frames" |
                "serial.inspect" | "log.files" | "data.snapshot.read" | "data.snapshot.stats" | "data.inspect" | "report.read" | "report.inspect" |
                "report.search" | "testing.status" | "testing.inspect" | "testing.cases" |
                "calc.inspect" | "calc.evaluate" | "instrument.list") ||
                !matches!(tool.effect, Effect::Read | Effect::LocalCompute) {
                return Err("AGENT.BATCH_REQUIRES_INDEPENDENT_READS".into());
            }
            if tool.effect != Effect::LocalCompute && (!self.read_data ||
                !self.cross_tools && !matches!(tool.page,"current"|"any") && tool.page != context.page) {
                return Err("AGENT.BATCH_SCOPE_DENIED".into());
            }
            if batch.contains(&call) {return Err("AGENT.DUPLICATE_BATCH_QUERY".into());}
            batch.push_back(call);
        }
        let first = batch.pop_front().ok_or("AGENT.EMPTY_BATCH")?;
        for call in &batch {
            let page = self.call_page(call).to_owned();
            self.watched.insert(page);
        }
        self.batch_remaining = batch;
        self.batch_outputs.clear();
        Ok(first)
    }

    pub(super) fn pair_native_output(&mut self, call: &Call, value: &Value) -> Option<Call> {
        let (assistant,expected) = self.native_pending.as_mut()?;
        if expected != call {return None;}
        let id = format!("E{}",self.evidence_sequence+1);
        let result = if assistant.calls.len()>1 && value.to_string().len()>16*1024 {
            json!({"ok":value["ok"],"error":value["error"],"details":"Use agent.evidence with this evidence_id for details","complete_in_transcript":false})
        } else {value.clone()};
        self.batch_outputs.push(json!({"evidence_id":id,"result":result}));
        if let Some(next)=self.batch_remaining.pop_front() {
            *expected=next.clone();
            return Some(next);
        }
        let (assistant,_)=self.native_pending.take()?;
        self.native_rounds.push(Round{assistant,outputs:std::mem::take(&mut self.batch_outputs)});
        while self.native_rounds.len()>1 && (self.native_rounds.len()>6 ||
            serde_json::to_vec(&self.native_rounds).map_or(usize::MAX,|v|v.len())>96*1024) {
            self.native_rounds.remove(0);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn batch(methods: &[&str])->Assistant {
        Assistant{content:None,reasoning_content:None,calls:methods.iter().enumerate().map(|(i,m)|tools::ToolCall{
            id:format!("c{i}"),name:"wiparse_call".into(),arguments:json!({"method":m,"params":{}}).to_string()
        }).collect()}
    }
    fn session()->Session {
        let mut s=Session::default();
        s.set_context(ContextState{page:"waveform".into(),binding:"scope".into(),project:"lab".into(),testing:false});
        s
    }
    #[test]
    fn read_batch_pairs_every_id_in_order_without_model_roundtrip() {
        let mut s=session();s.cross_tools=true;
        let message=batch(&["wave.channels","report.inspect"]);
        let first=s.prepare_read_batch(&message).unwrap();
        s.native_pending=Some((message,first.clone()));
        let second=s.pair_native_output(&first,&json!({"ok":true})).unwrap();
        assert!(s.native_rounds.is_empty());
        s.evidence_sequence+=1;
        assert!(s.pair_native_output(&second,&json!({"ok":false,"error":"missing report"})).is_none());
        let round=&s.native_rounds[0];
        assert_eq!(round.outputs[0]["evidence_id"],"E1");
        assert_eq!(round.outputs[1]["result"]["ok"],false);
        assert_eq!(round.assistant.calls[1].id,"c1");
    }
    #[test]
    fn invalid_member_rejects_entire_batch_before_dispatch() {
        let mut s=session();
        for methods in [["page.inspect","report.inspect"],["page.inspect","testing.stop"],["page.inspect","page.inspect"]]{
            assert!(s.prepare_read_batch(&batch(&methods)).is_err());
            assert!(s.batch_remaining.is_empty() && s.outgoing.is_empty());
        }
    }
}
