use crate::storage::{atomic_json, file_hash, safe_input};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use wiparse_core::automation::{
    analysis::{analyze, deadtime, AnalysisOptions, TimingProvenance},
    contracts::{evaluate, Limits, Measurement, Quality},
    stream::{EventRecord, StreamValidator},
};
#[derive(Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub command_id: String,
    pub method: String,
    pub params: Value,
    pub state: String,
    pub result: Value,
    pub error: Option<String>,
    #[serde(default)]
    pub cancel_requested: bool,
}
struct Lease {
    token: String,
    epoch: String,
    expires: Instant,
}
pub struct State {
    root: PathBuf,
    jobs: HashMap<String, Job>,
    cancel: HashMap<String, Arc<AtomicBool>>,
    leases: HashMap<String, Lease>,
    epoch: String,
    storage_fault: AtomicBool,
}
impl State {
    pub fn idle(&self) -> bool {
        self.cancel.is_empty()
    }
    pub fn health(&self) -> Value {
        json!({"active_jobs":self.cancel.len(),"storage_fault":self.storage_fault.load(Ordering::SeqCst),"accepting_jobs":!self.storage_fault.load(Ordering::SeqCst)})
    }
    pub fn open(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(root.join("projects")).map_err(|e| e.to_string())?;
        fs::create_dir_all(root.join("jobs")).map_err(|e| e.to_string())?;
        let p = root.join("jobs.json");
        let source = if p.exists() {
            p.clone()
        } else {
            p.with_extension("json.bak")
        };
        let mut jobs: HashMap<String, Job> = if source.exists() {
            serde_json::from_slice(&fs::read(source).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        } else {
            HashMap::new()
        };
        for j in jobs.values_mut() {
            if matches!(j.state.as_str(), "queued" | "running" | "cancelling") {
                j.state = "unknown".into();
                j.error = Some("SERVICE.INTERRUPTED".into());
            }
        }
        let s = Self {
            root,
            jobs,
            cancel: HashMap::new(),
            leases: HashMap::new(),
            epoch: uuid::Uuid::new_v4().to_string(),
            storage_fault: AtomicBool::new(false),
        };
        s.persist()?;
        Ok(s)
    }
    fn persist(&self) -> Result<(), String> {
        let result = atomic_json(
            &self.root.join("jobs.json"),
            &serde_json::to_value(&self.jobs).map_err(|e| e.to_string())?,
        );
        if result.is_err() {
            self.storage_fault.store(true, Ordering::SeqCst);
        }
        result
    }
}
fn string<'a>(p: &'a Value, key: &str) -> Result<&'a str, String> {
    p[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("API.MISSING_{key}"))
}
pub fn invoke(shared: &Arc<Mutex<State>>, v: &Value) -> Result<Value, String> {
    let method = string(v, "method")?;
    let p = v.get("params").cloned().unwrap_or(json!({}));
    if !p.is_object() {
        return Err("API.PARAMS_OBJECT_REQUIRED".into());
    }
    if shared
        .lock()
        .map_err(|_| "SERVICE.LOCK")?
        .storage_fault
        .load(Ordering::SeqCst)
        && !matches!(
            method,
            "job.get"
                | "job.cancel"
                | "system.capabilities"
                | "device.capabilities"
                | "check.evaluate"
                | "time.mapping.fit"
                | "time.mapping.convert"
        )
    {
        return Err("DATA.STORAGE_FAULT_LATCHED_RESTART_REQUIRED".into());
    }
    match method {
        "time.mapping.fit" => {
            let request = serde_json::from_value(p).map_err(|e| e.to_string())?;
            return serde_json::to_value(wiparse_core::automation::time::fit(&request)?)
                .map_err(|e| e.to_string());
        }
        "time.mapping.convert" => {
            let mapping =
                serde_json::from_value(p["mapping"].clone()).map_err(|e| e.to_string())?;
            let value = wiparse_core::automation::time::convert(
                &mapping,
                string(&p, "tick")?,
                string(&p, "boot_id")?,
                string(&p, "capture_session_id")?,
            )?;
            return Ok(
                json!({"reference_s":value,"uncertainty_s":mapping.uncertainty_s,"quality":mapping.quality}),
            );
        }
        "system.capabilities" => {
            return Ok(
                json!({"schema_version":"wiparse.automation/1","implemented":["wave.analyze","protocol.decode","check.evaluate","stream.validate","uart.capture","device.capabilities","device.lease.acquire","device.lease.release","job.get","job.cancel","time.mapping.fit","time.mapping.convert"],"industrial_approved":false,"unsupported":["dongle.capture.binary","instrument.output.control","scope.armed.verified","protocol.streaming"],"protocols":["uart","i2c","spi","i2s","ddsss","usb_pd","scp","ufcs"],"protocol_mode":"bounded_offline","legacy_instrument_owner":"GUI; not migrated to service"}),
            )
        }
        "device.capabilities" => {
            let ports = serialport::available_ports().map_err(|e| e.to_string())?;
            return Ok(
                json!({"devices":ports.into_iter().map(|port|json!({"resource":port.port_name,"identity":format!("{:?}",port.port_type),"capabilities":{"uart_raw":"experimental","io_timestamp":"unsupported","output_control":"unsupported"},"verified":false})).collect::<Vec<_>>(),"industrial_approved":false}),
            );
        }
        "check.evaluate" => {
            let m: Measurement =
                serde_json::from_value(p["measurement"].clone()).map_err(|e| e.to_string())?;
            let l: Limits =
                serde_json::from_value(p["limits"].clone()).map_err(|e| e.to_string())?;
            return Ok(serde_json::to_value(evaluate(&m, &l)).unwrap());
        }
        "job.get" => {
            let s = shared.lock().map_err(|_| "SERVICE.LOCK")?;
            return s
                .jobs
                .get(string(&p, "job_id")?)
                .map(|j| serde_json::to_value(j).unwrap())
                .ok_or("JOB.NOT_FOUND".into());
        }
        "job.cancel" => {
            let mut s = shared.lock().map_err(|_| "SERVICE.LOCK")?;
            let id = string(&p, "job_id")?;
            if !s.jobs.contains_key(id) {
                return Err("JOB.NOT_FOUND".into());
            }
            if let Some(c) = s.cancel.get(id) {
                c.store(true, Ordering::SeqCst);
            } else {
                return Ok(json!({"job_id":id,"state":s.jobs[id].state}));
            }
            s.jobs.get_mut(id).unwrap().cancel_requested = true;
            s.persist()?;
            return Ok(json!({"job_id":id,"state":s.jobs[id].state,"cancel_requested":true}));
        }
        "device.lease.acquire" => {
            let mut s = shared.lock().map_err(|_| "SERVICE.LOCK")?;
            let resource = string(&p, "resource")?.to_string();
            if s.leases
                .get(&resource)
                .is_some_and(|l| l.expires > Instant::now())
                || s.jobs.values().any(|j| {
                    matches!(j.state.as_str(), "queued" | "running" | "cancelling")
                        && j.params["resource"] == resource
                })
            {
                return Err("DEVICE.BUSY".into());
            }
            let ttl = p["ttl_s"].as_u64().unwrap_or(60);
            if !(1..=3600).contains(&ttl) {
                return Err("LEASE.INVALID_TTL".into());
            }
            let token = uuid::Uuid::new_v4().to_string();
            let epoch = s.epoch.clone();
            s.leases.insert(
                resource.clone(),
                Lease {
                    token: token.clone(),
                    epoch: epoch.clone(),
                    expires: Instant::now() + Duration::from_secs(ttl),
                },
            );
            return Ok(
                json!({"resource":resource,"lease_token":token,"session_epoch":epoch,"ttl_s":ttl}),
            );
        }
        "device.lease.release" => {
            let mut s = shared.lock().map_err(|_| "SERVICE.LOCK")?;
            let resource = string(&p, "resource")?;
            verify_lease(&s, resource, &p)?;
            if s.jobs.values().any(|j| {
                matches!(j.state.as_str(), "queued" | "running" | "cancelling")
                    && j.params["resource"] == resource
            }) {
                return Err("DEVICE.CAPTURE_ACTIVE".into());
            }
            s.leases.remove(resource);
            return Ok(json!({"released":true}));
        }
        "wave.analyze" | "protocol.decode" | "stream.validate" | "uart.capture" => {}
        _ => return Err("CAPABILITY.UNSUPPORTED (not implemented or not verified)".into()),
    }
    let allowed: &[&str] = match method {
        "wave.analyze" => &[
            "command_id",
            "deadline_ms",
            "path",
            "sha256",
            "channel",
            "analysis",
            "low_channel",
            "options",
            "quality",
            "provenance",
        ],
        "protocol.decode" => &[
            "command_id",
            "deadline_ms",
            "path",
            "sha256",
            "kind",
            "channels",
            "threshold_v",
            "baud",
            "scp_ui_us",
            "byte_budget",
            "window",
            "quality",
        ],
        "stream.validate" => &["command_id", "deadline_ms", "path", "sha256"],
        "uart.capture" => &[
            "command_id",
            "deadline_ms",
            "resource",
            "lease_token",
            "session_epoch",
            "run_dir",
            "baud",
            "duration_ms",
        ],
        _ => unreachable!(),
    };
    if let Some(key) = p
        .as_object()
        .unwrap()
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))
    {
        return Err(format!("API.UNKNOWN_PARAM_{key}"));
    }
    for key in [
        "deadline_ms",
        "channel",
        "low_channel",
        "byte_budget",
        "duration_ms",
    ] {
        if p.get(key).is_some_and(|v| !v.is_u64()) {
            return Err(format!("API.INVALID_{key}"));
        }
    }
    if p.get("baud")
        .is_some_and(|v| !v.is_number() || v.as_f64().is_none_or(|n| !n.is_finite() || n <= 0.0))
    {
        return Err("API.INVALID_baud".into());
    }
    if (method == "uart.capture" || p["kind"] == "ufcs")
        && p.get("baud").is_some_and(|v| !v.is_u64())
    {
        return Err("API.INTEGER_BAUD_REQUIRED".into());
    }
    if p.get("analysis")
        .is_some_and(|v| !matches!(v.as_str(), Some("electrical" | "deadtime")))
    {
        return Err("ANALYSIS.UNSUPPORTED".into());
    }
    for key in ["threshold_v", "scp_ui_us"] {
        if p.get(key)
            .is_some_and(|v| v.as_f64().is_none_or(|n| !n.is_finite()))
        {
            return Err(format!("API.INVALID_{key}"));
        }
    }
    let command = string(&p, "command_id")?.to_string();
    if command.len() > 256 {
        return Err("API.COMMAND_ID_TOO_LONG".into());
    }
    let mut s = shared.lock().map_err(|_| "SERVICE.LOCK")?;
    if let Some(j) = s.jobs.values().find(|j| j.command_id == command) {
        if j.method != method || j.params != p {
            return Err("JOB.COMMAND_ID_CONFLICT".into());
        }
        return Ok(json!({"job_id":j.id,"state":j.state}));
    }
    if serde_json::to_vec(&p).map_err(|e| e.to_string())?.len() > 65536 {
        return Err("API.PARAMS_TOO_LARGE".into());
    }
    if s.jobs.len() >= 1024 {
        return Err("JOB.LEDGER_FULL_ARCHIVE_REQUIRED".into());
    }
    if s.cancel.len() >= 4 {
        return Err("JOB.BUSY".into());
    }
    if matches!(method, "wave.analyze" | "protocol.decode")
        && s.jobs.values().any(|j| {
            matches!(j.method.as_str(), "wave.analyze" | "protocol.decode")
                && matches!(j.state.as_str(), "queued" | "running")
        })
    {
        return Err("ANALYSIS.BUSY".into());
    }
    let deadline = p["deadline_ms"].as_u64().unwrap_or(120_000);
    if !(1..=3_600_000).contains(&deadline) {
        return Err("JOB.INVALID_DEADLINE".into());
    }
    if method == "uart.capture" {
        verify_lease(&s, string(&p, "resource")?, &p)?;
        if s.jobs.values().any(|j| {
            matches!(j.state.as_str(), "queued" | "running")
                && j.params["resource"] == p["resource"]
        }) {
            return Err("DEVICE.CAPTURE_ACTIVE".into());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    // The deadline starts at acceptance, not when a delayed worker is scheduled.
    let end = Instant::now() + Duration::from_millis(deadline);
    let c = Arc::new(AtomicBool::new(false));
    let root = s.root.clone();
    let j = Job {
        id: id.clone(),
        command_id: command,
        method: method.into(),
        params: p.clone(),
        state: "queued".into(),
        result: Value::Null,
        error: None,
        cancel_requested: false,
    };
    s.jobs.insert(id.clone(), j);
    if let Err(e) = s.persist() {
        s.jobs.remove(&id);
        return Err(e);
    }
    s.cancel.insert(id.clone(), c.clone());
    drop(s);
    let shared = shared.clone();
    let method = method.to_string();
    let result_id = id.clone();
    std::thread::spawn(move || {
        let valid_start = {
            let mut s = shared.lock().unwrap();
            let valid = method != "uart.capture"
                || verify_lease(&s, p["resource"].as_str().unwrap_or(""), &p).is_ok();
            s.jobs.get_mut(&result_id).unwrap().state = "running".into();
            let persisted = s.persist().is_ok();
            valid && persisted
        };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if valid_start {
                execute(&root, &result_id, &method, &p, &c, end, &shared)
            } else {
                Err("LEASE.EXPIRED_OR_STORAGE_FAILED".into())
            }
        }))
        .unwrap_or_else(|_| Err("JOB.WORKER_PANIC".into()));
        let mut s = shared.lock().unwrap();
        let j = s.jobs.get_mut(&result_id).unwrap();
        match r {
            Ok(value) => {
                j.state = if c.load(Ordering::SeqCst) {
                    "cancelled"
                } else {
                    "succeeded"
                }
                .into();
                j.result = value;
            }
            Err(e) => {
                j.state = if c.load(Ordering::SeqCst) {
                    "cancelled"
                } else {
                    "failed"
                }
                .into();
                j.error = Some(e);
            }
        }
        s.cancel.remove(&result_id);
        if let Err(e) = s.persist() {
            eprintln!("DATA.JOB_COMMIT_FAILED: {e}");
            s.jobs.get_mut(&result_id).unwrap().state = "unknown".into();
        }
    });
    Ok(json!({"accepted":true,"job_id":id}))
}
fn verify_lease(s: &State, resource: &str, p: &Value) -> Result<(), String> {
    let l = s.leases.get(resource).ok_or("LEASE.REQUIRED")?;
    if l.expires <= Instant::now() || p["lease_token"] != l.token || p["session_epoch"] != l.epoch {
        return Err("LEASE.EXPIRED_OR_FENCED".into());
    }
    Ok(())
}
fn checkpoint(c: &AtomicBool, end: Instant) -> Result<(), String> {
    if c.load(Ordering::SeqCst) {
        Err("JOB.CANCELLED".into())
    } else if Instant::now() >= end {
        Err("JOB.DEADLINE".into())
    } else {
        Ok(())
    }
}
fn execute(
    root: &std::path::Path,
    id: &str,
    method: &str,
    p: &Value,
    c: &AtomicBool,
    end: Instant,
    shared: &Arc<Mutex<State>>,
) -> Result<Value, String> {
    checkpoint(c, end)?;
    let out = root.join("jobs").join(format!("{id}.result.json"));
    let result = match method {
        "wave.analyze" | "protocol.decode" => {
            let path = safe_input(root, string(p, "path")?)?;
            let hash = file_hash(&path)?;
            if hash != string(p, "sha256")?.to_ascii_lowercase() {
                return Err("DATA.HASH_MISMATCH".into());
            }
            if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
                return Err("DATA.FILE_TOO_LARGE".into());
            }
            let waves = wiparse_core::waveform_file::load_waveform_file_all(&path)
                .map_err(|e| e.to_string())?;
            if waves.iter().map(|w| w.x.len()).sum::<usize>() > 4_000_000 {
                return Err("ANALYSIS.SAMPLE_BUDGET".into());
            }
            checkpoint(c, end)?;
            let channel = p["channel"].as_u64().unwrap_or(0) as usize;
            let trace = waves.get(channel).ok_or("ACQ.INVALID_CHANNEL")?;
            let o: AnalysisOptions =
                serde_json::from_value(p.get("options").cloned().unwrap_or(json!({})))
                    .map_err(|e| e.to_string())?;
            let q: Quality = serde_json::from_value(p.get("quality").cloned().unwrap_or(json!({})))
                .map_err(|e| e.to_string())?;
            if method == "protocol.decode" {
                let result = decode_protocol(root, id, p, &waves, &hash, c, end)?;
                if file_hash(&path)? != hash {
                    return Err("DATA.SOURCE_CHANGED".into());
                }
                return Ok(result);
            }
            if p["analysis"]
                .as_str()
                .is_some_and(|a| !matches!(a, "deadtime" | "electrical"))
            {
                return Err("ANALYSIS.UNSUPPORTED".into());
            }
            let mut m = if p["analysis"] == "deadtime" {
                let low = p["low_channel"]
                    .as_u64()
                    .ok_or("ACQ.LOW_CHANNEL_REQUIRED")? as usize;
                if low == channel {
                    return Err("ACQ.DUPLICATE_CHANNEL".into());
                }
                let provenance: TimingProvenance =
                    serde_json::from_value(p["provenance"].clone()).map_err(|e| e.to_string())?;
                deadtime(
                    trace,
                    waves.get(low).ok_or("ACQ.INVALID_CHANNEL")?,
                    &o,
                    &provenance,
                    &q,
                )?
            } else {
                analyze(trace, &o, &q)?
            };
            for item in &mut m {
                item.evidence_refs.push(path.to_string_lossy().into());
            }
            if file_hash(&path)? != hash {
                return Err("DATA.SOURCE_CHANGED".into());
            }
            json!({"schema_version":"wiparse.automation/1","source_sha256":hash,"measurements":m,"industrial_approved":false})
        }
        "stream.validate" => {
            let path = safe_input(root, string(p, "path")?)?;
            let hash = file_hash(&path)?;
            if hash != string(p, "sha256")?.to_ascii_lowercase() {
                return Err("DATA.HASH_MISMATCH".into());
            }
            let mut v = StreamValidator::default();
            let mut count = 0u64;
            let mut flagged = 0u64;
            let file = fs::File::open(&path).map_err(|e| e.to_string())?;
            let mut reader = std::io::BufReader::new(file);
            use std::io::BufRead;
            let normalized = root.join("jobs").join(format!("{id}.events.jsonl"));
            let mut output = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&normalized)
                .map_err(|e| e.to_string())?;
            loop {
                checkpoint(c, end)?;
                let mut line = Vec::new();
                let n = reader
                    .by_ref()
                    .take(1_048_577)
                    .read_until(b'\n', &mut line)
                    .map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                if n > 1_048_576 {
                    return Err("STREAM.RECORD_TOO_LARGE".into());
                }
                let mut e: EventRecord =
                    serde_json::from_slice(&line).map_err(|e| e.to_string())?;
                v.ingest(&mut e)?;
                count += 1;
                if !e.quality.valid() {
                    flagged += 1;
                }
                writeln!(
                    output,
                    "{}",
                    serde_json::to_string(&e).map_err(|e| e.to_string())?
                )
                .map_err(|e| e.to_string())?;
            }
            output.sync_all().map_err(|e| e.to_string())?;
            if file_hash(&path)? != hash {
                return Err("DATA.SOURCE_CHANGED".into());
            }
            if count == 0 {
                return Err("STREAM.EMPTY".into());
            }
            json!({"schema_version":"wiparse.automation/1","source_sha256":hash,"events_path":normalized,"events_sha256":file_hash(&normalized)?,"observed_records":count,"quality_flagged_records":flagged,"complete":flagged==0,"industrial_approved":false})
        }
        "uart.capture" => {
            let resource = string(p, "resource")?;
            let run = string(p, "run_dir")?;
            let projects = fs::canonicalize(root.join("projects")).map_err(|e| e.to_string())?;
            let run = fs::canonicalize(root.join(run)).map_err(|e| e.to_string())?;
            if !run.starts_with(&projects) || !run.is_dir() {
                return Err("DATA.PATH_OUTSIDE_PROJECT".into());
            }
            let baud = p["baud"].as_u64().unwrap_or(115200);
            if !(1200..=2_000_000).contains(&baud) {
                return Err("UART.INVALID_BAUD".into());
            }
            let duration = p["duration_ms"].as_u64().ok_or("UART.DURATION_REQUIRED")?;
            if duration == 0 || duration > 3_600_000 {
                return Err("UART.INVALID_DURATION".into());
            }
            let mut port = serialport::new(resource, baud as u32)
                .timeout(Duration::from_millis(50))
                .open()
                .map_err(|e| e.to_string())?;
            let file = run.join(format!("uart_{id}.bin"));
            let mut f = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&file)
                .map_err(|e| e.to_string())?;
            let start = Instant::now();
            let mut bytes = 0u64;
            let mut buffer = [0u8; 8192];
            let capture_status = (|| -> Result<(), String> {
                while start.elapsed() < Duration::from_millis(duration) {
                    checkpoint(c, end)?;
                    {
                        let s = shared.lock().map_err(|_| "SERVICE.LOCK")?;
                        verify_lease(&s, resource, p)?;
                    }
                    match port.read(&mut buffer) {
                        Ok(n) => {
                            bytes += n as u64;
                            if bytes > 512 * 1024 * 1024 {
                                return Err("UART.CAPTURE_BUDGET".into());
                            }
                            f.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                        Err(e) => return Err(e.to_string()),
                    }
                }
                checkpoint(c, end)?;
                Ok(())
            })();
            f.sync_all().map_err(|e| e.to_string())?;
            let record = json!({"schema_version":"wiparse.automation/1","capture_id":id,"path":file,"bytes":fs::metadata(&file).map_err(|e|e.to_string())?.len(),"sha256":file_hash(&file)?,"complete":capture_status.is_ok(),"error":capture_status.as_ref().err(),"quality":{"issues":["UART.HARDWARE_TIMESTAMPS_UNAVAILABLE","UART.LINE_ERROR_COUNTERS_UNAVAILABLE"]},"industrial_approved":false});
            atomic_json(
                &root.join("jobs").join(format!("{id}.capture.json")),
                &record,
            )?;
            capture_status?;
            record
        }
        _ => return Err("CAPABILITY.UNSUPPORTED".into()),
    };
    checkpoint(c, end)?;
    atomic_json(&out, &result)?;
    Ok(json!({"path":out,"sha256":file_hash(&out)?}))
}

