//! Host-owned lifecycle of one tool call, shared by the panel and serial bridge.
use super::agent::{failure, HostReply};
use serde_json::Value;
use std::time::{Duration, Instant};
use wiparse_core::engineering_agent::Call;

pub(crate) fn timeout(method: &str) -> Duration {
    Duration::from_secs(match method {
        "testing.wait" => 600,
        "serial.start" | "serial.monitor.start" => 15,
        _ => 90,
    })
}

pub(crate) struct HostItem {
    receiver: Option<crossbeam_channel::Receiver<Value>>,
    pub continuation: Option<Call>,
    next: Instant,
}

impl Default for HostItem {
    fn default() -> Self {
        Self {
            receiver: None,
            continuation: None,
            next: Instant::now(),
        }
    }
}

impl HostItem {
    pub fn receiving(&self) -> bool {
        self.receiver.is_some()
    }

    pub fn attach(&mut self, reply: HostReply) -> Option<Value> {
        self.receiver = reply.receiver;
        self.receiver.is_none().then_some(reply.value)
    }

    pub fn receive(&mut self) -> Option<Value> {
        let result = match self.receiver.as_ref()?.try_recv() {
            Ok(value) => value,
            Err(crossbeam_channel::TryRecvError::Empty) => return None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                failure("AGENT.WORKER_DISCONNECTED")
            }
        };
        self.receiver = None;
        Some(result)
    }

    /// A status read may be repeated. An acknowledgement can never replay the original write.
    pub fn advance(&mut self, value: Value, current: &Call, internal: bool) -> Option<Value> {
        let delay;
        if let Some(wait) = value.get("_agent_wait") {
            let method = wait["method"].as_str().unwrap_or("");
            if !matches!(
                method,
                "serial.ready"
                    | "testing.result"
                    | "wave.decode.result"
                    | "data.statistics.result"
                    | "instrument.result"
            ) || !wait["params"].is_object()
            {
                return Some(failure("AI.INVALID_HOST_CONTINUATION"));
            }
            self.continuation = Some(Call {
                method: method.into(),
                params: wait["params"].clone(),
            });
            delay = wait["interval_ms"].as_u64().unwrap_or(250).clamp(16, 1000);
        } else if value["state"] == "pending" || value["data"]["state"] == "pending_or_unknown" {
            if !internal {
                return Some(failure("AI.MISSING_STATUS_CONTINUATION: outcome unknown"));
            }
            self.continuation = Some(current.clone());
            delay = 250;
        } else {
            return Some(value);
        }
        self.next = Instant::now() + Duration::from_millis(delay);
        None
    }

    pub fn take_due(&mut self, now: Instant) -> Option<Call> {
        if now < self.next || self.receiving() {
            return None;
        }
        self.continuation.take()
    }
}

/// Repairs only rejected model turns. Tool receipts reset the consecutive error budget.
#[derive(Default)]
pub(crate) struct ProtocolRecovery {
    consecutive: u8,
    pub json_fallback: bool,
}

impl ProtocolRecovery {
    pub fn made_progress(&mut self) {
        self.consecutive = 0;
    }
    pub fn retry(&mut self, native: bool, reason: &str) -> bool {
        if self.consecutive >= 3 {
            return false;
        }
        self.consecutive += 1;
        if native && self.consecutive >= 2 && !reason.starts_with("AGENT.UNFINISHED_OUTCOMES") {
            self.json_fallback = true;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn immediate_and_worker_replies_have_the_same_lifecycle() {
        for asynchronous in [false, true] {
            let mut item = HostItem::default();
            let write = Call {
                method: "instrument.command".into(),
                params: json!({}),
            };
            let ack = json!({"_agent_wait":{"method":"instrument.result","params":{"job_id":7},"interval_ms":16}});
            let reply = if asynchronous {
                let (tx, rx) = crossbeam_channel::bounded(1);
                tx.send(ack.clone()).unwrap();
                HostReply::work(rx)
            } else {
                HostReply::value(ack)
            };
            let value = item.attach(reply).or_else(|| item.receive()).unwrap();
            assert!(item.advance(value, &write, false).is_none());
            let query = item
                .take_due(Instant::now() + Duration::from_secs(1))
                .unwrap();
            assert_eq!(query.method, "instrument.result");
            assert!(item
                .advance(json!({"state":"pending"}), &query, true)
                .is_none());
            assert_eq!(
                item.take_due(Instant::now() + Duration::from_secs(1)),
                Some(query.clone())
            );
            let completed = json!({"ok":true,"data":{"physical_verified":false}});
            assert_eq!(
                item.advance(completed.clone(), &query, true),
                Some(completed)
            );
            assert_eq!(
                item.advance(json!({"state":"pending"}), &write, false)
                    .unwrap()["ok"],
                false
            );
        }
    }
}
