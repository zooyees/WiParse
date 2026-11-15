//! Staged serial command handoff. Not connected to the live serial path yet.
//!
//! All methods below perform bounded in-memory work only. The caller must
//! route *every* source and authorization change through the same gate before
//! relying on it for dispatch; it is not a substitute for the device journal.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub project: String,
    pub port: String,
    pub session: String,
    pub baud: u32,
    pub source_revision: u64,
    pub authorization_revision: u64,
    pub task: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Operation { Start, Stop }
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Command { pub id: uuid::Uuid, pub target: Target, pub operation: Operation }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome { Queued, Claimed, AwaitingObservation, CancelledBeforeClaim, Unknown }
struct Approval { command: Command, expires: Instant }
struct Entry { command: Command, outcome: Outcome, expires:Instant }
#[derive(Default)]
struct State { current: Option<Target>, authorized: bool, approval: Option<Approval>, entry: Option<Entry> }
#[derive(Clone, Default)]
pub(super) struct Gate { state: Arc<Mutex<State>> }

/// Exact command and source permits travel together to the owning worker.
pub(super) struct ReviewedStop {
    pub gate: Gate,
    pub command: Command,
    pub sources: wiparse_core::ai::ReviewedPair<serde_json::Value,serde_json::Value>,
    pub service: wiparse_core::ai::AiService,
    pub service_revision: usize,
    pub reply: crossbeam_channel::Sender<Result<(), String>>,
    pub slot: Arc<Mutex<Option<super::SerialStopIntent>>>,
    pub intent: super::SerialStopIntent,
}
impl ReviewedStop {
    pub fn claim(&self, stop: &AtomicBool) -> Result<(), &'static str> {
        let result = self.service.commit_config_revision(self.service_revision,||self.sources.commit_in_memory(|| {
            let command=self.gate.claim(self.command.id)?;
            if command != self.command || command.operation != Operation::Stop {
                let _ = self.gate.cancel(self.command.id);
                return Err("SERIAL.GATE_COMMAND_MISMATCH");
            }
            let mut slot=match self.slot.lock() {Ok(slot)=>slot,Err(_)=>{let _=self.gate.cancel(self.command.id);return Err("SERIAL.GATE_INTENT_UNAVAILABLE");}};
            if slot.is_some() {let _=self.gate.cancel(self.command.id);return Err("SERIAL.GATE_BUSY_OR_UNKNOWN");}
            *slot=Some(self.intent.clone());
            stop.store(true, Ordering::SeqCst);
            Ok(())
        })).flatten();
        match result {Some(result)=>result,None=>{let _=self.gate.cancel(self.command.id);Err("SERIAL.GATE_SOURCE_OR_CONFIG_CHANGED")}}
    }
}

