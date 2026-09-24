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
    subscribers: Mutex<Vec<Sender<String>>>,
    history: Mutex<VecDeque<(u64, String)>>,
}

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
            history.push_back((seq, line.clone()));
            while history.len() > HISTORY_CAPACITY {
                history.pop_front();
            }
        }
        if let Ok(mut subs) = self.inner.subscribers.lock() {
            // A slow subscriber is disconnected when its bounded queue fills.
            subs.retain(|tx| match tx.try_send(line.clone()) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
            });
        }
    }

    pub fn subscribe(&self, since_seq: u64) -> Result<Receiver<String>, &'static str> {
        let (tx, rx) = bounded(SUBSCRIBER_CAPACITY);
        if let Ok(mut subs) = self.inner.subscribers.lock() {
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
                            .filter(|(n, _)| *n > since_seq)
                            .map(|(_, s)| s.clone())
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
            subs.push(tx);
            return Ok(rx);
        }
        Err("event bus unavailable")
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
    fn enforces_subscriber_limit() {
        let bus = EventBus::new();
        let subscribers: Vec<_> = (0..MAX_SUBSCRIBERS)
            .map(|_| bus.subscribe(0).expect("within limit"))
            .collect();
        assert!(bus.subscribe(0).is_err());
        drop(subscribers);
    }
}
