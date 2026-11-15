//! Internal building blocks for a future station-owned wireless execution gate.
//!
//! This module is not connected to the service API, a TX source, or a hardware
//! executor. In particular, it does not authorize or enable live operation.
//! Only a station-owned reader may submit observations after it has established
//! their origin and continuity. A client-supplied readiness flag is insufficient.

use std::time::{Duration, Instant};
use std::collections::HashSet;
use serde::Deserialize;

/// Every component is established by the station, not accepted as proof from a client.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Identity {
    pub station: String,
    pub case: String,
    pub run: String,
    pub attempt: String,
    pub capture: String,
    pub device: String,
    pub lease_epoch: String,
    pub config_sha256: String,
}

impl Identity {
    fn valid(&self) -> bool {
        [&self.station, &self.case, &self.run, &self.attempt, &self.capture,
         &self.device, &self.lease_epoch].iter().all(|s| !s.trim().is_empty() && s.len() <= 256)
            && self.config_sha256.len() == 64
            && self.config_sha256.bytes().all(|c| c.is_ascii_hexdigit())
    }
}

/// Product-specific rules must come from an approved, versioned station profile.
/// No default rule is provided and this type is not deserialized from API input.
#[derive(Clone, Debug)]
pub(crate) struct ReadinessRule {
    pub version: String,
    pub literal: String,
    /// Approved indications that readiness is lost; fail closed if unknown.
    pub invalidating_literals: Vec<String>,
    pub valid_for: Duration,
}

/// Schema for an approved station-owned profile, not a public API request.
/// Loading this schema alone does not approve a profile or enable live mode.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleConfig {
    pub version: String,
    pub ready_literal: String,
    pub invalidating_literals: Vec<String>,
    pub valid_for_ms: u64,
}

impl TryFrom<RuleConfig> for ReadinessRule {
    type Error = GateError;
    fn try_from(config: RuleConfig) -> Result<Self, Self::Error> {
        let rule = Self { version: config.version, literal: config.ready_literal,
            invalidating_literals: config.invalidating_literals,
            valid_for: Duration::from_millis(config.valid_for_ms) };
        if rule.valid() { Ok(rule) } else { Err(GateError::RuleUnavailable) }
    }
}

