use crossbeam_channel::{unbounded, Receiver, Sender};
use serde_json::{json, Value};
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
}

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
        if let Ok(mut subs) = self.inner.subscribers.lock() {
            subs.retain(|tx| tx.send(line.clone()).is_ok());
        }
    }

    pub fn subscribe(&self, since_seq: u64) -> Receiver<String> {
        let (tx, rx) = unbounded();
        let _ = since_seq; // backlog not retained; clients start from live stream
        if let Ok(mut subs) = self.inner.subscribers.lock() {
            // greet with current seq so clients can sync
            let seq = self.inner.seq.load(Ordering::SeqCst);
            let hello = json!({
                "type": "subscribed",
                "seq": seq,
                "ts": chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                "data": { "since_seq": since_seq }
            });
            let _ = tx.send(hello.to_string());
            subs.push(tx);
        }
        rx
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