impl Gate {
    /// The integration must call this while publishing every target/authority
    /// transition. Claimed operations become unknown, not cancelled or replayable.
    pub fn publish(&self, target: Option<Target>, authorized: bool) -> Result<(), &'static str> {
        let mut state=self.state.lock().map_err(|_|"SERIAL.GATE_POISONED")?;
        if state.current != target || state.authorized != authorized {
            state.approval=None;
            if let Some(entry)=state.entry.as_mut() {
                entry.outcome=match entry.outcome {
                    Outcome::Queued=>Outcome::CancelledBeforeClaim,
                    Outcome::Claimed|Outcome::AwaitingObservation=>Outcome::Unknown,
                    other=>other,
                };
            }
            state.current=target;
            state.authorized=authorized;
        }
        Ok(())
    }
    /// Called only after explicit user confirmation of the complete command.
    pub fn confirm(&self, target: &Target, operation: Operation, lifetime: Duration) -> Result<Command, &'static str> {
        self.confirm_with_id(target,operation,lifetime,uuid::Uuid::new_v4())
    }
    pub fn confirm_with_id(&self, target: &Target, operation: Operation, lifetime: Duration, id:uuid::Uuid) -> Result<Command, &'static str> {
        let mut state=self.state.lock().map_err(|_|"SERIAL.GATE_POISONED")?;
        if !state.authorized || state.current.as_ref()!=Some(target) {return Err("SERIAL.GATE_STALE_OR_UNAUTHORIZED");}
        if state.entry.as_ref().is_some_and(|e| matches!(e.outcome,Outcome::Queued|Outcome::Claimed|Outcome::AwaitingObservation|Outcome::Unknown)) {return Err("SERIAL.GATE_BUSY_OR_UNKNOWN");}
        let expires=Instant::now().checked_add(lifetime).ok_or("SERIAL.GATE_INVALID_EXPIRY")?;
        let command=Command{id,target:target.clone(),operation};
        state.approval=Some(Approval{command:command.clone(),expires});
        Ok(command)
    }
    /// A confirmation is consumed even if validation fails.
    pub fn enqueue(&self, command: &Command) -> Result<(), &'static str> {
        let mut state=self.state.lock().map_err(|_|"SERIAL.GATE_POISONED")?;
        let approval=state.approval.take().ok_or("SERIAL.GATE_CONFIRMATION_REQUIRED")?;
        if approval.command!=*command || Instant::now()>=approval.expires || !state.authorized || state.current.as_ref()!=Some(&command.target) {
            return Err("SERIAL.GATE_STALE_OR_UNAUTHORIZED");
        }
        if state.entry.as_ref().is_some_and(|e|matches!(e.outcome,Outcome::Queued|Outcome::Claimed|Outcome::AwaitingObservation|Outcome::Unknown)) {return Err("SERIAL.GATE_BUSY_OR_UNKNOWN");}
        state.entry=Some(Entry{command:command.clone(),outcome:Outcome::Queued,expires:approval.expires});
        Ok(())
    }
    /// Worker calls this immediately before executing, with no I/O under lock.
    pub fn claim(&self, id: uuid::Uuid) -> Result<Command, &'static str> {
        let mut state=self.state.lock().map_err(|_|"SERIAL.GATE_POISONED")?;
        if state.entry.as_ref().is_some_and(|e|e.command.id==id && e.outcome==Outcome::Queued && Instant::now()>=e.expires) {
            if let Some(entry)=state.entry.as_mut() {entry.outcome=Outcome::CancelledBeforeClaim;}
            return Err("SERIAL.GATE_CONFIRMATION_EXPIRED");
        }
        let permitted=state.authorized && state.entry.as_ref().is_some_and(|e|e.command.id==id && e.outcome==Outcome::Queued && state.current.as_ref()==Some(&e.command.target));
        if !permitted {return Err("SERIAL.GATE_NOT_CLAIMABLE");}
        let entry=state.entry.as_mut().ok_or("SERIAL.GATE_NOT_CLAIMABLE")?;
        entry.outcome=Outcome::Claimed;
        Ok(entry.command.clone())
    }
    pub fn cancel(&self, id: uuid::Uuid) -> Result<Outcome, &'static str> {
        let mut state=self.state.lock().map_err(|_|"SERIAL.GATE_POISONED")?;
        if state.approval.as_ref().is_some_and(|approval|approval.command.id==id) {
            state.approval=None;
            return Ok(Outcome::CancelledBeforeClaim);
        }
        let entry=state.entry.as_mut().filter(|e|e.command.id==id).ok_or("SERIAL.GATE_UNKNOWN_COMMAND")?;
        entry.outcome=match entry.outcome {Outcome::Queued=>Outcome::CancelledBeforeClaim,Outcome::Claimed|Outcome::AwaitingObservation=>Outcome::Unknown,other=>other};
        Ok(entry.outcome)
    }
    /// A positive result only means the worker returned; it is not proof of
    /// physical connection or port release. Failed/late replies remain unknown.
    pub fn complete(&self, id: uuid::Uuid, worker_ok: bool) -> Result<Outcome, &'static str> {
        let mut state=self.state.lock().map_err(|_|"SERIAL.GATE_POISONED")?;
        let entry=state.entry.as_mut().filter(|e|e.command.id==id).ok_or("SERIAL.GATE_UNKNOWN_COMMAND")?;
        if entry.outcome==Outcome::Claimed {entry.outcome=if worker_ok {Outcome::AwaitingObservation} else {Outcome::Unknown};}
        Ok(entry.outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target()->Target {Target{project:"p".into(),port:"COM1".into(),session:"s1".into(),baud:115200,source_revision:1,authorization_revision:1,task:"task".into()}}
    #[test] fn changed_source_revokes_confirmation_and_queued_command() {
        let gate=Gate::default();let old=target();gate.publish(Some(old.clone()),true).unwrap();
        let cmd=gate.confirm(&old,Operation::Stop,Duration::from_secs(5)).unwrap();
        let mut new=old.clone();new.session="s2".into();gate.publish(Some(new),true).unwrap();
        assert!(gate.enqueue(&cmd).is_err());
        gate.publish(Some(old.clone()),true).unwrap();let next=gate.confirm(&old,Operation::Stop,Duration::from_secs(5)).unwrap();gate.enqueue(&next).unwrap();
        gate.publish(None,false).unwrap();assert!(gate.claim(next.id).is_err());
        assert_eq!(gate.cancel(next.id).unwrap(),Outcome::CancelledBeforeClaim);
    }
    #[test] fn claimed_cancel_and_late_success_remain_unknown() {
        let gate=Gate::default();let t=target();gate.publish(Some(t.clone()),true).unwrap();
        let cmd=gate.confirm(&t,Operation::Start,Duration::from_secs(5)).unwrap();gate.enqueue(&cmd).unwrap();
        assert!(gate.enqueue(&cmd).is_err());assert_eq!(gate.claim(cmd.id).unwrap(),cmd);
        assert_eq!(gate.cancel(cmd.id).unwrap(),Outcome::Unknown);assert_eq!(gate.complete(cmd.id,true).unwrap(),Outcome::Unknown);
        assert!(gate.confirm(&t,Operation::Start,Duration::from_secs(5)).is_err());
    }
    #[test] fn altered_command_consumes_approval_and_expired_confirmation_fails() {
        let gate=Gate::default();let t=target();gate.publish(Some(t.clone()),true).unwrap();
        let cmd=gate.confirm(&t,Operation::Start,Duration::from_secs(5)).unwrap();
        let mut altered=cmd.clone();altered.operation=Operation::Stop;
        assert!(gate.enqueue(&altered).is_err());assert!(gate.enqueue(&cmd).is_err());
        let expired=gate.confirm(&t,Operation::Start,Duration::ZERO).unwrap();
        assert!(gate.enqueue(&expired).is_err());
    }
    #[test] fn claimed_operation_loses_authority_and_cannot_be_replayed() {
        let gate=Gate::default();let t=target();gate.publish(Some(t.clone()),true).unwrap();
        let cmd=gate.confirm(&t,Operation::Stop,Duration::from_secs(5)).unwrap();gate.enqueue(&cmd).unwrap();gate.claim(cmd.id).unwrap();
        gate.publish(Some(t.clone()),false).unwrap();gate.publish(Some(t.clone()),true).unwrap();
        assert_eq!(gate.complete(cmd.id,true).unwrap(),Outcome::Unknown);
        assert!(gate.confirm(&t,Operation::Stop,Duration::from_secs(5)).is_err());
    }
    #[test] fn reviewed_stop_claim_checks_both_sources_before_setting_flag() {
        use wiparse_core::ai::{ReviewedSource,source_lease::SourceLease};
        let source=SourceLease::default();let authority=SourceLease::default();
        let pair=ReviewedSource::capture(&source,||serde_json::json!({"source":"COM1"})).unwrap()
            .join(ReviewedSource::capture(&authority,||serde_json::json!({"allowed":true})).unwrap());
        let gate=Gate::default();let t=target();gate.publish(Some(t.clone()),true).unwrap();
        let command=gate.confirm(&t,Operation::Stop,Duration::from_secs(5)).unwrap();gate.enqueue(&command).unwrap();
        let (reply,_)=crossbeam_channel::bounded(1);
        let queued=ReviewedStop{gate:gate.clone(),command:command.clone(),sources:pair,reply,
            service:wiparse_core::ai::AiService::new(wiparse_core::ai::AiConfig::default()),service_revision:0,
            slot:Arc::new(Mutex::new(None)),intent:super::super::SerialStopIntent{command_id:command.id.to_string(),project:"p".into(),fingerprint:"f".into(),audit:None}};
        authority.mutate_always(||());
        let stop=AtomicBool::new(false);assert!(queued.claim(&stop).is_err());
        assert!(!stop.load(Ordering::SeqCst));
        assert_eq!(gate.cancel(command.id).unwrap(),Outcome::CancelledBeforeClaim);
    }
    #[test] fn reviewed_stop_claim_sets_flag_only_once_with_current_sources() {
        use wiparse_core::ai::{ReviewedSource,source_lease::SourceLease};
        let source=SourceLease::default();let authority=SourceLease::default();
        let pair=ReviewedSource::capture(&source,||serde_json::json!({})).unwrap()
            .join(ReviewedSource::capture(&authority,||serde_json::json!({})).unwrap());
        let gate=Gate::default();let t=target();gate.publish(Some(t.clone()),true).unwrap();
        let command=gate.confirm(&t,Operation::Stop,Duration::from_secs(5)).unwrap();gate.enqueue(&command).unwrap();
        let (reply,_)=crossbeam_channel::bounded(1);
        let queued=ReviewedStop{gate:gate.clone(),command:command.clone(),sources:pair,reply,
            service:wiparse_core::ai::AiService::new(wiparse_core::ai::AiConfig::default()),service_revision:0,
            slot:Arc::new(Mutex::new(None)),intent:super::super::SerialStopIntent{command_id:command.id.to_string(),project:"p".into(),fingerprint:"f".into(),audit:None}};
        let stop=AtomicBool::new(false);assert!(queued.claim(&stop).is_ok());assert!(stop.load(Ordering::SeqCst));
        assert!(queued.claim(&stop).is_err());
        assert_eq!(gate.complete(command.id,true).unwrap(),Outcome::AwaitingObservation);
        assert!(gate.confirm(&t,Operation::Stop,Duration::from_secs(5)).is_err());
    }
    #[test] fn queued_confirmation_expiry_prevents_worker_claim() {
        let gate=Gate::default();let t=target();gate.publish(Some(t.clone()),true).unwrap();
        let command=gate.confirm(&t,Operation::Stop,Duration::from_secs(5)).unwrap();gate.enqueue(&command).unwrap();
        gate.state.lock().unwrap().entry.as_mut().unwrap().expires=Instant::now();
        assert_eq!(gate.claim(command.id).unwrap_err(),"SERIAL.GATE_CONFIRMATION_EXPIRED");
        assert_eq!(gate.cancel(command.id).unwrap(),Outcome::CancelledBeforeClaim);
    }
    #[test] fn changed_service_configuration_revokes_worker_claim_even_after_restore() {
        use wiparse_core::ai::{ReviewedSource,source_lease::SourceLease,AiConfig,AiService};
        let source=SourceLease::default();let authority=SourceLease::default();
        let pair=ReviewedSource::capture(&source,||serde_json::json!({})).unwrap()
            .join(ReviewedSource::capture(&authority,||serde_json::json!({})).unwrap());
        let gate=Gate::default();let t=target();gate.publish(Some(t.clone()),true).unwrap();
        let command=gate.confirm(&t,Operation::Stop,Duration::from_secs(5)).unwrap();gate.enqueue(&command).unwrap();
        let original=AiConfig::default();let service=AiService::new(original.clone());let mut changed=original.clone();changed.model="another".into();
        let (reply,_)=crossbeam_channel::bounded(1);
        let queued=ReviewedStop{gate,command:command.clone(),sources:pair,service:service.clone(),service_revision:0,reply,
            slot:Arc::new(Mutex::new(None)),intent:super::super::SerialStopIntent{command_id:command.id.to_string(),project:"p".into(),fingerprint:"f".into(),audit:None}};
        service.configure(changed);service.configure(original);
        let stop=AtomicBool::new(false);assert!(queued.claim(&stop).is_err());assert!(!stop.load(Ordering::SeqCst));
    }
}
