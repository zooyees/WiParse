use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct EventBus {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    seq: AtomicU64,
    subscribers: Mutex<Vec<Subscriber>>,
    history: Mutex<VecDeque<(u64, String, String)>>,
}

struct Subscriber {tx:Sender<String>,kind:Option<&'static str>}

const MAX_SUBSCRIBERS: usize = 32;
const SUBSCRIBER_CAPACITY: usize = 1024;
const HISTORY_CAPACITY: usize = 512;

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish(&self, event_type: &str, data: Value, session_id: Option<String>) {
        let seq = self.inner.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let envelope = EventEnvelope {
            event_type: event_type.into(),
            seq,
            ts: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            session_id,
            data,
        };
        let line = match serde_json::to_string(&envelope) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Ok(mut history) = self.inner.history.lock() {
            history.push_back((seq, event_type.into(), line.clone()));
            while history.len() > HISTORY_CAPACITY {
                history.pop_front();
            }
        }
        if let Ok(mut subs) = self.inner.subscribers.lock() {
            // A slow subscriber is disconnected when its bounded queue fills.
            subs.retain(|sub| {if sub.kind.is_some_and(|kind|kind!=event_type){return true;}match sub.tx.try_send(line.clone()) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
            }});
        }
    }

    pub fn subscribe(&self, since_seq: u64) -> Result<Receiver<String>, &'static str> {self.subscribe_kind(since_seq,None)}
    pub fn subscribe_serial_logs(&self,since_seq:u64)->Result<Receiver<String>,&'static str>{self.subscribe_kind(since_seq,Some("serial.log.changed"))}
    fn subscribe_kind(&self, since_seq:u64,kind:Option<&'static str>)->Result<Receiver<String>,&'static str>{
        let (tx, rx) = bounded(SUBSCRIBER_CAPACITY);
        if let Ok(mut subs) = self.inner.subscribers.lock() {
            // Reap closed idle streams before applying the subscriber limit.
            // Pings are unsequenced control messages, as in the HTTP keepalive.
            subs.retain(|sub|sub.tx.try_send(json!({"type":"ping"}).to_string()).is_ok());
            if subs.len() >= MAX_SUBSCRIBERS {
                return Err("too many event subscribers");
            }
            // greet with current seq so clients can sync
            let seq = self.inner.seq.load(Ordering::SeqCst);
            let (oldest, replay): (Option<u64>, Vec<String>) = self
                .inner
                .history
                .lock()
                .ok()
                .map(|h| {
                    (
                        h.front().map(|x| x.0),
                        h.iter()
                            .filter(|(n, event_type, _)| *n > since_seq && kind.is_none_or(|kind|kind==event_type))
                            .map(|(_, _, s)| s.clone())
                            .collect(),
                    )
                })
                .unwrap_or_default();
            let gap = since_seq > 0 && oldest.is_some_and(|first| since_seq + 1 < first);
            let hello = json!({
                "type": "subscribed",
                "seq": seq,
                "ts": chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                "data": { "since_seq": since_seq, "replayed": replay.len(), "gap": gap, "oldest_seq": oldest }
            });
            let _ = tx.try_send(hello.to_string());
            for line in replay {
                let _ = tx.try_send(line);
            }
            subs.push(Subscriber{tx,kind});
            return Ok(rx);
        }
        Err("event bus unavailable")
    }

    pub fn has_serial_log_subscribers(&self)->bool{
        self.inner.subscribers.lock().map(|subs|subs.iter().any(|sub|sub.kind==Some("serial.log.changed"))).unwrap_or(false)
    }
    pub fn next_seq(&self) -> u64 {
        self.inner.seq.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EventEnvelope {
    #[serde(rename = "type")]
    pub event_type: String,
    pub seq: u64,
    pub ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub data: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_subscription_ignores_line_payloads_and_other_device_events(){
        let bus=EventBus::new();bus.publish("serial.line",json!({"line":"older"}),None);bus.publish("serial.log.changed",json!({}),None);
        assert!(!bus.has_serial_log_subscribers());
        let rx=bus.subscribe_serial_logs(0).unwrap();assert!(bus.has_serial_log_subscribers());let hello:Value=serde_json::from_str(&rx.recv().unwrap()).unwrap();assert_eq!(hello["data"]["replayed"],1);
        assert!(rx.recv().unwrap().contains("serial.log.changed"));assert!(rx.try_recv().is_err());
        for _ in 0..2000{bus.publish("serial.line",json!({"line":"not for this consumer"}),None);}
        assert!(rx.try_recv().is_err());bus.publish("serial.log.changed",json!({}),None);assert!(rx.recv().unwrap().contains("serial.log.changed"));
        drop(rx);bus.publish("serial.log.changed",json!({}),None);assert!(!bus.has_serial_log_subscribers());
    }

    #[test]
    fn replays_events_after_requested_sequence() {
        let bus = EventBus::new();
        bus.publish("one", json!({}), None);
        bus.publish("two", json!({}), None);

        let rx = bus.subscribe(1).expect("subscribe");
        let hello: Value = serde_json::from_str(&rx.recv().expect("hello")).expect("json");
        assert_eq!(hello["data"]["gap"], false);
        assert_eq!(hello["data"]["replayed"], 1);
        let replay: Value = serde_json::from_str(&rx.recv().expect("replay")).expect("json");
        assert_eq!(replay["type"], "two");
        assert_eq!(replay["seq"], 2);
    }

    #[test]
    fn reports_gap_when_requested_history_was_evicted() {
        let bus = EventBus::new();
        for n in 0..(HISTORY_CAPACITY + 2) {
            bus.publish("sample", json!({ "n": n }), None);
        }

        let rx = bus.subscribe(1).expect("subscribe");
        let hello: Value = serde_json::from_str(&rx.recv().expect("hello")).expect("json");
        assert_eq!(hello["data"]["gap"], true);
        assert_eq!(hello["data"]["oldest_seq"], 3);
        assert_eq!(hello["data"]["replayed"], HISTORY_CAPACITY);
    }

    #[test]
    fn closed_idle_filtered_subscriptions_do_not_exhaust_capacity(){
        let bus=EventBus::new();for _ in 0..MAX_SUBSCRIBERS*3{let rx=bus.subscribe_serial_logs(0).unwrap();drop(rx);}
        assert!(bus.subscribe_serial_logs(0).is_ok());
    }

    #[test]
    fn enforces_subscriber_limit() {
        let bus = EventBus::new();
        let subscribers: Vec<_> = (0..MAX_SUBSCRIBERS)
            .map(|_| bus.subscribe(0).expect("within limit"))
            .collect();
        assert!(bus.subscribe(0).is_err());
        drop(subscribers);
    }
}