fn decode_protocol(
    root: &std::path::Path,
    id: &str,
    p: &Value,
    waves: &[wiparse_core::instrument::WaveformTrace],
    hash: &str,
    c: &AtomicBool,
    end: Instant,
) -> Result<Value, String> {
    use wiparse_core::bus_decode::{decode_bus_offline, BusDecodeSettings, BusKind};
    let mut s = BusDecodeSettings::default();
    s.kind = match string(p, "kind")? {
        "uart" => BusKind::Uart,
        "i2c" => BusKind::I2c,
        "spi" => BusKind::Spi,
        "i2s" => BusKind::I2s,
        "ddsss" => BusKind::Ddsss,
        "usb_pd" => BusKind::UsbPd,
        "scp" => BusKind::Scp,
        "ufcs" => BusKind::Ufcs,
        _ => return Err("PROTOCOL.UNSUPPORTED".into()),
    };
    let channels = p["channels"]
        .as_object()
        .ok_or("PROTOCOL.CHANNELS_REQUIRED")?;
    let mut normalized = waves.to_vec();
    for (role, index) in channels {
        if !matches!(
            role.as_str(),
            "signal" | "scl" | "sda" | "clk" | "mosi" | "miso" | "cs" | "ws" | "data"
        ) {
            return Err("PROTOCOL.UNKNOWN_CHANNEL_ROLE".into());
        }
        let index = index
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .ok_or("PROTOCOL.INVALID_CHANNEL")?;
        let trace = normalized
            .get_mut(index)
            .ok_or("PROTOCOL.INVALID_CHANNEL")?;
        if wiparse_core::automation::analysis::trace_stats(trace).is_none() {
            return Err("ACQ.INVALID_SAMPLES".into());
        }
        let scale = match trace.x_unit.as_str() {
            "s" => 1.0,
            "ms" => 1e-3,
            "us" | "µs" => 1e-6,
            "ns" => 1e-9,
            _ => return Err("ACQ.TIME_UNIT".into()),
        };
        if !matches!(trace.y_unit.as_str(), "V" | "mV" | "uV" | "µV") {
            return Err("ACQ.VOLTAGE_UNIT".into());
        }
        if scale != 1.0 {
            trace.x = trace.x.iter().map(|x| x * scale).collect::<Vec<_>>().into();
            trace.x_unit = "s".into();
        }
    }
    let idx = |name: &str| p["channels"][name].as_u64().map(|n| n as usize);
    s.channels.uart_signal = idx("signal");
    s.channels.charging_signal = idx("signal");
    s.channels.ddsss_signal = idx("signal");
    s.channels.i2c_scl = idx("scl");
    s.channels.i2c_sda = idx("sda");
    s.channels.spi_clk = idx("clk");
    s.channels.spi_mosi = idx("mosi");
    s.channels.spi_miso = idx("miso");
    s.channels.spi_cs = idx("cs");
    s.channels.i2s_bclk = idx("clk");
    s.channels.i2s_ws = idx("ws");
    s.channels.i2s_data = idx("data");
    s.threshold = p["threshold_v"].as_f64();
    if let Some(v) = s.threshold {
        if !v.is_finite() {
            return Err("PROTOCOL.INVALID_THRESHOLD".into());
        }
    }
    if let Some(window) = p.get("window") {
        let window: [f64; 2] = serde_json::from_value(window.clone()).map_err(|e| e.to_string())?;
        if !window[0].is_finite() || !window[1].is_finite() || window[0] >= window[1] {
            return Err("ACQ.INVALID_WINDOW".into());
        }
        s.time_gate = Some((window[0], window[1]));
    }
    s.uart.baud = p["baud"].as_f64();
    if let Some(baud) = p["baud"].as_u64() {
        s.ufcs.baud = u32::try_from(baud).map_err(|_| "PROTOCOL.INVALID_BAUD")?;
    }
    if let Some(v) = p["scp_ui_us"].as_f64() {
        s.scp.ui_us = v;
    }
    let bytes = p["byte_budget"].as_u64().unwrap_or(65536) as usize;
    let d = decode_bus_offline(&normalized, &s, bytes, 262144)?;
    checkpoint(c, end)?;
    let frame_path = root.join("jobs").join(format!("{id}.frames.jsonl"));
    let mut frames = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&frame_path)
        .map_err(|e| e.to_string())?;
    let mut bad = 0u64;
    for f in &d.frames {
        checkpoint(c, end)?;
        if f.has_error() {
            bad += 1;
        }
        writeln!(frames,"{}",json!({"start_s":f.t_start,"end_s":f.t_end,"summary":f.summary,"bytes":f.bytes,"integrity":format!("{:?}",f.error)})).map_err(|e|e.to_string())?;
    }
    frames.sync_all().map_err(|e| e.to_string())?;
    let mut quality: Quality =
        serde_json::from_value(p.get("quality").cloned().unwrap_or(json!({})))
            .map_err(|e| e.to_string())?;
    if d.truncated {
        quality.flag("PROTOCOL.TRUNCATED");
    }
    if d.frames.is_empty() {
        quality.flag("PROTOCOL.NO_FRAMES");
    }
    if d.error.is_some() {
        quality.flag("PROTOCOL.DECODER_ERROR");
    }
    let result = json!({"schema_version":"wiparse.automation/1","quality":quality,"source_sha256":hash,"observed_frames":d.frames.len(),"integrity_error_frames":bad,"truncated":d.truncated,"error":d.error,
        "complete":!d.truncated&&d.error.is_none()&&!d.frames.is_empty()&&quality.valid(),"denominator":"observed_decoder_frames_not_true_PER","info":d.info,
        "frames_path":frame_path,"frames_sha256":file_hash(&frame_path)?,"industrial_approved":false});
    let out = root.join("jobs").join(format!("{id}.result.json"));
    atomic_json(&out, &result)?;
    Ok(json!({"path":out,"sha256":file_hash(&out)?}))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("wiparse-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(p.join("projects")).unwrap();
            Self(p)
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn api(s: &Arc<Mutex<State>>, m: &str, p: Value) -> Result<Value, String> {
        invoke(s, &json!({"method":m,"params":p}))
    }
    #[test]
    fn restart_never_replays_ambiguous_jobs() {
        let dir = Sandbox::new();
        let mut s = State::open(dir.0.clone()).unwrap();
        for state in ["queued", "running", "cancelling"] {
            s.jobs.insert(
                state.into(),
                Job {
                    id: state.into(),
                    command_id: state.into(),
                    method: "uart.capture".into(),
                    params: json!({}),
                    state: state.into(),
                    result: Value::Null,
                    error: None,
                    cancel_requested: false,
                },
            );
        }
        s.persist().unwrap();
        drop(s);
        let recovered = State::open(dir.0.clone()).unwrap();
        assert!(recovered
            .jobs
            .values()
            .all(|j| j.state == "unknown" && j.error.as_deref() == Some("SERVICE.INTERRUPTED")));
    }
    #[test]
    fn fencing_conflict_and_expiry() {
        let dir = Sandbox::new();
        let s = Arc::new(Mutex::new(State::open(dir.0.clone()).unwrap()));
        let lease = api(&s, "device.lease.acquire", json!({"resource":"COM_TEST"})).unwrap();
        assert!(api(&s, "device.lease.acquire", json!({"resource":"COM_TEST"})).is_err());
        let bad = json!({"resource":"COM_TEST","lease_token":"old","session_epoch":lease["session_epoch"]});
        assert!(api(&s, "device.lease.release", bad).is_err());
        api(&s, "device.lease.release", lease).unwrap();
        let fresh = api(&s, "device.lease.acquire", json!({"resource":"COM_TEST"})).unwrap();
        s.lock()
            .unwrap()
            .leases
            .get_mut("COM_TEST")
            .unwrap()
            .expires = Instant::now();
        assert!(api(&s, "device.lease.release", fresh).is_err());
    }
    #[test]
    fn replay_hash_and_idempotent_job() {
        let dir = Sandbox::new();
        let path = dir.0.join("projects/events.jsonl");
        let event = |seq| {
            json!({"source":"sim","boot_id":"boot","capture_session_id":"session","stream_id":"io","seq":seq,"device_tick":"9007199254740993","tick_hz":1000000,"event_type":"edge","payload":{},"quality":{"issues":[]}}).to_string()
        };
        fs::write(&path, format!("{}\n{}\n", event("1"), event("3"))).unwrap();
        let hash = file_hash(&path).unwrap();
        let s = Arc::new(Mutex::new(State::open(dir.0.clone()).unwrap()));
        let p = json!({"command_id":"once","path":"projects/events.jsonl","sha256":hash});
        let accepted = api(&s, "stream.validate", p.clone()).unwrap();
        let id = accepted["job_id"].as_str().unwrap();
        assert_eq!(api(&s, "stream.validate", p.clone()).unwrap()["job_id"], id);
        let mut changed = p.clone();
        changed["sha256"] = json!("bad");
        assert!(api(&s, "stream.validate", changed).is_err());
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            let j = api(&s, "job.get", json!({"job_id":id})).unwrap();
            if j["state"] == "succeeded" {
                let result: Value = serde_json::from_slice(
                    &fs::read(j["result"]["path"].as_str().unwrap()).unwrap(),
                )
                .unwrap();
                assert_eq!(result["complete"], false);
                assert_eq!(result["quality_flagged_records"], 1);
                break;
            }
            assert!(!matches!(j["state"].as_str(), Some("failed" | "unknown")));
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(safe_input(&dir.0, "jobs.json").is_err());
    }
    #[test]
    fn storage_failure_latches_new_work_but_keeps_queries_available() {
        let dir = Sandbox::new();
        let s = Arc::new(Mutex::new(State::open(dir.0.clone()).unwrap()));
        fs::remove_file(dir.0.join("jobs.json")).unwrap();
        fs::create_dir(dir.0.join("jobs.json")).unwrap();
        assert!(s.lock().unwrap().persist().is_err());
        assert!(api(&s, "device.lease.acquire", json!({"resource":"MOCK"}))
            .unwrap_err()
            .contains("STORAGE_FAULT_LATCHED"));
        assert!(api(&s, "system.capabilities", json!({})).is_ok());
        assert_eq!(s.lock().unwrap().health()["accepting_jobs"], false);
    }
    #[test]
    fn unsupported_settings_and_mistyped_channels_are_not_silently_defaulted() {
        let dir = Sandbox::new();
        let s = Arc::new(Mutex::new(State::open(dir.0.clone()).unwrap()));
        assert!(
            api(&s, "protocol.decode", json!({"command_id":"bad","mode":3}))
                .unwrap_err()
                .contains("UNKNOWN_PARAM_mode")
        );
        assert!(api(
            &s,
            "wave.analyze",
            json!({"command_id":"bad","channel":"CH2"})
        )
        .unwrap_err()
        .contains("INVALID_channel"));
        assert!(api(
            &s,
            "protocol.decode",
            json!({"command_id":"bad","kind":"ufcs","baud":9600.5})
        )
        .unwrap_err()
        .contains("INTEGER_BAUD_REQUIRED"));
        assert!(s.lock().unwrap().jobs.is_empty());
    }
    #[test]
    fn cancellation_and_deadline_gate() {
        let c = AtomicBool::new(true);
        assert_eq!(
            checkpoint(&c, Instant::now() + Duration::from_secs(1)).unwrap_err(),
            "JOB.CANCELLED"
        );
        c.store(false, Ordering::SeqCst);
        assert_eq!(checkpoint(&c, Instant::now()).unwrap_err(), "JOB.DEADLINE");
    }
}
