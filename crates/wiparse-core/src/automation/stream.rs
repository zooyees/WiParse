use super::contracts::Quality;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Normalized events. This is NOT a guessed Dongle binary ICD.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventRecord {
    pub source: String,
    pub boot_id: String,
    pub capture_session_id: String,
    pub stream_id: String,
    pub seq: String,
    pub device_tick: String,
    pub tick_hz: u64,
    pub event_type: String,
    pub payload: Value,
    pub quality: Quality,
    #[serde(default)]
    pub host_monotonic_ns: Option<String>,
    #[serde(default)]
    pub utc: Option<String>,
    #[serde(default = "event_schema")]
    pub schema_version: String,
}
fn event_schema() -> String {
    super::contracts::SCHEMA_VERSION.into()
}
#[derive(Default)]
pub struct StreamValidator {
    last: HashMap<(String, String), (String, String, u64, u64, u64)>,
}
impl StreamValidator {
    pub fn ingest(&mut self, e: &mut EventRecord) -> Result<(), String> {
        if e.source.is_empty()
            || e.boot_id.is_empty()
            || e.capture_session_id.is_empty()
            || e.stream_id.is_empty()
            || e.tick_hz == 0
        {
            return Err("STREAM.IDENTITY_REQUIRED".into());
        }
        let seq = e
            .seq
            .parse::<u64>()
            .map_err(|_| "STREAM.INVALID_SEQUENCE")?;
        let tick = e
            .device_tick
            .parse::<u64>()
            .map_err(|_| "STREAM.INVALID_TICK")?;
        if e.schema_version != super::contracts::SCHEMA_VERSION || e.event_type.is_empty() {
            return Err("STREAM.SCHEMA_INVALID".into());
        }
        if e.host_monotonic_ns
            .as_ref()
            .is_some_and(|n| n.parse::<u64>().is_err())
        {
            return Err("STREAM.INVALID_HOST_TICK".into());
        }
        let key = (e.source.clone(), e.stream_id.clone());
        if self.last.len() >= 128 && !self.last.contains_key(&key) {
            return Err("STREAM.TOO_MANY_STREAMS".into());
        }
        if let Some((boot, session, old_seq, old_tick, hz)) = self.last.get(&key) {
            if boot != &e.boot_id || session != &e.capture_session_id {
                e.quality.flag("STREAM.SESSION_BREAK");
            } else {
                if old_seq.checked_add(1) != Some(seq) {
                    e.quality.flag("STREAM.SEQUENCE_GAP_OR_DUPLICATE");
                }
                if tick < *old_tick {
                    e.quality.flag("STREAM.CLOCK_REVERSED");
                }
                if e.tick_hz != *hz {
                    e.quality.flag("STREAM.CLOCK_RATE_CHANGED");
                }
            }
        }
        self.last.insert(
            key,
            (
                e.boot_id.clone(),
                e.capture_session_id.clone(),
                seq,
                tick,
                e.tick_hz,
            ),
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_ticks_session_and_loss_are_visible() {
        let mut v = StreamValidator::default();
        let mut e = EventRecord {
            source: "dongle".into(),
            boot_id: "b".into(),
            capture_session_id: "s".into(),
            stream_id: "io".into(),
            seq: "1".into(),
            device_tick: "9007199254740993".into(),
            tick_hz: 1_000_000,
            event_type: "edge".into(),
            payload: Value::Null,
            quality: Quality::default(),
            host_monotonic_ns: None,
            utc: None,
            schema_version: event_schema(),
        };
        v.ingest(&mut e).unwrap();
        assert!(e.quality.valid());
        e.seq = "3".into();
        v.ingest(&mut e).unwrap();
        assert!(!e.quality.valid());
        e.quality = Quality::default();
        e.boot_id = "new".into();
        v.ingest(&mut e).unwrap();
        assert!(e.quality.issues.contains(&"STREAM.SESSION_BREAK".into()));
    }
}