impl ReadinessRule {
    fn valid(&self) -> bool {
        !self.version.trim().is_empty() && self.version.len() <= 128
            && !self.literal.trim().is_empty() && self.literal.len() <= 1024
            && !self.invalidating_literals.is_empty() && self.invalidating_literals.len() <= 32
            && self.invalidating_literals.iter().all(|s| !s.trim().is_empty() && s.len() <= 1024)
            && self.valid_for > Duration::ZERO && self.valid_for <= Duration::from_secs(3600)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GateError {
    InvalidIdentity,
    RuleUnavailable,
    IdentityChanged,
    CaptureDiscontinuous,
    NotReady,
    Expired,
    Cancelled,
    Unsafe,
    OutcomeUnknown,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DispatchError<E> {
    Gate(GateError),
    InvalidCommand,
    DuplicateCommand,
    CommandBudget,
    OutcomeUnknown(E),
}

/// A readiness match records its source location without persisting raw logs.
#[derive(Clone, Debug)]
pub(crate) struct Evidence {
    pub rule_version: String,
    pub sequence: u64,
    pub observed_at: Instant,
    pub expires_at: Instant,
}

/// One instance belongs to exactly one live capture attempt. Invalidated
/// attempts cannot be rearmed: a new attempt must allocate a fresh instance.
pub(crate) struct Gate {
    identity: Identity,
    rule: ReadinessRule,
    next_sequence: u64,
    evidence: Option<Evidence>,
    invalid: Option<GateError>,
    dispatched: HashSet<String>,
}

impl Gate {
    pub(crate) fn new(identity: Identity, rule: Option<ReadinessRule>, first_sequence: u64)
        -> Result<Self, GateError> {
        if !identity.valid() { return Err(GateError::InvalidIdentity); }
        let rule = rule.filter(ReadinessRule::valid).ok_or(GateError::RuleUnavailable)?;
        Ok(Self { identity, rule, next_sequence: first_sequence, evidence: None, invalid: None,
            dispatched: HashSet::new() })
    }

    /// Called by the trusted reader for *every* complete line in capture order.
    /// A gap, malformed line, overflow or wrong capture permanently invalidates
    /// the attempt; partial lines must not be sent as complete observations.
    pub(crate) fn observe_line(&mut self, identity: &Identity, sequence: u64,
        line: &str, complete: bool, now: Instant) -> Result<bool, GateError> {
        self.current(identity)?;
        if !complete || sequence != self.next_sequence || line.len() > 64 * 1024 {
            self.invalidate(GateError::CaptureDiscontinuous);
            return Err(GateError::CaptureDiscontinuous);
        }
        self.next_sequence = match sequence.checked_add(1) {
            Some(next) => next,
            None => { self.invalidate(GateError::CaptureDiscontinuous); return Err(GateError::CaptureDiscontinuous); }
        };
        // Exact complete-line matching avoids treating e.g. NOT_READY as READY.
        // Richer product-specific parsers require separately reviewed rules.
        let line = line.trim_end_matches(['\r', '\n']);
        if self.rule.invalidating_literals.iter().any(|term| line == term) {
            self.invalidate(GateError::NotReady);
            return Err(GateError::NotReady);
        }
        if line == self.rule.literal {
            let Some(expires_at) = now.checked_add(self.rule.valid_for) else {
                self.invalidate(GateError::CaptureDiscontinuous);
                return Err(GateError::CaptureDiscontinuous);
            };
            self.evidence = Some(Evidence { rule_version: self.rule.version.clone(), sequence,
                observed_at: now, expires_at });
            return Ok(true);
        }
        Ok(false)
    }

    pub(crate) fn invalidate(&mut self, reason: GateError) {
        if self.invalid.is_none() { self.invalid = Some(reason); }
        self.evidence = None;
    }

    fn current(&mut self, identity: &Identity) -> Result<(), GateError> {
        if let Some(reason) = &self.invalid { return Err(reason.clone()); }
        if identity != &self.identity {
            self.invalidate(GateError::IdentityChanged);
            return Err(GateError::IdentityChanged);
        }
        Ok(())
    }

    /// Check immediately at the station's physical I²C dispatch boundary.
    /// The eventual owner must serialize this check and dispatch with capture
    /// invalidation, lease expiry, cancellation, and device-session changes.
    /// This method by itself is not an atomic hardware gate.
    pub(crate) fn check(&mut self, identity: &Identity, now: Instant,
        lease_valid: bool, cancelled: bool, protection_healthy: bool) -> Result<&Evidence, GateError> {
        self.current(identity)?;
        if cancelled { self.invalidate(GateError::Cancelled); return Err(GateError::Cancelled); }
        if !lease_valid { self.invalidate(GateError::IdentityChanged); return Err(GateError::IdentityChanged); }
        if !protection_healthy { self.invalidate(GateError::Unsafe); return Err(GateError::Unsafe); }
        let Some(evidence) = &self.evidence else { return Err(GateError::NotReady); };
        if now < evidence.observed_at || now >= evidence.expires_at {
            self.evidence = None;
            return Err(GateError::Expired);
        }
        Ok(self.evidence.as_ref().expect("evidence checked above"))
    }

    /// Serializes checking and a single dispatch under the caller's exclusive
    /// ownership of this gate. Once dispatch has started, any error is treated
    /// as an unknown outcome and permanently blocks this attempt. Persistence,
    /// capture cancellation and device-session synchronization still belong to
    /// the eventual station executor; this in-memory helper is not that executor.
    pub(crate) fn dispatch_once<T, E>(&mut self, identity: &Identity, now: Instant,
        lease_valid: bool, cancelled: bool, protection_healthy: bool, command_id: &str,
        dispatch: impl FnOnce() -> Result<T, E>) -> Result<T, DispatchError<E>> {
        if command_id.trim().is_empty() || command_id.len() > 128 {
            return Err(DispatchError::InvalidCommand);
        }
        self.check(identity, now, lease_valid, cancelled, protection_healthy)
            .map_err(DispatchError::Gate)?;
        if self.dispatched.contains(command_id) { return Err(DispatchError::DuplicateCommand); }
        if self.dispatched.len() >= 1024 { return Err(DispatchError::CommandBudget); }
        self.dispatched.insert(command_id.to_owned());
        match dispatch() {
            Ok(value) => Ok(value),
            Err(error) => {
                self.invalidate(GateError::OutcomeUnknown);
                Err(DispatchError::OutcomeUnknown(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Identity {
        Identity { station: "station".into(), case: "case".into(), run: "run".into(),
            attempt: "attempt".into(), capture: "capture".into(), device: "device".into(),
            lease_epoch: "epoch".into(), config_sha256: "a".repeat(64) }
    }
    fn rule() -> ReadinessRule {
        ReadinessRule { version: "fixture-v1".into(), literal: "READY".into(),
            invalidating_literals: vec!["STOPPED".into()], valid_for: Duration::from_secs(2) }
    }

    #[test]
    fn no_rule_or_missing_withdrawal_cannot_arm_gate() {
        assert!(matches!(Gate::new(identity(), None, 0), Err(GateError::RuleUnavailable)));
        let mut incomplete = rule(); incomplete.invalidating_literals.clear();
        assert!(matches!(Gate::new(identity(), Some(incomplete), 0), Err(GateError::RuleUnavailable)));
    }

    #[test]
    fn rule_config_requires_both_indications_and_bounded_duration() {
        let valid = r#"{"version":"v1","ready_literal":"READY","invalidating_literals":["STOPPED"],"valid_for_ms":500}"#;
        let config: RuleConfig = serde_json::from_str(valid).unwrap();
        assert!(ReadinessRule::try_from(config).is_ok());
        let missing = r#"{"version":"v1","ready_literal":"READY","valid_for_ms":500}"#;
        assert!(serde_json::from_str::<RuleConfig>(missing).is_err());
        let extra = r#"{"version":"v1","ready_literal":"READY","invalidating_literals":["STOPPED"],"valid_for_ms":500,"approved":true}"#;
        assert!(serde_json::from_str::<RuleConfig>(extra).is_err());
    }

    #[test]
    fn evidence_is_bound_to_attempt_and_expires() {
        let id = identity(); let now = Instant::now();
        let mut gate = Gate::new(id.clone(), Some(rule()), 7).unwrap();
        assert_eq!(gate.check(&id, now, true, false, true).unwrap_err(), GateError::NotReady);
        assert!(gate.observe_line(&id, 7, "READY", true, now).unwrap());
        assert_eq!(gate.check(&id, now, true, false, true).unwrap().sequence, 7);
        assert_eq!(gate.check(&id, now + Duration::from_secs(2), true, false, true).unwrap_err(), GateError::Expired);
        let mut other = id.clone(); other.attempt = "other".into();
        assert_eq!(gate.check(&other, now, true, false, true).unwrap_err(), GateError::IdentityChanged);
        assert_eq!(gate.check(&id, now, true, false, true).unwrap_err(), GateError::IdentityChanged);
    }

    #[test]
    fn unrelated_line_containing_ready_is_not_readiness_evidence() {
        let id = identity(); let now = Instant::now();
        let mut gate = Gate::new(id.clone(), Some(rule()), 0).unwrap();
        assert!(!gate.observe_line(&id, 0, "NOT_READY", true, now).unwrap());
        assert_eq!(gate.check(&id, now, true, false, true).unwrap_err(), GateError::NotReady);
    }

    #[test]
    fn discontinuity_cancellation_or_lost_readiness_cannot_rearm_attempt() {
        let id = identity(); let now = Instant::now();
        for failure in ["gap", "cancel", "stop", "protection"] {
            let mut gate = Gate::new(id.clone(), Some(rule()), 0).unwrap();
            gate.observe_line(&id, 0, "READY", true, now).unwrap();
            match failure {
                "gap" => { assert_eq!(gate.observe_line(&id, 2, "READY", true, now).unwrap_err(), GateError::CaptureDiscontinuous); },
                "cancel" => { assert_eq!(gate.check(&id, now, true, true, true).unwrap_err(), GateError::Cancelled); },
                "stop" => { assert_eq!(gate.observe_line(&id, 1, "STOPPED", true, now).unwrap_err(), GateError::NotReady); },
                _ => { assert_eq!(gate.check(&id, now, true, false, false).unwrap_err(), GateError::Unsafe); },
            }
            assert!(gate.observe_line(&id, 1, "READY", true, now).is_err());
        }
    }

    #[test]
    fn unknown_dispatch_cannot_be_retried_in_the_same_attempt() {
        let id = identity(); let now = Instant::now();
        let mut gate = Gate::new(id.clone(), Some(rule()), 0).unwrap();
        gate.observe_line(&id, 0, "READY", true, now).unwrap();
        let mut calls = 0;
        assert_eq!(gate.dispatch_once(&id, now, true, false, true, "op-1", || {
            calls += 1; Err::<(), _>("lost response")
        }), Err(DispatchError::OutcomeUnknown("lost response")));
        assert_eq!(gate.dispatch_once(&id, now, true, false, true, "op-1", || {
            calls += 1; Ok::<_, &str>(())
        }), Err(DispatchError::Gate(GateError::OutcomeUnknown)));
        assert_eq!(calls, 1);
    }
}
